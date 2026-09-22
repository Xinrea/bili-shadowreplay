use std::path::PathBuf;

use danmu_stream::LiveEvent;
use serde::Serialize;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncBufReadExt, BufReader},
    sync::RwLock,
};

/// Live event types surfaced by the preview (danmu list + floating overlay).
/// New displayable event kinds are added here and picked up end to end.
pub const PREVIEW_EVENT_TYPES: &[&str] = &["danmu", "super_chat"];

#[derive(Clone, Serialize, Debug)]
pub struct DanmuEntry {
    pub ts: i64,
    /// Event type, aligned with the recorded `LiveEvent` "type" field
    /// (e.g. "danmu", "super_chat"). Open set: more event types may follow.
    #[serde(rename = "type")]
    pub event_type: String,
    pub content: String,
    pub user_name: Option<String>,
    /// Super chat price in CNY (battery). `None` for other event types.
    pub price: Option<u32>,
    /// Super chat pinned duration in seconds.
    pub sc_duration: Option<u32>,
}

impl DanmuEntry {
    pub fn is_danmu(&self) -> bool {
        self.event_type == "danmu"
    }
}

pub struct DanmuStorage {
    cache: RwLock<Vec<LiveEvent>>,
    file: RwLock<File>,
}

impl DanmuStorage {
    pub async fn new(file_path: &PathBuf) -> Option<DanmuStorage> {
        let file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(file_path)
            .await
        {
            Ok(file) => file,
            Err(e) => {
                log::error!("Open danmu file failed: {e}");
                return None;
            }
        };
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut preload_cache: Vec<LiveEvent> = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(event) = serde_json::from_str::<LiveEvent>(&line) {
                preload_cache.push(event);
            } else {
                // Read old recordings as a migration aid. New writes are
                // always JSONL and never append to the legacy format.
                let Some((ts, content)) = line.split_once(':') else {
                    continue;
                };
                let Ok(ts) = ts.parse() else { continue };
                preload_cache.push(LiveEvent {
                    ts,
                    platform: "unknown".to_string(),
                    room_id: String::new(),
                    event_type: "danmu".to_string(),
                    data: serde_json::json!({ "content": content }),
                    raw: Value::Null,
                });
            }
        }
        // lines.next_line() consumes the reader, so the file is closed when lines is dropped
        drop(lines);

        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(file_path)
            .await
            .map_err(|e| {
                log::error!("Failed to open danmu file for append: {}", e);
                e
            })
            .ok()?;
        Some(DanmuStorage {
            cache: RwLock::new(preload_cache),
            file: RwLock::new(file),
        })
    }

    pub async fn add_event(&self, event: LiveEvent) -> Result<(), std::io::Error> {
        let Ok(mut line) = serde_json::to_string(&event) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "serialize live event failed",
            ));
        };
        line.push('\n');
        self.file.write().await.write_all(line.as_bytes()).await?;
        self.cache.write().await.push(event);
        Ok(())
    }

    pub async fn add_line(&self, ts: i64, content: &str) -> Result<(), std::io::Error> {
        self.add_event(LiveEvent {
            ts,
            platform: "unknown".to_string(),
            room_id: String::new(),
            event_type: "danmu".to_string(),
            data: serde_json::json!({ "content": content }),
            raw: Value::Null,
        })
        .await
    }

    /// Get entries with ts relative to live start time.
    ///
    /// All preview-relevant event types are returned; consumers that only want
    /// danmaku (e.g. ASS export) filter on `event_type`.
    pub async fn get_entries(&self, live_start_ts: i64) -> Vec<DanmuEntry> {
        let mut danmus: Vec<DanmuEntry> = self
            .cache
            .read()
            .await
            .iter()
            .filter(|event| PREVIEW_EVENT_TYPES.contains(&event.event_type.as_str()))
            .filter_map(|event| {
                let is_super_chat = event.event_type == "super_chat";
                event.data.get("content").and_then(|content| {
                    content.as_str().map(|content| {
                        let price = if is_super_chat {
                            event
                                .data
                                .get("price")
                                .and_then(|price| price.as_u64())
                                .map(|price| price as u32)
                        } else {
                            None
                        };
                        let sc_duration = if is_super_chat {
                            event
                                .data
                                .get("duration")
                                .and_then(|duration| duration.as_u64())
                                .map(|duration| duration as u32)
                        } else {
                            None
                        };
                        DanmuEntry {
                            ts: event.ts - live_start_ts,
                            event_type: event.event_type.clone(),
                            content: content.to_string(),
                            user_name: event
                                .data
                                .get("user_name")
                                .and_then(|name| name.as_str())
                                .map(str::to_string),
                            price,
                            sc_duration,
                        }
                    })
                })
            })
            .collect();
        // filter out danmus with ts < 0
        danmus.retain(|entry| entry.ts >= 0);
        danmus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_lines(path: &PathBuf, lines: &[String]) {
        use std::io::Write;
        let mut file = std::fs::File::create(path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
    }

    #[tokio::test]
    async fn get_entries_keeps_super_chats_and_danmaku() {
        let path = std::env::temp_dir().join(format!(
            "danmu-storage-test-{}-{}.jsonl",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        write_lines(
            &path,
            &[
                json!({
                    "ts": 1_700_000_000_000i64,
                    "platform": "bilibili",
                    "room_id": "1",
                    "type": "danmu",
                    "data": { "content": "hello", "user_name": "alice" },
                    "raw": null,
                })
                .to_string(),
                json!({
                    "ts": 1_700_000_005_000i64,
                    "platform": "bilibili",
                    "room_id": "1",
                    "type": "super_chat",
                    "data": {
                        "content": "hi sc",
                        "user_name": "bob",
                        "price": 30,
                        "duration": 60,
                    },
                    "raw": null,
                })
                .to_string(),
                json!({
                    "ts": 1_700_000_006_000i64,
                    "platform": "bilibili",
                    "room_id": "1",
                    "type": "gift",
                    "data": { "content": "not a danmu", "user_name": "carol" },
                    "raw": null,
                })
                .to_string(),
            ],
        );

        let storage = DanmuStorage::new(&path).await.unwrap();
        let entries = storage.get_entries(1_700_000_000_000).await;

        std::fs::remove_file(&path).ok();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].event_type, "danmu");
        assert_eq!(entries[0].content, "hello");
        assert_eq!(entries[0].user_name.as_deref(), Some("alice"));
        assert_eq!(entries[0].price, None);
        assert_eq!(entries[0].sc_duration, None);
        assert_eq!(entries[1].event_type, "super_chat");
        assert_eq!(entries[1].content, "hi sc");
        assert_eq!(entries[1].user_name.as_deref(), Some("bob"));
        assert_eq!(entries[1].price, Some(30));
        assert_eq!(entries[1].sc_duration, Some(60));
    }
}
