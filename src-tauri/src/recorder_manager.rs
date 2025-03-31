use crate::config::Config;
use crate::database::DatabaseError;
use crate::database::{account::AccountRow, record::RecordRow, Database};
use crate::progress_event::ProgressReporter;
use crate::recorder::bilibili::BiliRecorder;
use crate::recorder::danmu::DanmuEntry;
use crate::recorder::douyin::DouyinRecorder;
use crate::recorder::errors::RecorderError;
use crate::recorder::PlatformType;
use crate::recorder::Recorder;
use crate::recorder::RecorderInfo;
use custom_error::custom_error;
use hyper::Method;
use hyper::{
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::{convert::Infallible, sync::Arc};
use tauri::AppHandle;
use tokio::{net::TcpListener, sync::RwLock};

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct RecorderList {
    pub count: usize,
    pub recorders: Vec<RecorderInfo>,
}

#[derive(Debug, Deserialize)]
pub struct ClipRangeParams {
    pub title: String,
    pub cover: String,
    pub platform: String,
    pub room_id: u64,
    pub live_id: String,
    pub x: f64,
    pub y: f64,
}

pub struct RecorderManager {
    app_handle: AppHandle,
    db: Arc<Database>,
    config: Arc<RwLock<Config>>,
    recorders: Arc<RwLock<HashMap<String, Box<dyn Recorder>>>>,
    hls_server_addr: Arc<RwLock<Option<SocketAddr>>>,
}

custom_error! {pub RecorderManagerError
    AlreadyExisted { room_id: u64 } = "房间 {room_id} 已存在",
    NotFound {room_id: u64 } = "房间 {room_id} 不存在",
    InvalidPlatformType { platform: String } = "不支持的平台: {platform}",
    RecorderError { err: RecorderError } = "录播器错误: {err}",
    IOError {err: std::io::Error } = "IO 错误: {err}",
    HLSError { err: hyper::Error } = "HLS 服务器错误: {err}",
    DatabaseError { err: DatabaseError } = "数据库错误: {err}",
    Recording { live_id: String } = "无法删除正在录制的直播 {live_id}",
}

impl From<hyper::Error> for RecorderManagerError {
    fn from(value: hyper::Error) -> Self {
        RecorderManagerError::HLSError { err: value }
    }
}

impl From<std::io::Error> for RecorderManagerError {
    fn from(value: std::io::Error) -> Self {
        RecorderManagerError::IOError { err: value }
    }
}

impl From<RecorderError> for RecorderManagerError {
    fn from(value: RecorderError) -> Self {
        RecorderManagerError::RecorderError { err: value }
    }
}

impl From<DatabaseError> for RecorderManagerError {
    fn from(value: DatabaseError) -> Self {
        RecorderManagerError::DatabaseError { err: value }
    }
}

impl From<RecorderManagerError> for String {
    fn from(value: RecorderManagerError) -> Self {
        value.to_string()
    }
}

impl RecorderManager {
    pub fn new(
        app_handle: AppHandle,
        db: Arc<Database>,
        config: Arc<RwLock<Config>>,
    ) -> RecorderManager {
        RecorderManager {
            app_handle,
            db,
            config,
            recorders: Arc::new(RwLock::new(HashMap::new())),
            hls_server_addr: Arc::new(RwLock::new(None)),
        }
    }

    /// starting HLS server
    pub async fn run_hls(&self) -> Result<(), RecorderManagerError> {
        let addr = SocketAddr::from(([127, 0, 0, 1], 0));
        let listener = TcpListener::bind(addr).await?;
        let addr = self.start_hls_server(listener).await?;
        *self.hls_server_addr.write().await = Some(addr);
        Ok(())
    }

    pub async fn add_recorder(
        &self,
        webid: &str,
        account: &AccountRow,
        platform: PlatformType,
        room_id: u64,
        auto_start: bool,
    ) -> Result<(), RecorderManagerError> {
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if self.recorders.read().await.contains_key(&recorder_id) {
            return Err(RecorderManagerError::AlreadyExisted { room_id });
        }

        let recorder: Box<dyn Recorder + 'static> = match platform {
            PlatformType::BiliBili => Box::new(
                BiliRecorder::new(
                    self.app_handle.clone(),
                    webid,
                    &self.db,
                    room_id,
                    account,
                    self.config.clone(),
                    auto_start,
                )
                .await?,
            ),
            PlatformType::Douyin => Box::new(
                DouyinRecorder::new(
                    self.app_handle.clone(),
                    room_id,
                    self.config.clone(),
                    account,
                    &self.db,
                    auto_start,
                )
                .await?,
            ),
            _ => {
                return Err(RecorderManagerError::InvalidPlatformType {
                    platform: platform.as_str().to_string(),
                })
            }
        };
        self.recorders
            .write()
            .await
            .insert(recorder_id.clone(), recorder);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            recorder_ref.run().await;
        }
        Ok(())
    }

    pub async fn stop_all(&self) {
        for recorder_ref in self.recorders.read().await.values() {
            recorder_ref.stop().await;
        }

        // remove all recorders
        self.recorders.write().await.clear();
    }

    pub async fn remove_recorder(
        &self,
        platform: PlatformType,
        room_id: u64,
    ) -> Result<(), RecorderManagerError> {
        // check recorder exists
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if !self.recorders.read().await.contains_key(&recorder_id) {
            return Err(RecorderManagerError::NotFound { room_id });
        }

        // stop recorder
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            recorder_ref.stop().await;
        }

        // remove recorder
        self.recorders.write().await.remove(&recorder_id);

        // remove related cache folder
        let cache_folder = format!(
            "{}/{}/{}",
            self.config.read().await.cache,
            platform.as_str(),
            room_id
        );
        let _ = tokio::fs::remove_dir_all(cache_folder).await;
        log::info!("Recorder {} cache folder removed", room_id);

        Ok(())
    }

    pub async fn clip_range(
        &self,
        reporter: &ProgressReporter,
        clip_file: PathBuf,
        params: &ClipRangeParams,
    ) -> Result<PathBuf, RecorderManagerError> {
        let recorders = self.recorders.read().await;
        let recorder_id = format!("{}:{}", params.platform, params.room_id);
        if !recorders.contains_key(&recorder_id) {
            log::error!("Recorder {} not found", recorder_id);
            return Err(RecorderManagerError::NotFound {
                room_id: params.room_id,
            });
        }
        let recorder = recorders.get(&recorder_id).unwrap();
        Ok(recorder
            .clip_range(reporter, &params.live_id, params.x, params.y, clip_file)
            .await?)
    }

    pub async fn get_recorder_list(&self) -> RecorderList {
        let mut summary = RecorderList {
            count: self.recorders.read().await.len(),
            recorders: Vec::new(),
        };

        for recorder_ref in self.recorders.read().await.iter() {
            let room_info = recorder_ref.1.info().await;
            summary.recorders.push(room_info);
        }

        summary.recorders.sort_by(|a, b| a.room_id.cmp(&b.room_id));
        summary
    }

    pub async fn get_recorder_info(
        &self,
        platform: PlatformType,
        room_id: u64,
    ) -> Option<RecorderInfo> {
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            let room_info = recorder_ref.info().await;
            Some(room_info)
        } else {
            None
        }
    }

    pub async fn get_archives(&self, room_id: u64) -> Result<Vec<RecordRow>, RecorderManagerError> {
        Ok(self.db.get_records(room_id).await?)
    }

    pub async fn get_archive(
        &self,
        room_id: u64,
        live_id: &str,
    ) -> Result<RecordRow, RecorderManagerError> {
        Ok(self.db.get_record(room_id, live_id).await?)
    }

    pub async fn delete_archive(
        &self,
        platform: PlatformType,
        room_id: u64,
        live_id: &str,
    ) -> Result<(), RecorderManagerError> {
        log::info!("Deleting {}:{}", room_id, live_id);
        // check if this is still recording
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            let recorder = recorder_ref.as_ref();
            if recorder.is_recording(live_id).await {
                return Err(RecorderManagerError::Recording {
                    live_id: live_id.to_string(),
                });
            }
        }
        Ok(self.db.remove_record(live_id).await?)
    }

    pub async fn get_danmu(
        &self,
        platform: PlatformType,
        room_id: u64,
        live_id: &str,
    ) -> Result<Vec<DanmuEntry>, RecorderManagerError> {
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            Ok(recorder_ref.comments(live_id).await?)
        } else {
            Err(RecorderManagerError::NotFound { room_id })
        }
    }

    async fn start_hls_server(
        &self,
        listener: TcpListener,
    ) -> Result<SocketAddr, RecorderManagerError> {
        let recorders = self.recorders.clone();
        let config = self.config.clone();
        let make_svc = make_service_fn(move |_conn| {
            let recorders = recorders.clone();
            let config = config.clone();
            async move {
                Ok::<_, Infallible>(service_fn(move |req: Request<Body>| {
                    let recorders = recorders.clone();
                    let config = config.clone();
                    async move {
                        // handle cors preflight request
                        if req.method() == Method::OPTIONS {
                            return Ok::<_, Infallible>(
                                Response::builder()
                                    .status(200)
                                    .header("Access-Control-Allow-Origin", "*")
                                    .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
                                    .header("Access-Control-Allow-Headers", "Content-Type")
                                    .body(Body::empty())
                                    .unwrap(),
                            );
                        }

                        let cache_path = config.read().await.cache.clone();
                        let path = req.uri().path();
                        let path_segs: Vec<&str> = path.split('/').collect();

                        // path_segs should be size 5: /{platform}/{room_id}/{live_id}/playlist.m3u8
                        if path_segs.len() != 5 {
                            log::warn!("Invalid request path: {}", path);
                            return Ok::<_, Infallible>(
                                Response::builder()
                                    .status(400)
                                    .body(Body::from("Request Path Not Found"))
                                    .unwrap(),
                            );
                        }
                        // parse recorder type
                        let platform = path_segs[1];
                        // parse room id
                        let room_id = path_segs[2].parse::<u64>().unwrap();
                        // parse live id
                        let live_id = path_segs[3];

                        if path_segs[4] == "playlist.m3u8" {
                            // get recorder
                            let recorder_key = format!("{}:{}", platform, room_id);
                            let recorders = recorders.read().await;
                            let recorder = recorders.get(&recorder_key);
                            if recorder.is_none() {
                                return Ok::<_, Infallible>(
                                    Response::builder()
                                        .status(404)
                                        .body(Body::from("Recorder Not Found"))
                                        .unwrap(),
                                );
                            }
                            let recorder = recorder.unwrap();
                            // response with recorder generated m3u8, which contains ts entries that cached in local
                            let m3u8_content = recorder.m3u8_content(live_id).await;
                            Ok::<_, Infallible>(
                                Response::builder()
                                    .status(200)
                                    .header("Content-Type", "application/vnd.apple.mpegurl")
                                    .header("Access-Control-Allow-Origin", "*")
                                    .header("Access-Control-Allow-Methods", "GET, OPTIONS")
                                    .body(Body::from(m3u8_content))
                                    .unwrap(),
                            )
                        } else {
                            // try to find requested ts file in recorder's cache
                            // cache files are stored in {cache_dir}/{room_id}/{timestamp}/{ts_file}
                            let ts_file = format!("{}/{}", cache_path, path.replace("%7C", "|"));
                            let recorders = recorders.read().await;
                            let recorder_id = format!("{}:{}", platform, room_id);
                            let recorder = recorders.get(&recorder_id);
                            if recorder.is_none() {
                                return Ok::<_, Infallible>(
                                    Response::builder()
                                        .status(404)
                                        .body(Body::from("Recorder Not Found"))
                                        .unwrap(),
                                );
                            }
                            let ts_file_content = tokio::fs::read(ts_file).await;
                            if ts_file_content.is_err() {
                                return Ok::<_, Infallible>(
                                    Response::builder()
                                        .status(404)
                                        .body(Body::from("TS File Not Found"))
                                        .unwrap(),
                                );
                            }
                            let ts_file_content = ts_file_content.unwrap();
                            Ok::<_, Infallible>(
                                Response::builder()
                                    .status(200)
                                    .header("Content-Type", "video/MP2T")
                                    .header("Access-Control-Allow-Origin", "*")
                                    .header("Access-Control-Allow-Methods", "GET, OPTIONS")
                                    .body(Body::from(ts_file_content))
                                    .unwrap(),
                            )
                        }
                    }
                }))
            }
        });

        let server = Server::from_tcp(listener.into_std().unwrap())?.serve(make_svc);
        let addr = server.local_addr();
        tokio::spawn(async move {
            if let Err(e) = server.await {
                log::error!("HLS server error: {}", e);
            }
        });

        Ok(addr)
    }

    pub async fn get_hls_server_addr(&self) -> Option<SocketAddr> {
        *self.hls_server_addr.read().await
    }

    pub async fn set_auto_start(&self, platform: PlatformType, room_id: u64, auto_start: bool) {
        // update RecordRow auto_start field
        if let Err(e) = self.db.update_recorder(platform, room_id, auto_start).await {
            log::error!("Failed to update recorder auto_start: {}", e);
        }

        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            recorder_ref.set_auto_start(auto_start).await;
        }
    }

    pub async fn force_start(&self, platform: PlatformType, room_id: u64) {
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            recorder_ref.force_start().await;
        }
    }

    pub async fn force_stop(&self, platform: PlatformType, room_id: u64) {
        let recorder_id = format!("{}:{}", platform.as_str(), room_id);
        if let Some(recorder_ref) = self.recorders.read().await.get(&recorder_id) {
            recorder_ref.force_stop().await;
        }
    }
}
