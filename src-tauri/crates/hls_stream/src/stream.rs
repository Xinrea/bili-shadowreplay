use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, Duration};

use crate::{provider::*, HlsStreamError, StreamEvent, StreamInfo};

#[derive(Clone)]
pub struct HlsStream {
    provider: Arc<RwLock<Box<dyn HlsProvider>>>,
    event_tx: mpsc::UnboundedSender<StreamEvent>,
    event_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<StreamEvent>>>,
    segment_deduplicator: Arc<RwLock<HashSet<u64>>>, // Prevent duplicates
    is_running: Arc<RwLock<bool>>,
}

impl HlsStream {
    /// Create HLS stream parser
    pub async fn new(
        provider_type: ProviderType,
        room_id: &str,
        auth: &str,
    ) -> Result<Self, HlsStreamError> {
        let (tx, rx) = mpsc::unbounded_channel();
        let provider = create_provider(provider_type, room_id, auth).await?;

        Ok(Self {
            provider: Arc::new(RwLock::new(provider)),
            event_tx: tx,
            event_rx: Arc::new(tokio::sync::Mutex::new(rx)),
            segment_deduplicator: Arc::new(RwLock::new(HashSet::new())),
            is_running: Arc::new(RwLock::new(false)),
        })
    }

    /// Start stream parsing (no data downloading)
    pub async fn start(&self) -> Result<(), HlsStreamError> {
        let mut running = self.is_running.write().await;
        if *running {
            return Ok(()); // Already running
        }
        *running = true;
        drop(running);

        let provider = self.provider.clone();
        let tx = self.event_tx.clone();
        let deduplicator = self.segment_deduplicator.clone();
        let is_running = self.is_running.clone();

        // Send stream started event
        let info = self.provider.read().await.get_info().await?;
        let _ = tx.send(StreamEvent::StreamStarted {
            live_id: info.live_id.clone(),
            playlist_url: info.playlist_url.clone(),
            target_duration: info.target_duration,
        });

        tokio::spawn(async move {
            Self::parse_stream_loop(provider, tx, deduplicator, is_running).await;
        });

        Ok(())
    }

    /// Stop stream parsing
    pub async fn stop(&self) -> Result<(), HlsStreamError> {
        let mut running = self.is_running.write().await;
        *running = false;
        drop(running);

        self.provider.write().await.stop().await?;
        let _ = self.event_tx.send(StreamEvent::StreamEnded);
        Ok(())
    }

    /// Receive stream events
    pub async fn recv(&self) -> Option<StreamEvent> {
        self.event_rx.lock().await.recv().await
    }

    /// Get current stream information
    pub async fn info(&self) -> Result<StreamInfo, HlsStreamError> {
        self.provider.read().await.get_info().await
    }

    /// Change quality (only updates parsing URL)
    pub async fn change_quality(&self, quality: &str) -> Result<(), HlsStreamError> {
        let old_info = self.provider.read().await.get_info().await?;
        self.provider.write().await.change_quality(quality).await?;
        let new_info = self.provider.read().await.get_info().await?;

        let _ = self.event_tx.send(StreamEvent::QualityChanged {
            from: old_info.current_quality,
            to: new_info.current_quality,
            new_playlist_url: new_info.playlist_url,
        });

        Ok(())
    }

    /// Check if stream is running
    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }

    // Core parsing loop
    async fn parse_stream_loop(
        provider: Arc<RwLock<Box<dyn HlsProvider>>>,
        tx: mpsc::UnboundedSender<StreamEvent>,
        deduplicator: Arc<RwLock<HashSet<u64>>>,
        is_running: Arc<RwLock<bool>>,
    ) {
        let mut refresh_interval = interval(Duration::from_secs(1));
        let mut last_sequence = 0u64;
        let mut consecutive_errors = 0u32;
        const MAX_CONSECUTIVE_ERRORS: u32 = 5;

        while *is_running.read().await {
            refresh_interval.tick().await;

            let playlist_result = {
                let provider_guard = provider.read().await;
                provider_guard.fetch_playlist().await
            };

            match playlist_result {
                Ok(segments) => {
                    consecutive_errors = 0; // Reset error counter on success
                    let mut new_segments = 0;
                    let mut dedup_guard = deduplicator.write().await;

                    for segment in segments {
                        // Prevent duplicate processing
                        if segment.sequence <= last_sequence
                            || dedup_guard.contains(&segment.sequence)
                        {
                            continue;
                        }

                        dedup_guard.insert(segment.sequence);
                        last_sequence = segment.sequence;
                        new_segments += 1;

                        // Send new segment event (metadata only)
                        if tx.send(StreamEvent::NewSegment(segment)).is_err() {
                            log::debug!("Event receiver closed, stopping parse loop");
                            return; // Receiver closed
                        }
                    }

                    if new_segments > 0 {
                        let _ = tx.send(StreamEvent::PlaylistRefreshed {
                            total_segments: dedup_guard.len(),
                            new_segments,
                        });
                    }

                    // Clean old sequences (keep last 1000)
                    if dedup_guard.len() > 1000 {
                        let min_keep = last_sequence.saturating_sub(1000);
                        dedup_guard.retain(|&seq| seq >= min_keep);
                    }
                }
                Err(e) => {
                    consecutive_errors += 1;
                    log::error!(
                        "Failed to fetch playlist (attempt {}): {}",
                        consecutive_errors,
                        e
                    );

                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                        log::error!("Too many consecutive errors, stopping stream");
                        let _ = tx.send(StreamEvent::StreamEnded);
                        break;
                    }

                    // Exponential backoff for retries
                    let delay = Duration::from_secs((consecutive_errors * 2).min(30) as u64);
                    tokio::time::sleep(delay).await;
                }
            }
        }

        log::debug!("Parse loop ended");
    }
}
