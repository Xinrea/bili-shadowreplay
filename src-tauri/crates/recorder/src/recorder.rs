use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, RwLock, Mutex};
use tokio::time::interval;
use hls_stream::{HlsStream, ProviderType, StreamEvent, HlsSegment};

use crate::{
    RecorderError, RecorderConfig, RecorderEvent,
    EntryStore, TsEntry, SegmentDownloader, StorageManager,
    entry_store::RecordingStats
};

/// Recording session state
#[derive(Debug, Clone)]
pub enum RecordingState {
    Idle,
    Starting,
    Recording,
    Stopping,
    Stopped,
    Error(String),
}

/// Recorder for downloading and storing HLS streams
pub struct Recorder {
    /// Unique identifier for this recording session
    live_id: String,
    /// HLS stream parser
    hls_stream: HlsStream,
    /// Segment downloader
    downloader: SegmentDownloader,
    /// Storage manager
    storage_manager: StorageManager,
    /// Entry store for managing segments
    entry_store: Arc<Mutex<Option<EntryStore>>>,
    /// Recording configuration
    config: RecorderConfig,
    /// Current recording state
    state: Arc<RwLock<RecordingState>>,
    /// Work directory for this recording
    work_dir: Arc<RwLock<Option<PathBuf>>>,
    /// Event sender for recording events
    event_tx: mpsc::UnboundedSender<RecorderEvent>,
    /// Event receiver (for external consumption)
    event_rx: Arc<Mutex<mpsc::UnboundedReceiver<RecorderEvent>>>,
    /// Stop signal
    stop_signal: Arc<RwLock<bool>>,
    /// Downloaded segment tracker (for deduplication)
    downloaded_segments: Arc<RwLock<HashSet<u64>>>,
    /// Recording start time
    start_time: Arc<RwLock<Option<Instant>>>,
}

impl Recorder {
    /// Create new recorder
    pub async fn new(
        provider_type: ProviderType,
        room_id: &str,
        auth: &str,
        config: RecorderConfig,
    ) -> Result<Self, RecorderError> {
        let live_id = format!("{}_{}", 
            provider_type.as_str(), 
            chrono::Utc::now().timestamp_millis()
        );

        // Create HLS stream
        let hls_stream = HlsStream::new(provider_type, room_id, auth).await
            .map_err(|e| RecorderError::ConfigError(format!("Failed to create HLS stream: {}", e)))?;

        // Create downloader
        let downloader = SegmentDownloader::new(config.clone())?;

        // Create storage manager
        let storage_manager = StorageManager::new(&config.work_dir, config.clone());

        // Create event channel
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Ok(Self {
            live_id,
            hls_stream,
            downloader,
            storage_manager,
            entry_store: Arc::new(Mutex::new(None)),
            config,
            state: Arc::new(RwLock::new(RecordingState::Idle)),
            work_dir: Arc::new(RwLock::new(None)),
            event_tx,
            event_rx: Arc::new(Mutex::new(event_rx)),
            stop_signal: Arc::new(RwLock::new(false)),
            downloaded_segments: Arc::new(RwLock::new(HashSet::new())),
            start_time: Arc::new(RwLock::new(None)),
        })
    }

    /// Start recording
    pub async fn start(&self) -> Result<(), RecorderError> {
        // Check current state
        {
            let state = self.state.read().await;
            match *state {
                RecordingState::Recording => return Ok(()), // Already recording
                RecordingState::Starting => return Ok(()), // Already starting
                _ => {}
            }
        }

        // Set state to starting
        *self.state.write().await = RecordingState::Starting;
        *self.stop_signal.write().await = false;
        *self.start_time.write().await = Some(Instant::now());

        log::info!("Starting recording for live_id: {}", self.live_id);

        // Create work directory
        let work_dir = self.storage_manager.create_work_dir(&self.live_id).await?;
        *self.work_dir.write().await = Some(work_dir.clone());

        // Initialize entry store
        let entry_store = EntryStore::new(&work_dir).await?;
        *self.entry_store.lock().await = Some(entry_store);

        // Start HLS stream
        self.hls_stream.start().await
            .map_err(|e| RecorderError::ConfigError(format!("Failed to start HLS stream: {}", e)))?;

        // Start recording loop
        let recorder = self.clone();
        tokio::spawn(async move {
            if let Err(e) = recorder.recording_loop().await {
                log::error!("Recording loop error: {}", e);
                let _ = recorder.set_error_state(format!("Recording error: {}", e)).await;
            }
        });

        // Send started event
        let _ = self.event_tx.send(RecorderEvent::RecordingStarted {
            live_id: self.live_id.clone(),
            start_time: chrono::Utc::now().timestamp_millis(),
            work_dir: work_dir.to_string_lossy().to_string(),
        });

        Ok(())
    }

    /// Stop recording
    pub async fn stop(&self) -> Result<(), RecorderError> {
        log::info!("Stopping recording for live_id: {}", self.live_id);

        // Set stop signal
        *self.stop_signal.write().await = true;
        *self.state.write().await = RecordingState::Stopping;

        // Stop HLS stream
        self.hls_stream.stop().await
            .map_err(|e| RecorderError::ConfigError(format!("Failed to stop HLS stream: {}", e)))?;

        // Get final statistics
        let stats = self.get_stats().await;
        let end_time = chrono::Utc::now().timestamp_millis();

        // Send stopped event
        let _ = self.event_tx.send(RecorderEvent::RecordingStopped {
            live_id: self.live_id.clone(),
            end_time,
            total_segments: stats.total_segments,
            total_size: stats.total_size,
            total_duration: stats.total_duration,
        });

        *self.state.write().await = RecordingState::Stopped;
        log::info!("Recording stopped for live_id: {}", self.live_id);

        Ok(())
    }

    /// Main recording loop
    async fn recording_loop(&self) -> Result<(), RecorderError> {
        *self.state.write().await = RecordingState::Recording;
        log::info!("Recording loop started for live_id: {}", self.live_id);

        // Progress reporting interval
        let mut progress_interval = interval(Duration::from_secs(5));

        while !*self.stop_signal.read().await {
            tokio::select! {
                // Handle HLS stream events
                stream_event = self.hls_stream.recv() => {
                    match stream_event {
                        Some(event) => {
                            if let Err(e) = self.handle_stream_event(event).await {
                                log::error!("Error handling stream event: {}", e);
                            }
                        }
                        None => {
                            log::info!("HLS stream ended");
                            break;
                        }
                    }
                }
                
                // Send periodic progress updates
                _ = progress_interval.tick() => {
                    self.send_progress_update().await;
                }
            }
        }

        log::info!("Recording loop ended for live_id: {}", self.live_id);
        Ok(())
    }

    /// Handle HLS stream events
    async fn handle_stream_event(&self, event: StreamEvent) -> Result<(), RecorderError> {
        match event {
            StreamEvent::NewSegment(segment) => {
                self.handle_new_segment(segment).await?;
            }
            StreamEvent::StreamEnded => {
                log::info!("Stream ended for live_id: {}", self.live_id);
                *self.stop_signal.write().await = true;
            }
            StreamEvent::QualityChanged { from, to, .. } => {
                log::info!("Quality changed from {} to {} for live_id: {}", from, to, self.live_id);
            }
            _ => {
                // Handle other events as needed
                log::debug!("Received stream event: {:?}", event);
            }
        }
        Ok(())
    }

    /// Handle new segment from stream
    async fn handle_new_segment(&self, segment: HlsSegment) -> Result<(), RecorderError> {
        // Check for duplicates
        {
            let mut downloaded = self.downloaded_segments.write().await;
            if downloaded.contains(&segment.sequence) {
                return Ok(());
            }
            downloaded.insert(segment.sequence);
            
            // Limit memory usage by keeping only recent sequences
            if downloaded.len() > 1000 {
                let min_keep = segment.sequence.saturating_sub(500);
                downloaded.retain(|&seq| seq >= min_keep);
            }
        }

        // Get work directory
        let work_dir = {
            let work_dir_lock = self.work_dir.read().await;
            work_dir_lock.as_ref()
                .ok_or_else(|| RecorderError::ConfigError("No work directory set".to_string()))?
                .clone()
        };

        // Generate filename
        let filename = self.extract_filename(&segment.url);
        let file_path = work_dir.join(&filename);

        // Download segment
        log::debug!("Downloading segment {} for live_id: {}", segment.sequence, self.live_id);
        
        let download_result = self.downloader.download_segment(
            &segment.url,
            &file_path,
            Some(&[("Referer", "https://live.bilibili.com/")])
        ).await;

        match download_result {
            Ok(result) => {
                // Create entry with precise timing if available
                let mut entry = TsEntry::new(
                    filename,
                    segment.sequence,
                    segment.duration,
                    result.size,
                    segment.timestamp,
                    false,
                );

                // Add BILI-AUX timing if available
                if let Some(bili_aux_offset) = segment.get_metadata::<i64>("bili_aux_offset") {
                    entry = entry.with_bili_aux_offset(bili_aux_offset);
                }

                // Add to entry store
                if let Some(ref mut entry_store) = *self.entry_store.lock().await {
                    entry_store.add_entry(entry).await?;
                }

                // Send segment downloaded event
                let _ = self.event_tx.send(RecorderEvent::SegmentDownloaded {
                    live_id: self.live_id.clone(),
                    sequence: segment.sequence,
                    size: result.size,
                    duration: segment.duration,
                    timestamp: segment.timestamp,
                });

                log::debug!("Successfully downloaded segment {} ({} bytes)", 
                           segment.sequence, result.size);
            }
            Err(e) => {
                log::error!("Failed to download segment {}: {}", segment.sequence, e);
                
                // Send download failed event
                let _ = self.event_tx.send(RecorderEvent::DownloadFailed {
                    live_id: self.live_id.clone(),
                    sequence: segment.sequence,
                    url: segment.url.clone(),
                    error: e.to_string(),
                    retry_count: 0, // TODO: Get actual retry count from downloader
                });

                return Err(e);
            }
        }

        Ok(())
    }

    /// Send progress update
    async fn send_progress_update(&self) {
        if let Ok(stats) = self.get_stats_internal().await {
            let _ = self.event_tx.send(RecorderEvent::ProgressUpdate {
                live_id: self.live_id.clone(),
                total_segments: stats.total_segments,
                total_size: stats.total_size,
                total_duration: stats.total_duration,
                last_updated: chrono::Utc::now().timestamp_millis(),
            });
        }
    }

    /// Set error state
    async fn set_error_state(&self, error: String) {
        *self.state.write().await = RecordingState::Error(error.clone());
        
        if self.config.cleanup_on_error {
            if let Some(work_dir) = self.work_dir.read().await.as_ref() {
                if let Err(e) = self.storage_manager.cleanup_work_dir(work_dir).await {
                    log::error!("Failed to cleanup work directory on error: {}", e);
                }
            }
        }
    }

    /// Extract filename from URL
    fn extract_filename(&self, url: &str) -> String {
        url.split('/')
            .last()
            .unwrap_or("segment")
            .split('?')
            .next()
            .unwrap_or("segment")
            .to_string()
    }

    /// Get recording statistics
    pub async fn get_stats(&self) -> RecordingStats {
        self.get_stats_internal().await.unwrap_or_default()
    }

    /// Internal get stats implementation
    async fn get_stats_internal(&self) -> Result<RecordingStats, RecorderError> {
        let entry_store = self.entry_store.lock().await;
        match entry_store.as_ref() {
            Some(store) => Ok(store.get_stats()),
            None => Ok(RecordingStats {
                total_segments: 0,
                total_duration: 0.0,
                total_size: 0,
                last_sequence: 0,
                has_header: false,
                start_time: None,
                end_time: None,
            })
        }
    }

    /// Get current recording state
    pub async fn get_state(&self) -> RecordingState {
        self.state.read().await.clone()
    }

    /// Get live ID
    pub fn live_id(&self) -> &str {
        &self.live_id
    }

    /// Get event receiver
    pub async fn get_event_receiver(&self) -> mpsc::UnboundedReceiver<RecorderEvent> {
        let mut rx_guard = self.event_rx.lock().await;
        let (new_tx, new_rx) = mpsc::unbounded_channel();
        
        // Move existing messages to new receiver
        while let Ok(event) = rx_guard.try_recv() {
            let _ = new_tx.send(event);
        }
        
        new_rx
    }

    /// Generate M3U8 for recorded content
    pub async fn generate_m3u8(&self, vod: bool) -> Result<String, RecorderError> {
        let entry_store = self.entry_store.lock().await;
        match entry_store.as_ref() {
            Some(store) => Ok(store.generate_m3u8(vod, None)),
            None => Err(RecorderError::ConfigError("No entry store available".to_string()))
        }
    }

    /// Get work directory path
    pub async fn get_work_dir(&self) -> Option<PathBuf> {
        self.work_dir.read().await.clone()
    }
}

impl Clone for Recorder {
    fn clone(&self) -> Self {
        Self {
            live_id: self.live_id.clone(),
            hls_stream: self.hls_stream.clone(),
            downloader: self.downloader.clone(),
            storage_manager: StorageManager::new(self.storage_manager.base_dir(), self.config.clone()),
            entry_store: self.entry_store.clone(),
            config: self.config.clone(),
            state: self.state.clone(),
            work_dir: self.work_dir.clone(),
            event_tx: self.event_tx.clone(),
            event_rx: self.event_rx.clone(),
            stop_signal: self.stop_signal.clone(),
            downloaded_segments: self.downloaded_segments.clone(),
            start_time: self.start_time.clone(),
        }
    }
}

impl Default for RecordingStats {
    fn default() -> Self {
        Self {
            total_segments: 0,
            total_duration: 0.0,
            total_size: 0,
            last_sequence: 0,
            has_header: false,
            start_time: None,
            end_time: None,
        }
    }
}