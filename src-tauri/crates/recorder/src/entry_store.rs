use std::path::{Path, PathBuf};
use std::fmt::{self, Display};
use serde::{Serialize, Deserialize};
use chrono::{TimeZone, Utc};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::RecorderError;

const ENTRY_FILE_NAME: &str = "entries.log";

/// HLS segment entry for persistent storage
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TsEntry {
    /// Segment filename (relative to work directory)
    pub url: String,
    /// Segment sequence number
    pub sequence: u64,
    /// Segment duration in seconds
    pub length: f64,
    /// Segment file size in bytes
    pub size: u64,
    /// Timestamp when segment was created (milliseconds)
    pub ts: i64,
    /// Whether this is a header/initialization segment
    pub is_header: bool,
    /// Optional BILI-AUX offset for precise timing
    pub bili_aux_offset: Option<i64>,
}

impl TsEntry {
    pub fn new(
        url: String, 
        sequence: u64, 
        length: f64, 
        size: u64, 
        ts: i64, 
        is_header: bool
    ) -> Self {
        Self {
            url,
            sequence,
            length,
            size,
            ts,
            is_header,
            bili_aux_offset: None,
        }
    }

    pub fn with_bili_aux_offset(mut self, offset: i64) -> Self {
        self.bili_aux_offset = Some(offset);
        self
    }

    /// Parse entry from log line
    pub fn from_line(line: &str) -> Result<Self, RecorderError> {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() < 6 {
            return Err(RecorderError::ParseError(
                "Invalid entry format: expected at least 6 fields".to_string()
            ));
        }

        let bili_aux_offset = if parts.len() >= 7 && !parts[6].is_empty() {
            Some(parts[6].parse().map_err(|e| {
                RecorderError::ParseError(format!("Failed to parse bili_aux_offset: {}", e))
            })?)
        } else {
            None
        };

        Ok(TsEntry {
            url: parts[0].to_string(),
            sequence: parts[1].parse().map_err(|e| {
                RecorderError::ParseError(format!("Failed to parse sequence: {}", e))
            })?,
            length: parts[2].parse().map_err(|e| {
                RecorderError::ParseError(format!("Failed to parse length: {}", e))
            })?,
            size: parts[3].parse().map_err(|e| {
                RecorderError::ParseError(format!("Failed to parse size: {}", e))
            })?,
            ts: parts[4].parse().map_err(|e| {
                RecorderError::ParseError(format!("Failed to parse timestamp: {}", e))
            })?,
            is_header: parts[5].parse().map_err(|e| {
                RecorderError::ParseError(format!("Failed to parse is_header: {}", e))
            })?,
            bili_aux_offset,
        })
    }

    /// Get timestamp in seconds (handles legacy ms/s conversion)
    pub fn ts_seconds(&self) -> i64 {
        if self.ts > 10000000000 {
            self.ts / 1000  // Convert ms to s
        } else {
            self.ts  // Already in seconds
        }
    }

    /// Get timestamp in milliseconds 
    pub fn ts_millis(&self) -> i64 {
        if self.ts > 10000000000 {
            self.ts  // Already in ms
        } else {
            self.ts * 1000  // Convert s to ms
        }
    }

    /// Generate program date time tag
    pub fn program_date_time(&self) -> String {
        let dt = Utc.timestamp_opt(self.ts_seconds(), 0).unwrap();
        format!("#EXT-X-PROGRAM-DATE-TIME:{}\n", dt.to_rfc3339())
    }

    /// Convert to M3U8 segment entry
    pub fn to_m3u8_segment(&self) -> String {
        if self.is_header {
            return String::new();
        }
        format!("#EXTINF:{:.4},\n{}\n", self.length, self.url)
    }
}

impl Display for TsEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}|{}|{}|{}|{}|{}|{}",
            self.url,
            self.sequence,
            self.length,
            self.size,
            self.ts,
            self.is_header,
            self.bili_aux_offset.map_or(String::new(), |v| v.to_string())
        )
    }
}

/// Time range for M3U8 generation
#[derive(Debug, Clone)]
pub struct TimeRange {
    pub start: f32,
    pub end: f32,
}

impl TimeRange {
    pub fn new(start: f32, end: f32) -> Self {
        Self { start, end }
    }

    pub fn contains(&self, time: f32) -> bool {
        time >= self.start && time <= self.end
    }
}

/// Entry store for managing HLS segments
pub struct EntryStore {
    work_dir: PathBuf,
    log_file: File,
    entries: Vec<TsEntry>,
    header: Option<TsEntry>,
    
    // Statistics
    last_sequence: u64,
    total_duration: f64,
    total_size: u64,
}

impl EntryStore {
    /// Create new entry store
    pub async fn new<P: AsRef<Path>>(work_dir: P) -> Result<Self, RecorderError> {
        let work_dir = work_dir.as_ref().to_path_buf();
        
        // Ensure work directory exists
        tokio::fs::create_dir_all(&work_dir).await?;
        
        let log_path = work_dir.join(ENTRY_FILE_NAME);
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await?;

        let mut store = Self {
            work_dir,
            log_file,
            entries: Vec::new(),
            header: None,
            last_sequence: 0,
            total_duration: 0.0,
            total_size: 0,
        };

        // Load existing entries if log file exists
        store.load_existing_entries().await?;
        
        Ok(store)
    }

    /// Load entries from existing log file
    async fn load_existing_entries(&mut self) -> Result<(), RecorderError> {
        let log_path = self.work_dir.join(ENTRY_FILE_NAME);
        
        if log_path.exists() {
            let file = File::open(&log_path).await?;
            let reader = BufReader::new(file);
            let mut lines = reader.lines();

            while let Some(line) = lines.next_line().await? {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                match TsEntry::from_line(line) {
                    Ok(entry) => {
                        self.last_sequence = self.last_sequence.max(entry.sequence);
                        self.total_duration += entry.length;
                        self.total_size += entry.size;

                        if entry.is_header {
                            self.header = Some(entry);
                        } else {
                            self.entries.push(entry);
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to parse entry line '{}': {}", line, e);
                    }
                }
            }

            log::info!(
                "Loaded {} entries from existing log (total: {:.2}s, {:.2}MB)",
                self.entries.len(),
                self.total_duration,
                self.total_size as f64 / 1024.0 / 1024.0
            );
        }

        Ok(())
    }

    /// Add new segment entry
    pub async fn add_entry(&mut self, entry: TsEntry) -> Result<(), RecorderError> {
        // Update in-memory state
        if entry.is_header {
            self.header = Some(entry.clone());
        } else {
            self.entries.push(entry.clone());
        }

        // Update statistics
        self.last_sequence = self.last_sequence.max(entry.sequence);
        self.total_duration += entry.length;
        self.total_size += entry.size;

        // Persist to log file
        self.log_file.write_all(format!("{}\n", entry).as_bytes()).await?;
        self.log_file.flush().await?;

        Ok(())
    }

    /// Generate M3U8 manifest for given time range
    pub fn generate_m3u8(&self, vod: bool, time_range: Option<TimeRange>) -> String {
        let mut content = String::new();
        
        // M3U8 header
        content.push_str("#EXTM3U\n");
        content.push_str("#EXT-X-VERSION:6\n");
        
        if vod {
            content.push_str("#EXT-X-PLAYLIST-TYPE:VOD\n");
        } else {
            content.push_str("#EXT-X-PLAYLIST-TYPE:EVENT\n");
        }

        // Calculate target duration
        if !self.entries.is_empty() {
            let max_duration = self.entries.iter()
                .map(|e| e.length)
                .fold(0.0_f64, f64::max);
            content.push_str(&format!("#EXT-X-TARGETDURATION:{}\n", max_duration.ceil() as u64));
        } else {
            content.push_str("#EXT-X-TARGETDURATION:6\n");
        }

        // Add initialization segment (header)
        if let Some(header) = &self.header {
            content.push_str(&format!("#EXT-X-MAP:URI=\"{}\"\n", header.url));
        }

        // Filter entries by time range
        let first_entry_ts = self.entries.first().map(|e| e.ts_seconds()).unwrap_or(0);
        let filtered_entries: Vec<_> = if let Some(range) = time_range {
            self.entries.iter().filter(|entry| {
                let relative_time = (entry.ts_seconds() - first_entry_ts) as f32;
                range.contains(relative_time)
            }).collect()
        } else {
            self.entries.iter().collect()
        };

        // Add segment entries
        let mut prev_sequence = 0u64;
        for (i, entry) in filtered_entries.iter().enumerate() {
            // Detect discontinuity
            if entry.sequence < prev_sequence || (prev_sequence > 0 && entry.sequence - prev_sequence > 1) {
                content.push_str("#EXT-X-DISCONTINUITY\n");
            }

            // Add program date time for key segments
            if i == 0 || i == filtered_entries.len() - 1 {
                content.push_str(&entry.program_date_time());
            }

            // Add segment
            content.push_str(&entry.to_m3u8_segment());
            prev_sequence = entry.sequence;
        }

        // Add end marker for VOD
        if vod {
            content.push_str("#EXT-X-ENDLIST\n");
        }

        content
    }

    /// Get recording statistics
    pub fn get_stats(&self) -> RecordingStats {
        RecordingStats {
            total_segments: self.entries.len(),
            total_duration: self.total_duration,
            total_size: self.total_size,
            last_sequence: self.last_sequence,
            has_header: self.header.is_some(),
            start_time: self.entries.first().map(|e| e.ts),
            end_time: self.entries.last().map(|e| e.ts),
        }
    }

    /// Get entries in time range
    pub fn get_entries_in_range(&self, range: TimeRange) -> Vec<&TsEntry> {
        let first_entry_ts = self.entries.first().map(|e| e.ts_seconds()).unwrap_or(0);
        
        self.entries.iter().filter(|entry| {
            let relative_time = (entry.ts_seconds() - first_entry_ts) as f32;
            range.contains(relative_time)
        }).collect()
    }

    /// Get work directory path
    pub fn work_dir(&self) -> &Path {
        &self.work_dir
    }
}

/// Recording statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingStats {
    pub total_segments: usize,
    pub total_duration: f64,
    pub total_size: u64,
    pub last_sequence: u64,
    pub has_header: bool,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
}