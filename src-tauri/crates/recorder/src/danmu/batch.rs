use super::event_line;
use danmu_stream::LiveEvent;
use recording_storage::{AppendLog, RecordingStorage, LOG_CHUNK_BYTES};
use std::{io, time::Duration};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::{sleep, sleep_until, Instant},
};

const BATCH_DELAY: Duration = Duration::from_secs(5);
pub(super) const QUEUED_EVENTS: usize = 128;

struct QueuedEvent {
    bytes: Vec<u8>,
    received: Instant,
}

/// The recording hot path: open only the append cursor, never replay history.
///
/// Acceptance queues a complete JSONL event, not a durable remote write. A
/// worker commits at 64 KiB or five seconds from the batch's first arrival.
/// Normal close drains the queue; Drop also initiates draining, but callers
/// must await close (or the store's log drain) before reopening/deleting assets.
/// There is no crash-recovery spool: process loss can lose uncommitted events.
pub struct DanmuWriter {
    sender: mpsc::Sender<QueuedEvent>,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<io::Result<()>>,
}

impl DanmuWriter {
    pub async fn from_storage(storage: &RecordingStorage, key: &str) -> io::Result<Self> {
        let log = storage.open_log(key).await?;
        let (sender, receiver) = mpsc::channel(QUEUED_EVENTS);
        let (shutdown, stopped) = oneshot::channel();
        let context = format!("storage={:?} key={key}", storage.identity());
        let task = storage.spawn_log_task(async move {
            let mut writer = BatchWriter {
                log,
                receiver,
                stopped,
                closing: false,
                pending: Vec::new(),
                deadline: None,
                context,
            };
            let result = writer.run().await;
            if let Err(error) = &result {
                log::error!(
                    "Event batch shutdown failed: {} unconfirmed_bytes={} queued_events={} error_kind={:?}; buffered events may be missing from the archive",
                    writer.context, writer.pending.len(), writer.receiver.len(), error.kind()
                );
            }
            result
        });
        Ok(Self {
            sender,
            shutdown,
            task,
        })
    }

    /// Own the event and sender so callers can release their recording-state
    /// lock before waiting for capacity. Closing rejects outstanding enqueues.
    pub fn add_event(
        &self,
        event: &LiveEvent,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send + 'static {
        let bytes = event_line(event);
        let received = Instant::now();
        let sender = self.sender.clone();
        async move {
            sender
                .send(QueuedEvent {
                    bytes: bytes?,
                    received,
                })
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "event batch writer stopped")
                })
        }
    }

    /// Stop accepting events and wait for every accepted event's final commit.
    /// Cancelling this wait does not cancel the worker or release its log lease.
    pub async fn close(self) -> io::Result<()> {
        let Self {
            sender,
            shutdown,
            task,
        } = self;
        drop(sender);
        drop(shutdown);
        task.await.map_err(io::Error::other)?
    }
}

struct BatchWriter {
    log: AppendLog,
    receiver: mpsc::Receiver<QueuedEvent>,
    stopped: oneshot::Receiver<()>,
    closing: bool,
    pending: Vec<u8>,
    deadline: Option<Instant>,
    context: String,
}

impl BatchWriter {
    async fn run(&mut self) -> io::Result<()> {
        loop {
            tokio::select! {
                biased;
                _ = &mut self.stopped, if !self.closing => {
                    self.closing = true;
                    self.receiver.close();
                }
                // New arrivals never postpone the oldest event's deadline.
                _ = sleep_until(self.deadline.unwrap_or_else(Instant::now)), if self.deadline.is_some() => {
                    self.fill_ready();
                    self.commit().await?;
                }
                event = self.receiver.recv() => {
                    let Some(event) = event else {
                        self.closing = true;
                        return self.commit().await;
                    };
                    self.push(event);
                    self.fill_ready();
                    // Keep each event whole in a committed batch. The object
                    // log can split an oversized event into bounded chunks.
                    if self.pending.len() >= LOG_CHUNK_BYTES {
                        self.commit().await?;
                    }
                }
            }
        }
    }

    fn push(&mut self, event: QueuedEvent) {
        if self.pending.is_empty() {
            self.deadline = Some(event.received + BATCH_DELAY);
        }
        self.pending.extend_from_slice(&event.bytes);
    }

    fn fill_ready(&mut self) {
        // Catch up in batches after a slow upload, even when queued events'
        // deadlines have already passed. Never flush one object per late event.
        while self.pending.len() < LOG_CHUNK_BYTES {
            let Ok(event) = self.receiver.try_recv() else {
                break;
            };
            self.push(event);
        }
    }

    async fn commit(&mut self) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        loop {
            let started = Instant::now();
            match self.log.append(&self.pending).await {
                Ok(()) => {
                    log::debug!(
                        "Event batch committed: {} bytes={} duration_ms={}",
                        self.context,
                        self.pending.len(),
                        started.elapsed().as_millis()
                    );
                    self.pending.clear();
                    self.deadline = None;
                    return Ok(());
                }
                Err(error) => {
                    if self.closing || self.receiver.is_closed() {
                        return Err(error);
                    }
                    log::warn!(
                        "Event batch commit failed; retaining batch and retrying in 5s: {} bytes={} error_kind={:?}",
                        self.context, self.pending.len(), error.kind()
                    );
                    // Do not consume more events while this batch is failing.
                    // The bounded channel applies backpressure, not unbounded
                    // RAM growth or silent discard. Shutdown makes one final
                    // attempt instead of leaving a retrying writer behind.
                    tokio::select! {
                        _ = &mut self.stopped => {
                            self.closing = true;
                            self.receiver.close();
                        }
                        _ = sleep(BATCH_DELAY) => {}
                    }
                }
            }
        }
    }
}
