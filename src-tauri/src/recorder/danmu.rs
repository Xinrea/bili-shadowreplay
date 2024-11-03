use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncBufReadExt, BufReader},
    sync::RwLock,
};

#[derive(Clone, Serialize)]
pub struct DanmuEntry {
    pub ts: u64,
    pub content: String,
}

pub struct DanmuStorage {
    cache: RwLock<Vec<DanmuEntry>>,
    file: RwLock<File>,
}

impl DanmuStorage {
    pub async fn new(file_path: &str) -> DanmuStorage {
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(file_path)
            .await
            .expect("create danmu.txt failed");
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut preload_cache: Vec<DanmuEntry> = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            let parts: Vec<&str> = line.split(':').collect();
            let ts: u64 = parts[0].parse().unwrap();
            let content = parts[1].to_string();
            preload_cache.push(DanmuEntry { ts, content })
        }
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(file_path)
            .await
            .expect("create danmu.txt failed");
        return DanmuStorage {
            cache: RwLock::new(preload_cache),
            file: RwLock::new(file),
        };
    }

    pub async fn add_line(&self, ts: u64, content: &str) {
        self.cache.write().await.push(DanmuEntry {
            ts,
            content: content.to_string(),
        });
        let _ = self
            .file
            .write()
            .await
            .write(format!("{}:{}\n", ts, content).as_bytes())
            .await;
    }

    pub async fn get_entries(&self) -> Vec<DanmuEntry> {
        self.cache.read().await.clone()
    }
}
