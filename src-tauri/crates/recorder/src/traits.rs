use std::{
    path::PathBuf,
    sync::{atomic, Arc},
};

use crate::platforms::PlatformType;
use crate::{
    account::Account, danmu::DanmuStorage, events::RecorderEvent, CachePath, RecorderInfo,
    RoomInfo, UserInfo,
};
use async_trait::async_trait;
use tokio::{
    sync::{broadcast, Mutex, RwLock},
    task::JoinHandle,
};

#[allow(dead_code)]
pub trait RecorderBasicTrait<T> {
    fn platform(&self) -> PlatformType;
    fn room_id(&self) -> String;
    fn account(&self) -> &Account;
    fn client(&self) -> &reqwest::Client;
    fn event_channel(&self) -> &broadcast::Sender<RecorderEvent>;
    fn cache_dir(&self) -> PathBuf;
    fn quit(&self) -> &atomic::AtomicBool;
    fn enabled(&self) -> &atomic::AtomicBool;
    fn is_recording(&self) -> &atomic::AtomicBool;
    fn room_info(&self) -> Arc<RwLock<RoomInfo>>;
    fn user_info(&self) -> Arc<RwLock<UserInfo>>;
    fn platform_live_id(&self) -> Arc<RwLock<String>>;
    fn live_id(&self) -> Arc<RwLock<String>>;
    fn danmu_task(&self) -> Arc<Mutex<Option<JoinHandle<()>>>>;
    fn record_task(&self) -> Arc<Mutex<Option<JoinHandle<()>>>>;
    fn danmu_storage(&self) -> Arc<RwLock<Option<DanmuStorage>>>;
    fn last_update(&self) -> &atomic::AtomicI64;
    fn last_sequence(&self) -> &atomic::AtomicU64;
    fn total_duration(&self) -> &atomic::AtomicU64;
    fn total_size(&self) -> &atomic::AtomicU64;
    fn extra(&self) -> &T;
}

#[async_trait]
pub trait RecorderTrait<T>: RecorderBasicTrait<T> {
    async fn run(&self);
    async fn stop(&self) {
        self.quit().store(true, atomic::Ordering::Relaxed);
        if let Some(danmu_task) = self.danmu_task().lock().await.take() {
            danmu_task.abort();
            let _ = danmu_task.await;
        }
        if let Some(record_task) = self.record_task().lock().await.take() {
            record_task.abort();
            let _ = record_task.await;
        }
    }
    async fn should_record(&self) -> bool {
        if self.quit().load(atomic::Ordering::Relaxed) {
            return false;
        }

        self.enabled().load(atomic::Ordering::Relaxed)
    }

    async fn work_dir(&self, live_id: &str) -> CachePath {
        CachePath::new(self.cache_dir(), self.platform(), &self.room_id(), live_id)
    }
    async fn info(&self) -> RecorderInfo {
        let room_info = self.room_info().read().await.clone();
        let user_info = self.user_info().read().await.clone();
        let is_recording = self.is_recording().load(atomic::Ordering::Relaxed);
        RecorderInfo {
            platform_live_id: self.platform_live_id().read().await.clone(),
            live_id: self.live_id().read().await.clone(),
            recording: is_recording,
            enabled: self.enabled().load(atomic::Ordering::Relaxed),
            room_info: RoomInfo {
                platform: self.platform().as_str().to_string(),
                room_id: self.room_id().to_string(),
                room_title: room_info.room_title.clone(),
                room_cover: room_info.room_cover.clone(),
                status: room_info.status,
            },
            user_info: UserInfo {
                user_id: user_info.user_id.to_string(),
                user_name: user_info.user_name.clone(),
                user_avatar: user_info.user_avatar.clone(),
            },
        }
    }
    async fn enable(&self) {
        self.enabled().store(true, atomic::Ordering::Relaxed);
    }
    async fn disable(&self) {
        self.enabled().store(false, atomic::Ordering::Relaxed);
    }
}
