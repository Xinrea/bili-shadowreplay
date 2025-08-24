use std::path::{Path, PathBuf};
use tokio::fs;
use serde::{Serialize, Deserialize};

use crate::{RecorderError, RecorderConfig};

/// Storage information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageInfo {
    pub work_dir: PathBuf,
    pub total_size: u64,
    pub file_count: usize,
    pub created_at: i64,
    pub last_modified: i64,
}

/// Storage manager for organizing recording files
pub struct StorageManager {
    base_dir: PathBuf,
    config: RecorderConfig,
}

impl StorageManager {
    /// Create new storage manager
    pub fn new<P: AsRef<Path>>(base_dir: P, config: RecorderConfig) -> Self {
        Self {
            base_dir: base_dir.as_ref().to_path_buf(),
            config,
        }
    }

    /// Create work directory for a recording session
    pub async fn create_work_dir(&self, live_id: &str) -> Result<PathBuf, RecorderError> {
        let work_dir = self.base_dir.join(live_id);
        
        if work_dir.exists() {
            log::warn!("Work directory already exists: {:?}", work_dir);
        }
        
        fs::create_dir_all(&work_dir).await?;
        log::info!("Created work directory: {:?}", work_dir);
        
        Ok(work_dir)
    }

    /// Clean up work directory (remove all files)
    pub async fn cleanup_work_dir<P: AsRef<Path>>(&self, work_dir: P) -> Result<(), RecorderError> {
        let work_dir = work_dir.as_ref();
        
        if !work_dir.exists() {
            return Ok(());
        }

        log::info!("Cleaning up work directory: {:?}", work_dir);
        
        let mut entries = fs::read_dir(work_dir).await?;
        let mut removed_files = 0;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() {
                match fs::remove_file(&path).await {
                    Ok(()) => {
                        removed_files += 1;
                        log::debug!("Removed file: {:?}", path);
                    }
                    Err(e) => {
                        log::error!("Failed to remove file {:?}: {}", path, e);
                    }
                }
            }
        }

        // Remove the directory itself if empty
        match fs::remove_dir(work_dir).await {
            Ok(()) => {
                log::info!("Removed work directory: {:?} ({} files cleaned)", work_dir, removed_files);
            }
            Err(e) => {
                log::warn!("Failed to remove work directory {:?}: {} (but {} files were cleaned)", 
                          work_dir, e, removed_files);
            }
        }

        Ok(())
    }

    /// Get storage information for a work directory
    pub async fn get_storage_info<P: AsRef<Path>>(&self, work_dir: P) -> Result<StorageInfo, RecorderError> {
        let work_dir = work_dir.as_ref();
        
        if !work_dir.exists() {
            return Err(RecorderError::StorageError(
                format!("Work directory does not exist: {:?}", work_dir)
            ));
        }

        let metadata = fs::metadata(work_dir).await?;
        let created_at = metadata.created()
            .unwrap_or_else(|_| std::time::SystemTime::now())
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let last_modified = metadata.modified()
            .unwrap_or_else(|_| std::time::SystemTime::now())
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let mut total_size = 0u64;
        let mut file_count = 0usize;
        
        let mut entries = fs::read_dir(work_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() {
                if let Ok(file_metadata) = fs::metadata(&path).await {
                    total_size += file_metadata.len();
                    file_count += 1;
                }
            }
        }

        Ok(StorageInfo {
            work_dir: work_dir.to_path_buf(),
            total_size,
            file_count,
            created_at,
            last_modified,
        })
    }

    /// List all recording directories
    pub async fn list_recordings(&self) -> Result<Vec<String>, RecorderError> {
        if !self.base_dir.exists() {
            return Ok(Vec::new());
        }

        let mut recordings = Vec::new();
        let mut entries = fs::read_dir(&self.base_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir() {
                if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                    recordings.push(dir_name.to_string());
                }
            }
        }

        recordings.sort();
        Ok(recordings)
    }

    /// Check if work directory exists and is valid
    pub async fn validate_work_dir<P: AsRef<Path>>(&self, work_dir: P) -> Result<bool, RecorderError> {
        let work_dir = work_dir.as_ref();
        
        if !work_dir.exists() {
            return Ok(false);
        }

        let metadata = fs::metadata(work_dir).await?;
        if !metadata.is_dir() {
            return Ok(false);
        }

        // Check if directory is writable by trying to create a temp file
        let test_file = work_dir.join(".test_write");
        match fs::write(&test_file, b"test").await {
            Ok(()) => {
                let _ = fs::remove_file(&test_file).await; // Clean up
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }

    /// Calculate total storage usage
    pub async fn get_total_usage(&self) -> Result<u64, RecorderError> {
        if !self.base_dir.exists() {
            return Ok(0);
        }

        let mut total_size = 0u64;
        let mut stack = vec![self.base_dir.clone()];

        while let Some(current_dir) = stack.pop() {
            let mut entries = fs::read_dir(&current_dir).await?;
            
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                let metadata = fs::metadata(&path).await?;
                
                if metadata.is_file() {
                    total_size += metadata.len();
                } else if metadata.is_dir() {
                    stack.push(path);
                }
            }
        }

        Ok(total_size)
    }

    /// Clean up old recordings (older than specified days)
    pub async fn cleanup_old_recordings(&self, days: u32) -> Result<CleanupResult, RecorderError> {
        let cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() - (days as u64 * 24 * 3600);

        let mut cleanup_result = CleanupResult::default();
        
        if !self.base_dir.exists() {
            return Ok(cleanup_result);
        }

        let mut entries = fs::read_dir(&self.base_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            
            if path.is_dir() {
                if let Ok(metadata) = fs::metadata(&path).await {
                    let created = metadata.created()
                        .or_else(|_| metadata.modified())
                        .unwrap_or_else(|_| std::time::SystemTime::now())
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();

                    if created < cutoff {
                        match self.cleanup_work_dir(&path).await {
                            Ok(()) => {
                                cleanup_result.removed_directories += 1;
                                log::info!("Cleaned up old recording: {:?}", path);
                            }
                            Err(e) => {
                                cleanup_result.errors.push(format!("{:?}: {}", path, e));
                                log::error!("Failed to cleanup {:?}: {}", path, e);
                            }
                        }
                    }
                }
            }
        }

        Ok(cleanup_result)
    }

    /// Get base directory path
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }
}

/// Cleanup operation result
#[derive(Debug, Default)]
pub struct CleanupResult {
    pub removed_directories: usize,
    pub errors: Vec<String>,
}

impl CleanupResult {
    pub fn is_success(&self) -> bool {
        self.errors.is_empty()
    }
}