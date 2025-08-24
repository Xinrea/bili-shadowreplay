use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Semaphore, mpsc};
use tokio::time::timeout;
use futures_util::StreamExt;

use crate::{RecorderError, RecorderConfig};

/// Download result
#[derive(Debug, Clone)]
pub struct DownloadResult {
    pub url: String,
    pub file_path: String,
    pub size: u64,
    pub duration_ms: u64,
    pub retry_count: u32,
}

/// Download statistics
#[derive(Debug, Clone, Default)]
pub struct DownloadStats {
    pub total_downloads: u64,
    pub successful_downloads: u64,
    pub failed_downloads: u64,
    pub total_bytes: u64,
    pub total_duration_ms: u64,
    pub retry_count: u64,
}

/// Segment downloader with concurrency control
pub struct SegmentDownloader {
    client: reqwest::Client,
    config: RecorderConfig,
    semaphore: Arc<Semaphore>,
    stats: Arc<tokio::sync::RwLock<DownloadStats>>,
}

impl SegmentDownloader {
    /// Create new segment downloader
    pub fn new(config: RecorderConfig) -> Result<Self, RecorderError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.download_timeout))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .map_err(RecorderError::NetworkError)?;

        let semaphore = Arc::new(Semaphore::new(config.max_concurrent_downloads));

        Ok(Self {
            client,
            config,
            semaphore,
            stats: Arc::new(tokio::sync::RwLock::new(DownloadStats::default())),
        })
    }

    /// Download single segment with retry logic
    pub async fn download_segment<P: AsRef<Path>>(
        &self,
        url: &str,
        file_path: P,
        headers: Option<&[(&str, &str)]>,
    ) -> Result<DownloadResult, RecorderError> {
        let file_path = file_path.as_ref();
        let start_time = std::time::Instant::now();

        // Acquire semaphore permit for concurrency control
        let _permit = self.semaphore.acquire().await
            .map_err(|_| RecorderError::DownloadError("Failed to acquire download permit".to_string()))?;

        let mut retry_count = 0;
        let mut last_error = None;

        while retry_count <= self.config.max_retry_count {
            match self.download_with_timeout(url, file_path, headers).await {
                Ok(size) => {
                    let duration_ms = start_time.elapsed().as_millis() as u64;
                    
                    // Update statistics
                    let mut stats = self.stats.write().await;
                    stats.total_downloads += 1;
                    stats.successful_downloads += 1;
                    stats.total_bytes += size;
                    stats.total_duration_ms += duration_ms;
                    stats.retry_count += retry_count as u64;

                    return Ok(DownloadResult {
                        url: url.to_string(),
                        file_path: file_path.to_string_lossy().to_string(),
                        size,
                        duration_ms,
                        retry_count,
                    });
                }
                Err(e) => {
                    retry_count += 1;
                    last_error = Some(e);
                    
                    if retry_count <= self.config.max_retry_count {
                        log::warn!(
                            "Download attempt {} failed for {}: {:?}, retrying in {}ms",
                            retry_count, url, last_error, self.config.retry_delay_ms
                        );
                        
                        tokio::time::sleep(Duration::from_millis(
                            self.config.retry_delay_ms * retry_count as u64
                        )).await;
                    }
                }
            }
        }

        // Update failed download statistics
        let mut stats = self.stats.write().await;
        stats.total_downloads += 1;
        stats.failed_downloads += 1;
        stats.retry_count += retry_count as u64;

        Err(last_error.unwrap_or_else(|| {
            RecorderError::DownloadError(format!("Download failed after {} retries", retry_count))
        }))
    }

    /// Download with timeout
    async fn download_with_timeout<P: AsRef<Path>>(
        &self,
        url: &str,
        file_path: P,
        headers: Option<&[(&str, &str)]>,
    ) -> Result<u64, RecorderError> {
        let download_future = self.download_internal(url, file_path, headers);
        
        timeout(
            Duration::from_secs(self.config.download_timeout),
            download_future
        )
        .await
        .map_err(|_| RecorderError::DownloadError("Download timeout".to_string()))?
    }

    /// Internal download implementation
    async fn download_internal<P: AsRef<Path>>(
        &self,
        url: &str,
        file_path: P,
        headers: Option<&[(&str, &str)]>,
    ) -> Result<u64, RecorderError> {
        let file_path = file_path.as_ref();

        // Build request
        let mut request = self.client.get(url);
        if let Some(headers) = headers {
            for (key, value) in headers {
                request = request.header(*key, *value);
            }
        }

        // Send request and get response
        let response = request.send().await.map_err(RecorderError::NetworkError)?;
        
        if !response.status().is_success() {
            return Err(RecorderError::DownloadError(
                format!("HTTP error {}: {}", response.status(), url)
            ));
        }

        // Create parent directories if needed
        if let Some(parent) = file_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Create file and download stream
        let mut file = File::create(file_path).await?;
        let mut stream = response.bytes_stream();
        let mut total_size = 0u64;

        // Download content in chunks
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(RecorderError::NetworkError)?;
            file.write_all(&chunk).await?;
            total_size += chunk.len() as u64;
        }

        file.flush().await?;

        // Verify file was actually written
        if total_size == 0 {
            return Err(RecorderError::DownloadError(
                "Downloaded file is empty".to_string()
            ));
        }

        log::debug!("Downloaded {} bytes to {:?}", total_size, file_path);
        Ok(total_size)
    }

    /// Download multiple segments concurrently
    pub async fn download_segments_batch<P: AsRef<Path>>(
        &self,
        downloads: Vec<(String, P)>, // (url, file_path)
        headers: Option<&[(&str, &str)]>,
        progress_tx: Option<mpsc::UnboundedSender<DownloadResult>>,
    ) -> Vec<Result<DownloadResult, RecorderError>> {
        let mut handles = Vec::new();

        for (url, file_path) in downloads {
            let downloader = self.clone();
            let url_clone = url.clone();
            let file_path_clone = file_path.as_ref().to_path_buf();
            let headers_clone = headers.map(|h| h.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>());
            let progress_tx_clone = progress_tx.clone();

            let handle = tokio::spawn(async move {
                let headers_ref = headers_clone.as_ref().map(|h| {
                    h.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect::<Vec<_>>()
                });
                
                let result = downloader.download_segment(
                    &url_clone,
                    file_path_clone,
                    headers_ref.as_deref()
                ).await;

                // Send progress update if channel provided
                if let (Ok(download_result), Some(tx)) = (&result, progress_tx_clone) {
                    let _ = tx.send(download_result.clone());
                }

                result
            });

            handles.push(handle);
        }

        // Wait for all downloads to complete
        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(e) => results.push(Err(RecorderError::DownloadError(
                    format!("Download task failed: {}", e)
                ))),
            }
        }

        results
    }

    /// Get download statistics
    pub async fn get_stats(&self) -> DownloadStats {
        self.stats.read().await.clone()
    }

    /// Reset statistics
    pub async fn reset_stats(&self) {
        let mut stats = self.stats.write().await;
        *stats = DownloadStats::default();
    }

    /// Get average download speed in bytes per second
    pub async fn get_average_speed(&self) -> f64 {
        let stats = self.stats.read().await;
        if stats.total_duration_ms > 0 {
            (stats.total_bytes as f64 * 1000.0) / stats.total_duration_ms as f64
        } else {
            0.0
        }
    }

    /// Get success rate as percentage
    pub async fn get_success_rate(&self) -> f64 {
        let stats = self.stats.read().await;
        if stats.total_downloads > 0 {
            (stats.successful_downloads as f64 / stats.total_downloads as f64) * 100.0
        } else {
            100.0
        }
    }
}

impl Clone for SegmentDownloader {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            config: self.config.clone(),
            semaphore: self.semaphore.clone(),
            stats: self.stats.clone(),
        }
    }
}