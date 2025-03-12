use async_std::{
    fs::{File, OpenOptions}, io::{prelude::BufReadExt, BufReader, WriteExt}, path::Path, stream::StreamExt
};

const ENTRY_FILE_NAME: &str = "entries.log";

#[derive(Clone)]
pub struct TsEntry {
    pub url: String,
    pub sequence: u64,
    pub length: f64,
    pub size: u64,
    pub ts: i64,
    pub is_header: bool,
}

pub struct EntryStore {
    // append only log file
    log_file: File,
    header: Option<TsEntry>,
    entries: Vec<TsEntry>,
    total_duration: f64,
    total_size: u64,
    last_sequence: u64,

    pub continue_sequence: u64,
}

impl EntryStore {
    pub async fn new(work_dir: &str) -> Self {
        // if work_dir is not exists, create it
        if !Path::new(work_dir).exists().await {
            std::fs::create_dir_all(work_dir).unwrap();
        }
        // open append only log file
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(format!("{}/{}", work_dir, ENTRY_FILE_NAME))
            .await
            .unwrap();
        let mut entry_store = Self {
            log_file,
            header: None,
            entries: vec![],
            total_duration: 0.0,
            total_size: 0,
            last_sequence: 0,
            continue_sequence: 0,
        };

        entry_store.load(work_dir).await;

        entry_store
    }

    async fn load(&mut self, work_dir: &str) {
        let file = OpenOptions::new()
            .create(false)
            .read(true)
            .open(format!("{}/{}", work_dir, ENTRY_FILE_NAME))
            .await
            .unwrap();
        let mut lines = BufReader::new(file).lines();
        while let Some(Ok(line)) = lines.next().await {
            let parts: Vec<&str> = line.split('|').collect();
            let entry = TsEntry {
                url: parts[0].to_string(),
                sequence: parts[1].parse().unwrap(),
                length: parts[2].parse().unwrap(),
                size: parts[3].parse().unwrap(),
                ts: parts[4].parse().unwrap(),
                is_header: parts[5].parse().unwrap(),
            };

            if entry.sequence > self.last_sequence {
                self.last_sequence = entry.sequence;
            }

            if entry.is_header {
                self.header = Some(entry.clone());
            } else {
                self.entries.push(entry.clone());
            }

            self.total_duration += entry.length;
            self.total_size += entry.size;
        }

        self.continue_sequence = self.last_sequence + 100;
    }

    pub async fn add_entry(&mut self, entry: TsEntry) {
        if entry.is_header {
            self.header = Some(entry.clone());
        } else {
            self.entries.push(entry.clone());
        }

        if let Err(e) = self
            .log_file
            .write_all(
                format!(
                    "{}|{}|{}|{}|{}|{}\n",
                    entry.url, entry.sequence, entry.length, entry.size, entry.ts, entry.is_header
                )
                .as_bytes(),
            )
            .await
        {
            log::error!("Failed to write entry to log file: {}", e);
        }
        self.log_file.flush().await.unwrap();

        if self.last_sequence < entry.sequence {
            self.last_sequence = entry.sequence;
        }

        self.total_duration += entry.length;
        self.total_size += entry.size;
    }

    pub fn get_header(&self) -> Option<&TsEntry> {
        self.header.as_ref()
    }

    pub fn get_entries(&self) -> &Vec<TsEntry> {
        &self.entries
    }

    pub fn total_duration(&self) -> f64 {
        self.total_duration
    }

    pub fn total_size(&self) -> u64 {
        self.total_size
    }


    pub fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    pub fn last_ts(&self) -> Option<i64> {
        self.entries.last().map(|entry| entry.ts)
    }
}
