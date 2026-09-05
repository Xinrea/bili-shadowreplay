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

#[derive(Clone, Serialize, Debug)]
pub struct DanmuEntry {
    pub ts: i64,
    pub content: String,
}

pub struct DanmuStorage {
    cache: RwLock<Vec<LiveEvent>>,
    file: RwLock<File>,
}

impl DanmuStorage {
    pub async fn new(file_path: &PathBuf) -> Option<DanmuStorage> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(file_path)
            .await;
        if file.is_err() {
            log::error!("Open danmu file failed: {}", file.err().unwrap());
            return None;
        }
        let file = file.unwrap();
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

    // get entries with ts relative to live start time
    pub async fn get_entries(&self, live_start_ts: i64) -> Vec<DanmuEntry> {
        let mut danmus: Vec<DanmuEntry> = self
            .cache
            .read()
            .await
            .iter()
            .filter(|event| event.event_type == "danmu")
            .filter_map(|event| {
                event.data.get("content").and_then(|content| {
                    content.as_str().map(|content| DanmuEntry {
                        ts: event.ts - live_start_ts,
                        content: content.to_string(),
                    })
                })
            })
            .collect();
        // filter out danmus with ts < 0
        danmus.retain(|entry| entry.ts >= 0);
        danmus
    }
}
