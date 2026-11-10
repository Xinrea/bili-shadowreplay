use std::{io, path::Path};

use danmu_stream::LiveEvent;
use recording_storage::{AppendLog, RecordingStorage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::RwLock,
};

#[cfg(test)]
pub(crate) mod writer_tests;

mod batch;
pub use batch::DanmuWriter;

/// Live event types surfaced by the preview (danmu list + floating overlay).
/// New displayable event kinds are added here and picked up end to end.
pub const PREVIEW_EVENT_TYPES: &[&str] = &["danmu", "super_chat"];

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct DanmuEntry {
    pub ts: i64,
    /// Event type, aligned with the recorded `LiveEvent` "type" field
    /// (e.g. "danmu", "super_chat"). Open set: more event types may follow.
    #[serde(rename = "type")]
    pub event_type: String,
    pub content: String,
    pub emoji_content: Option<danmu_stream::EmojiContent>,
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

fn event_line(event: &LiveEvent) -> io::Result<Vec<u8>> {
    let mut line = serde_json::to_vec(event)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "serialize live event failed"))?;
    line.push(b'\n');
    Ok(line)
}

/// Explicit history-backed compatibility API; not used to start live recording.
pub struct DanmuStorage {
    cache: RwLock<Vec<LiveEvent>>,
    writer: AppendLog,
}

impl DanmuStorage {
    /// Open a local event sidecar. Asset names must be UTF-8; parent directory
    /// paths may contain non-UTF-8 components.
    pub async fn new(file_path: &Path) -> Option<DanmuStorage> {
        let Some(key) = file_path.file_name().and_then(|name| name.to_str()) else {
            log::error!("Open danmu file failed: invalid file name");
            return None;
        };
        let parent = file_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let storage = RecordingStorage::local(parent);
        Self::from_storage(&storage, key)
            .await
            .map_err(|e| log::error!("Open danmu file failed: {e}"))
            .ok()
    }

    /// Resume a JSONL event log, also accepting legacy `timestamp:content` lines.
    pub async fn from_storage(storage: &RecordingStorage, key: &str) -> io::Result<Self> {
        // Read before opening for append: only a missing log starts empty.
        let contents = match storage.read_log(key).await {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };
        let preload_cache = Self::parse_events(&contents).await;
        // Keep the explicit compatibility API durable-on-return. Only the live
        // recording writer batches, so offline callers need no new lifecycle.
        let writer = storage.open_log(key).await?;
        Ok(DanmuStorage {
            cache: RwLock::new(preload_cache),
            writer,
        })
    }

    /// Read preview entries with absolute timestamps, without opening/creating a
    /// writable log. Missing logs and backend failures are returned to the caller.
    pub async fn read_entries(
        storage: &RecordingStorage,
        key: &str,
    ) -> io::Result<Vec<DanmuEntry>> {
        let started = std::time::Instant::now();
        log::debug!(
            "Reading event history: storage={:?} key={key}",
            storage.identity()
        );
        let contents = match storage.read_log(key).await {
            Ok(contents) => contents,
            Err(error) => {
                log::warn!("Event history read failed: storage={:?} key={key} duration_ms={} error_kind={:?}",
                    storage.identity(), started.elapsed().as_millis(), error.kind());
                return Err(error);
            }
        };
        let events = Self::parse_events(&contents).await;
        let entries = Self::entries(&events, 0);
        log::debug!("Event history loaded: storage={:?} key={key} bytes={} events={} preview_entries={} duration_ms={}",
            storage.identity(), contents.len(), events.len(), entries.len(), started.elapsed().as_millis());
        Ok(entries)
    }

    async fn parse_events(contents: &[u8]) -> Vec<LiveEvent> {
        let reader = BufReader::new(contents);
        let mut lines = reader.lines();
        let mut preload_cache: Vec<LiveEvent> = Vec::new();
        // Preserve the readable prefix of an interrupted legacy log, including
        // one whose final UTF-8 character was only partially written.
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

        preload_cache
    }

    pub async fn add_event(&self, event: LiveEvent) -> Result<(), std::io::Error> {
        self.writer.append(&event_line(&event)?).await?;
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
        Self::entries(&self.cache.read().await, live_start_ts)
    }

    fn entries(events: &[LiveEvent], live_start_ts: i64) -> Vec<DanmuEntry> {
        let mut danmus: Vec<DanmuEntry> = events
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
                            emoji_content: event.emoji_content(),
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
    use std::path::PathBuf;

    #[tokio::test]
    async fn read_entries_never_creates_or_opens_a_writable_log() {
        let dir = tempfile::tempdir().unwrap();
        let storage = RecordingStorage::local(dir.path());
        assert_eq!(
            DanmuStorage::read_entries(&storage, "events.jsonl")
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound,
        );
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
        storage.write("danmu.txt", b"100:legacy\n").await.unwrap();
        let entries = DanmuStorage::read_entries(&storage, "danmu.txt")
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].ts, 100);
        assert_eq!(entries[0].content, "legacy");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert_eq!(storage.read("danmu.txt").await.unwrap(), b"100:legacy\n");
    }

    #[tokio::test]
    async fn storage_resumes_jsonl_and_appends_complete_lines() {
        let dir = tempfile::tempdir().unwrap();
        let storage = RecordingStorage::local(dir.path());
        let danmu = DanmuStorage::from_storage(&storage, "events.jsonl")
            .await
            .unwrap();
        assert!(danmu.get_entries(0).await.is_empty());
        danmu.add_line(100, "hello\nwith a newline").await.unwrap();
        drop(danmu);

        let danmu = DanmuStorage::from_storage(&storage, "events.jsonl")
            .await
            .unwrap();
        danmu.add_line(200, "resumed").await.unwrap();
        assert_eq!(danmu.get_entries(0).await.len(), 2);
        drop(danmu);

        let bytes = storage.read("events.jsonl").await.unwrap();
        assert_eq!(bytes.iter().filter(|&&byte| byte == b'\n').count(), 2);
        assert_eq!(bytes.last(), Some(&b'\n'));
        let events: Vec<LiveEvent> = std::str::from_utf8(&bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(events[0].data["content"], "hello\nwith a newline");
        assert_eq!(events[1].ts, 200);
        let entries = DanmuStorage::new(&dir.path().join("events.jsonl"))
            .await
            .unwrap()
            .get_entries(0)
            .await;
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn legacy_danmu_wrapper_accepts_crlf_colons_and_skips_invalid_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("danmu.txt");
        let storage = RecordingStorage::local(dir.path());
        storage
            .write(
                "danmu.txt",
                b"100:first:message\r\ninvalid\nbad:time\n200:last\n",
            )
            .await
            .unwrap();
        let danmu = DanmuStorage::new(&path).await.unwrap();
        let entries = danmu.get_entries(0).await;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].content, "first:message");
        assert_eq!(entries[0].ts, 100);
        assert_eq!(entries[1].content, "last");
        // New records remain JSONL even when reading a legacy archive.
        danmu.add_line(300, "new event").await.unwrap();
        drop(danmu);
        let reloaded = DanmuStorage::from_storage(&storage, "danmu.txt")
            .await
            .unwrap();
        assert_eq!(reloaded.get_entries(0).await.len(), 3);
    }

    #[tokio::test]
    async fn storage_reports_read_and_key_errors_without_replacing_contents() {
        let dir = tempfile::tempdir().unwrap();
        let storage = RecordingStorage::local(dir.path());
        std::fs::create_dir(dir.path().join("events.jsonl")).unwrap();
        assert!(DanmuStorage::from_storage(&storage, "events.jsonl")
            .await
            .is_err());
        assert!(DanmuStorage::new(&dir.path().join("events.jsonl"))
            .await
            .is_none());
        assert!(DanmuStorage::from_storage(&storage, "../outside.jsonl")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn interrupted_utf8_tail_preserves_preloaded_events() {
        let dir = tempfile::tempdir().unwrap();
        let storage = RecordingStorage::local(dir.path());
        let bytes = b"100:complete\n200:interrupted \xe4";
        storage.write("events.jsonl", bytes).await.unwrap();
        let danmu = DanmuStorage::from_storage(&storage, "events.jsonl")
            .await
            .unwrap();
        let entries = danmu.get_entries(0).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content, "complete");
        let wrapped = DanmuStorage::new(&dir.path().join("events.jsonl"))
            .await
            .unwrap();
        assert_eq!(wrapped.get_entries(0).await.len(), 1);
        assert_eq!(storage.read("events.jsonl").await.unwrap(), bytes);
    }

    #[tokio::test]
    async fn missing_parent_is_an_open_error_not_an_empty_storage() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("missing-parent");
        let storage = RecordingStorage::local(&parent);
        assert!(DanmuStorage::from_storage(&storage, "events.jsonl")
            .await
            .is_err());
        assert!(DanmuStorage::new(&parent.join("events.jsonl"))
            .await
            .is_none());
        assert!(!parent.exists());
    }

    // Linux permits arbitrary non-NUL path bytes; macOS filesystems may reject
    // their creation with EILSEQ before the compatibility wrapper is exercised.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn path_wrapper_supports_non_utf8_parent_directories() {
        use std::os::unix::ffi::OsStringExt;

        let dir = tempfile::tempdir().unwrap();
        let parent = dir
            .path()
            .join(std::ffi::OsString::from_vec(b"recording-\xff".to_vec()));
        std::fs::create_dir(&parent).unwrap();
        let path = parent.join("events.jsonl");
        let danmu = DanmuStorage::new(&path).await.unwrap();
        danmu.add_line(100, "hello").await.unwrap();
        drop(danmu);
        assert_eq!(
            DanmuStorage::new(&path)
                .await
                .unwrap()
                .get_entries(0)
                .await
                .len(),
            1
        );
    }

    fn write_lines(path: &PathBuf, lines: &[String]) {
        use std::io::Write;
        let mut file = std::fs::File::create(path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
    }

    #[tokio::test]
    async fn image_metadata_survives_jsonl_and_matches_live_preview() {
        let path = std::env::temp_dir().join(format!(
            "emoji-storage-{}-{}.jsonl",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let emoji = json!({"url": "https://example.com/emoji.webp", "width": 80, "height": 60, "emoticon_unique": "test"});
        let raw = json!({"info": [[0,0,0,0,0,0,0,0,0,0,0,0,1,emoji]]});
        let storage = DanmuStorage::new(&path).await.unwrap();
        for normalized in [true, false] {
            let event = LiveEvent {
                ts: 100,
                platform: "bilibili".into(),
                room_id: "1".into(),
                event_type: "danmu".into(),
                data: if normalized {
                    json!({"content": "[emoji]", "emoji_content": emoji})
                } else {
                    json!({"content": "[emoji]"})
                },
                raw: raw.clone(),
            };
            let preview =
                crate::events::RecorderEvent::danmu_received_from_event("1".into(), &event)
                    .unwrap();
            let crate::events::RecorderEvent::DanmuReceived {
                emoji_content,
                content,
                ..
            } = preview
            else {
                panic!("expected preview");
            };
            assert_eq!(content, "[emoji]");
            assert_eq!(serde_json::to_value(emoji_content).unwrap(), emoji);
            storage.add_event(event).await.unwrap();
        }
        drop(storage);
        let storage = DanmuStorage::new(&path).await.unwrap();
        let entries = storage.get_entries(0).await;
        assert_eq!(entries.len(), 2);
        for entry in entries {
            let value = serde_json::to_value(entry).unwrap();
            assert_eq!(value["content"], "[emoji]");
            assert_eq!(value["emoji_content"], emoji);
        }
        drop(storage);
        std::fs::remove_file(path).unwrap();
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
