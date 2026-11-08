//! An owned RTMP ingest listener. Valid publishers feed the existing native FLV
//! recorder over a loopback socket, so HLS, archive events and replay stay shared.
mod config;
mod session;
pub use config::{normalize_room_id, RtmpConfig};

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use async_trait::async_trait;
use tokio::{
    net::TcpListener,
    sync::{watch, Mutex},
    task::JoinSet,
};

use crate::{
    account::Account, errors::RecorderError, platforms::PlatformType, traits::RecorderTrait,
    Recorder, RoomInfo, UserInfo,
};

#[derive(Clone)]
pub struct RtmpExtra {
    config: RtmpConfig,
    listener: Arc<Mutex<Option<TcpListener>>>,
    publisher: Arc<Mutex<()>>,
    control: watch::Sender<u64>,
}

pub type RtmpRecorder = Recorder<RtmpExtra>;

impl RtmpRecorder {
    pub async fn new(
        room_id: &str,
        mut config: RtmpConfig,
        storage: impl Into<recording_storage::RecordingStore>,
        event_channel: impl Into<crate::events::RecorderEventSender>,
        enabled: bool,
    ) -> Result<Self, RecorderError> {
        let room_id =
            normalize_room_id(room_id).map_err(|error| RecorderError::ApiError { error })?;
        // Reserve the port atomically, then persist the actual address. Do not
        // probe and release a random port before starting the listener.
        let listener = TcpListener::bind(config.socket_addr())
            .await
            .map_err(|error| RecorderError::ApiError {
                error: format!("无法监听 RTMP {}: {error}", config.socket_addr()),
            })?;
        config.port = listener
            .local_addr()
            .map_err(|error| RecorderError::ApiError {
                error: format!("无法获取 RTMP 监听端口: {error}"),
            })?
            .port();
        let (control, _) = watch::channel(0);
        let name = config.name.clone();
        let recorder = Self::with_extra(
            PlatformType::Rtmp,
            &room_id,
            &Account::default(),
            storage,
            event_channel,
            Arc::new(AtomicU64::new(0)),
            enabled,
            RtmpExtra {
                config,
                listener: Arc::new(Mutex::new(Some(listener))),
                publisher: Arc::new(Mutex::new(())),
                control,
            },
        );
        *recorder.room_info.write().await = RoomInfo {
            platform: "rtmp".into(),
            room_id: room_id.clone(),
            room_title: name.clone(),
            room_cover: String::new(),
            status: false,
        };
        *recorder.user_info.write().await = UserInfo {
            user_id: room_id,
            user_name: name,
            user_avatar: String::new(),
        };
        Ok(recorder)
    }

    pub fn config(&self) -> &RtmpConfig {
        &self.extra.config
    }

    async fn listen(&self, listener: TcpListener, mut control: watch::Receiver<u64>) {
        let mut clients = JoinSet::new();
        while !self.quit.load(Ordering::Acquire) {
            tokio::select! {
                biased;
                _ = control.changed() => {},
                Some(result) = clients.join_next(), if !clients.is_empty() => {
                    if let Err(error) = result {
                        log::error!("RTMP connection task failed: {error}");
                    }
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((socket, _)) if clients.len() < 8 => {
                            let recorder = self.clone();
                            let control = self.extra.control.subscribe();
                            clients.spawn(async move { session::receive(recorder, socket, control).await; });
                        }
                        Ok(_) => {},
                        Err(error) => {
                            log::warn!("RTMP accept failed: {error}");
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                    }
                }
            }
        }
        // No new clients; wait for native muxers to flush before releasing the port.
        while clients.join_next().await.is_some() {}
    }
}

#[async_trait]
impl RecorderTrait for RtmpRecorder {
    async fn run(&self) {
        let mut task = self.record_task.lock().await;
        if self.quit.load(Ordering::Acquire) || task.is_some() {
            return;
        }
        if let Some(listener) = self.extra.listener.lock().await.take() {
            let recorder = self.clone();
            let control = self.extra.control.subscribe();
            *task = Some(tokio::spawn(async move {
                recorder.listen(listener, control).await;
            }));
        }
    }

    async fn stop(&self) {
        self.quit.store(true, Ordering::Release);
        self.extra
            .control
            .send_modify(|generation| *generation += 1);
        let mut task = self.record_task.lock().await;
        if let Some(task) = task.take() {
            let _ = task.await;
        }
        self.extra.listener.lock().await.take();
    }

    async fn disable(&self) {
        self.enabled.store(false, Ordering::Release);
        // Disconnect the publisher and finalize its archive. Keep the listener
        // reserved; enabling the room permits OBS to reconnect to the same URL.
        self.extra
            .control
            .send_modify(|generation| *generation += 1);
    }
}

#[cfg(test)]
mod tests;
