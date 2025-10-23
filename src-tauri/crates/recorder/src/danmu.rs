use std::path::PathBuf;

use serde::Serialize;
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
    cache: RwLock<Vec<DanmuEntry>>,
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
        let mut preload_cache: Vec<DanmuEntry> = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            let parts: Vec<&str> = line.split(':').collect();
            let ts: i64 = parts[0].parse().unwrap();
            let content = parts[1].to_string();
            preload_cache.push(DanmuEntry { ts, content });
        }
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(file_path)
            .await
            .expect("create danmu.txt failed");
        Some(DanmuStorage {
            cache: RwLock::new(preload_cache),
            file: RwLock::new(file),
        })
    }

    pub async fn add_line(&self, ts: i64, content: &str) {
        self.cache.write().await.push(DanmuEntry {
            ts,
            content: content.to_string(),
        });
        let _ = self
            .file
            .write()
            .await
            .write(format!("{ts}:{content}\n").as_bytes())
            .await;
    }

    // get entries with ts relative to live start time
    pub async fn get_entries(&self, live_start_ts: i64) -> Vec<DanmuEntry> {
        let mut danmus: Vec<DanmuEntry> = self
            .cache
            .read()
            .await
            .iter()
            .map(|entry| DanmuEntry {
                ts: entry.ts - live_start_ts,
                content: entry.content.clone(),
            })
            .collect();
        // filter out danmus with ts < 0
        danmus.retain(|entry| entry.ts >= 0);
        danmus
    }
}
