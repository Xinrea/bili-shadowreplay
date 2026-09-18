pub mod api;
pub mod profile;
pub mod response;
pub mod stream_info;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::{atomic, Arc};

use async_trait::async_trait;
use chrono::Utc;
use danmu_stream::provider::ProviderType;
use tokio::sync::{broadcast, RwLock};

use crate::account::Account;
use crate::core::{Codec, Format, HlsStream};
use crate::errors::RecorderError;
use crate::platforms::bilibili::api::{BiliStream, Protocol, Qn, UserInfoCache};
use crate::platforms::common::{
    download_file, DanmuConfig, DanmuSpawn, PlatformApi, RoomPoll, StreamPull,
};
use crate::platforms::PlatformType;
use crate::traits::RecorderTrait;
use crate::{Recorder, UserInfo};

/// A recorder for `BiliBili` live streams
///
/// This recorder fetches, caches and serves TS entries, currently supporting only `StreamType::FMP4`.
/// As high-quality streams are accessible only to logged-in users, the use of a `BiliClient`, which manages cookies, is required.
pub type BiliRecorder = Recorder<BiliExtra>;

#[derive(Clone)]
pub struct BiliExtra {
    live_stream: Arc<RwLock<Option<BiliStream>>>,
    user_info_cache: Arc<dyn UserInfoCache>,
}

impl BiliRecorder {
    pub fn new(
        room_id: &str,
        account: &Account,
        cache_dir: PathBuf,
        event_channel: broadcast::Sender<crate::events::RecorderEvent>,
        update_interval: Arc<AtomicU64>,
        enabled: bool,
        user_info_cache: Arc<dyn UserInfoCache>,
    ) -> Result<Self, RecorderError> {
        let recorder = Self::with_extra(
            PlatformType::BiliBili,
            room_id,
            account,
            cache_dir,
            event_channel,
            update_interval,
            enabled,
            BiliExtra {
                live_stream: Arc::new(RwLock::new(None)),
                user_info_cache,
            },
        );

        log::info!("[{room_id}]Recorder for room {room_id} created.");

        Ok(recorder)
    }

    fn log_error(&self, message: &str) {
        log::error!("[{}]{}", self.room_id, message);
    }
}

#[async_trait]
impl PlatformApi for BiliRecorder {
    async fn poll_room(&self) -> Result<RoomPoll, RecorderError> {
        let room_info = api::get_room_info(&self.client, &self.account, &self.room_id).await?;
        let live_status = room_info.live_status == 1;

        // Only update user info once
        let user = if self.user_info.read().await.user_id != room_info.user_id {
            match api::get_user_info_cached(
                &self.client,
                &self.account,
                &room_info.user_id,
                self.extra.user_info_cache.as_ref(),
            )
            .await
            {
                Ok(user_info) => Some(UserInfo {
                    user_id: room_info.user_id.to_string(),
                    user_name: user_info.user_name,
                    user_avatar: user_info.user_avatar,
                }),
                Err(e) => {
                    self.log_error(&format!("Failed to get user info: {e}"));
                    None
                }
            }
        } else {
            None
        };

        Ok(RoomPoll {
            live: live_status,
            room_title: room_info.room_title,
            room_cover: room_info.room_cover_url,
            user,
            platform_live_id: live_status.then(|| room_info.live_start_time.to_string()),
        })
    }

    async fn on_live_start(&self) {
        // Cache the room cover, the recording attempt copies it per session.
        let room_cover_path = Path::new(PlatformType::BiliBili.as_str())
            .join(&self.room_id)
            .join("cover.jpg");
        let full_room_cover_path = self.cache_dir.join(&room_cover_path);
        let room_cover_url = self.room_info.read().await.room_cover.clone();
        let _ = download_file(&self.client, &room_cover_url, &full_room_cover_path).await;
    }

    async fn poll_stream(&self) -> bool {
        let new_stream = api::get_stream_info(
            &self.client,
            &self.account,
            &self.room_id,
            Protocol::HttpHls,
            Format::TS,
            &[Codec::Avc, Codec::Hevc],
            Qn::Q25000,
        )
        .await;

        match new_stream {
            Ok(stream) => {
                let pre_live_stream = self.extra.live_stream.read().await.clone();
                *self.extra.live_stream.write().await = Some(stream.clone());
                self.last_update
                    .store(Utc::now().timestamp(), atomic::Ordering::Relaxed);

                log::info!(
                    "[{}]Update to a new stream: {:#?} => {:#?}",
                    self.room_id,
                    pre_live_stream,
                    stream
                );

                true
            }
            Err(e) => {
                if let RecorderError::FormatNotFound { format } = e {
                    log::error!("[{}]Format {} not found", self.room_id, format);
                } else {
                    log::error!("[{}]Fetch stream failed: {}", self.room_id, e);
                }

                // Keep recording with the cached stream until it expires.
                true
            }
        }
    }

    async fn open_pull(&self, live_id: &str) -> Result<StreamPull, RecorderError> {
        let Some(current_stream) = self.extra.live_stream.read().await.clone() else {
            return Err(RecorderError::NoStreamAvailable);
        };
        let Some(first_url_info) = current_stream.url_info.first() else {
            return Err(RecorderError::NoStreamAvailable);
        };

        let stream = Arc::new(HlsStream::new(
            live_id.to_string(),
            first_url_info.host.clone(),
            current_stream.base_url.clone(),
            first_url_info.extra.clone(),
            current_stream.format,
            current_stream.codec,
            first_url_info.get_expire(),
        ));

        Ok(StreamPull::Hls {
            stream,
            cookies: None,
        })
    }

    async fn clear_stream(&self) {
        *self.extra.live_stream.write().await = None;
    }

    async fn prepare_cover(&self, work_dir: &crate::CachePath) -> Result<(), RecorderError> {
        let cover_path = work_dir.with_filename("cover.jpg");
        let room_cover_path = self
            .cache_dir
            .join(PlatformType::BiliBili.as_str())
            .join(&self.room_id)
            .join("cover.jpg");

        tokio::fs::copy(room_cover_path, &cover_path.full_path())
            .await
            .map_err(RecorderError::IoError)?;

        Ok(())
    }

    fn danmu_config(&self) -> Option<DanmuConfig> {
        Some(DanmuConfig {
            provider: ProviderType::BiliBili,
            spawn: DanmuSpawn::LongLived,
        })
    }
}

#[async_trait]
impl RecorderTrait for BiliRecorder {
    async fn run(&self) {
        self.run_recording_loop().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::danmu::DanmuStorage;
    use crate::events::RecorderEvent;
    use std::sync::atomic::Ordering;

    struct UnusedUserInfoCache;

    #[async_trait]
    impl UserInfoCache for UnusedUserInfoCache {
        async fn get_user_info(&self, _: &str) -> Result<Option<UserInfo>, String> {
            panic!("lifecycle tests must not request user info")
        }

        async fn save_user_info(&self, _: &UserInfo) -> Result<(), String> {
            panic!("lifecycle tests must not save user info")
        }
    }

    async fn recording_fixture() -> (BiliRecorder, broadcast::Receiver<RecorderEvent>) {
        let (tx, rx) = broadcast::channel(10);
        let recorder = BiliRecorder::new(
            "10220184",
            &Account::default(),
            std::env::temp_dir().join(format!("bsr-lifecycle-{}", uuid::Uuid::new_v4())),
            tx,
            Arc::new(AtomicU64::new(30)),
            true,
            Arc::new(UnusedUserInfoCache),
        )
        .unwrap();
        *recorder.platform_live_id.write().await = "session-1".into();
        *recorder.live_id.write().await = "segment-1".into();
        *recorder.pre_live_id.write().await = Some("segment-1".into());
        recorder.should_continue.store(true, Ordering::Relaxed);
        recorder.is_recording.store(true, Ordering::Relaxed);
        *recorder.extra.live_stream.write().await = Some(BiliStream::new(
            Format::TS,
            Codec::Avc,
            "/expired.m3u8",
            vec![],
            false,
            None,
        ));
        (recorder, rx)
    }

    #[tokio::test]
    async fn recording_reset_preserves_session_and_expiry_resume_state() {
        let (recorder, _) = recording_fixture().await;
        tokio::fs::create_dir_all(&recorder.cache_dir)
            .await
            .unwrap();
        *recorder.danmu_storage.write().await =
            DanmuStorage::new(&recorder.cache_dir.join("events.jsonl")).await;
        assert!(recorder.danmu_storage.read().await.is_some());

        recorder.reset_recording().await;

        let info = recorder.info().await;
        assert_eq!(info.platform_live_id, "session-1");
        assert!(info.live_id.is_empty());
        assert!(!info.recording);
        assert!(recorder.extra.live_stream.read().await.is_none());
        assert!(recorder.danmu_storage.read().await.is_none());
        assert_eq!(
            recorder.pre_live_id.read().await.as_deref(),
            Some("segment-1")
        );
        assert!(recorder.should_continue.load(Ordering::Relaxed));
        tokio::fs::remove_dir_all(&recorder.cache_dir)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn live_end_after_recording_error_keeps_session_snapshot_and_clears_resume() {
        let (recorder, mut rx) = recording_fixture().await;
        recorder.reset_recording().await;

        // A failed restart must not expose the previous segment as the current recording.
        assert!(matches!(
            recorder.start_recording("failed-segment").await,
            Err(RecorderError::NoStreamAvailable)
        ));
        assert!(recorder.info().await.live_id.is_empty());

        recorder.end_live().await;

        let RecorderEvent::LiveEnd {
            recorder: ended, ..
        } = rx.try_recv().unwrap()
        else {
            panic!("expected a live end event")
        };
        assert_eq!(ended.platform_live_id, "session-1");
        assert!(ended.live_id.is_empty());
        assert!(!ended.recording);
        assert!(recorder.info().await.platform_live_id.is_empty());
        assert!(recorder.pre_live_id.read().await.is_none());
        assert!(!recorder.should_continue.load(Ordering::Relaxed));
    }

    #[test]
    fn parse_fmp4_playlist() {
        let content = r#"#EXTM3U
        #EXT-X-VERSION:7
        #EXT-X-START:TIME-OFFSET=0
        #EXT-X-MEDIA-SEQUENCE:323066244
        #EXT-X-TARGETDURATION:1
        #EXT-X-MAP:URI=\"h1758715459.m4s\"
        #EXT-BILI-AUX:97d350|K|7d1e3|fe1425ab
        #EXTINF:1.00,7d1e3|fe1425ab
        323066244.m4s
        #EXT-BILI-AUX:97d706|N|757d4|c9094969
        #EXTINF:1.00,757d4|c9094969
        323066245.m4s
        #EXT-BILI-AUX:97daee|N|8223d|f307566a
        #EXTINF:1.00,8223d|f307566a
        323066246.m4s
        #EXT-BILI-AUX:97dee7|N|775cc|428d567
        #EXTINF:1.00,775cc|428d567
        323066247.m4s
        #EXT-BILI-AUX:97e2df|N|10410|9a62fe61
        #EXTINF:0.17,10410|9a62fe61
        323066248.m4s
        #EXT-BILI-AUX:97e397|K|679d2|8fbee7df
        #EXTINF:1.00,679d2|8fbee7df
        323066249.m4s
        #EXT-BILI-AUX:97e74d|N|8907b|67d1c6ad
        #EXTINF:1.00,8907b|67d1c6ad
        323066250.m4s
        #EXT-BILI-AUX:97eb35|N|87374|f6406797
        #EXTINF:1.00,87374|f6406797
        323066251.m4s
        #EXT-BILI-AUX:97ef2d|N|6b792|b8125097
        #EXTINF:1.00,6b792|b8125097
        323066252.m4s
        #EXT-BILI-AUX:97f326|N|e213|b30c02c6
        #EXTINF:0.17,e213|b30c02c6
        323066253.m4s
        #EXT-BILI-AUX:97f3de|K|65754|7ea6dcc8
        #EXTINF:1.00,65754|7ea6dcc8
        323066254.m4s
        "#;
        let (_, pl) = m3u8_rs::parse_media_playlist(content.as_bytes()).unwrap();
        // ExtTag { tag: "X-MAP", rest: Some("URI=\\\"h1758715459.m4s\\\"") }
        let header_url = pl
            .segments
            .first()
            .unwrap()
            .unknown_tags
            .iter()
            .find(|t| t.tag == "X-MAP")
            .map(|t| {
                let rest = t.rest.clone().unwrap();
                rest.split('=').nth(1).unwrap().replace("\\\"", "")
            });
        // #EXT-BILI-AUX:a5e4e0|K|79b3e|ebde469e
        let is_key = pl
            .segments
            .first()
            .unwrap()
            .unknown_tags
            .iter()
            .find(|t| t.tag == "BILI-AUX")
            .map(|t| {
                let rest = t.rest.clone().unwrap();
                rest.split('|').nth(1).unwrap() == "K"
            });
        assert_eq!(is_key, Some(true));
        assert_eq!(header_url, Some("h1758715459.m4s".to_string()));
    }
}
