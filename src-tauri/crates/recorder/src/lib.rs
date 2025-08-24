pub mod entry_store;
pub mod recorder;
pub mod downloader;
pub mod storage;

use thiserror::Error;
use serde::{Serialize, Deserialize};

// Re-export main types
pub use recorder::Recorder;
pub use entry_store::{EntryStore, TsEntry};
pub use downloader::SegmentDownloader;
pub use storage::StorageManager;

#[derive(Error, Debug)]
pub enum RecorderError {
    #[error("NetworkError: {0}")]
    NetworkError(#[from] reqwest::Error),
    #[error("IOError: {0}")]
    IOError(#[from] std::io::Error),
    #[error("ParseError: {0}")]
    ParseError(String),
    #[error("StorageError: {0}")]
    StorageError(String),
    #[error("DownloadError: {0}")]
    DownloadError(String),
    #[error("ConfigError: {0}")]
    ConfigError(String),
}

/// Recording events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecorderEvent {
    /// Recording started
    RecordingStarted { 
        live_id: String,
        start_time: i64,
        work_dir: String,
    },
    /// New segment downloaded
    SegmentDownloaded { 
        live_id: String,
        sequence: u64,
        size: u64,
        duration: f64,
        timestamp: i64,
    },
    /// Recording stopped
    RecordingStopped { 
        live_id: String,
        end_time: i64,
        total_segments: usize,
        total_size: u64,
        total_duration: f64,
    },
    /// Download failed
    DownloadFailed { 
        live_id: String,
        sequence: u64,
        url: String,
        error: String,
        retry_count: u32,
    },
    /// Progress update
    ProgressUpdate {
        live_id: String,
        total_segments: usize,
        total_size: u64,
        total_duration: f64,
        last_updated: i64,
    },
}

/// Recording configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecorderConfig {
    /// Working directory for storing segments
    pub work_dir: String,
    /// Maximum concurrent downloads
    pub max_concurrent_downloads: usize,
    /// Download timeout in seconds
    pub download_timeout: u64,
    /// Maximum retry count for failed downloads
    pub max_retry_count: u32,
    /// Retry delay in milliseconds
    pub retry_delay_ms: u64,
    /// Whether to cleanup incomplete recordings
    pub cleanup_on_error: bool,
    /// Buffer size for segment downloads
    pub download_buffer_size: usize,
}

impl Default for RecorderConfig {
    fn default() -> Self {
        Self {
            work_dir: "recordings".to_string(),
            max_concurrent_downloads: 3,
            download_timeout: 30,
            max_retry_count: 3,
            retry_delay_ms: 1000,
            cleanup_on_error: false,
            download_buffer_size: 64 * 1024, // 64KB
        }
    }
}