use crate::db::{AccountRow, Database, RecordRow};
use crate::recorder::bilibili::UserInfo;
use crate::recorder::danmu::DanmuEntry;
use crate::recorder::RecorderError;
use crate::recorder::{bilibili::RoomInfo, BiliRecorder};
use crate::Config;
use custom_error::custom_error;
use dashmap::DashMap;
use hyper::Method;
use hyper::{
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server,
};
use std::net::SocketAddr;
use std::{convert::Infallible, sync::Arc};
use tauri::AppHandle;
use tokio::{net::TcpListener, sync::RwLock};

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct RecorderList {
    pub count: usize,
    pub recorders: Vec<RecorderInfo>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct RecorderInfo {
    pub room_id: u64,
    pub room_info: RoomInfo,
    pub user_info: UserInfo,
    pub total_length: f64,
    pub current_ts: u64,
    pub live_status: bool,
}

pub struct RecorderManager {
    app_handle: AppHandle,
    config: Arc<RwLock<Config>>,
    recorders: Arc<DashMap<u64, BiliRecorder>>,
    hls_server_addr: Arc<RwLock<Option<SocketAddr>>>,
}

custom_error! {pub RecorderManagerError
    AlreadyExisted { room_id: u64 } = "Recorder {room_id} already existed",
    NotFound {room_id: u64 } = "Recorder {room_id} not found",
    RecorderError { err: RecorderError } = "Recorder error",
    IOError {err: std::io::Error } = "IO error",
    HLSError { err: hyper::Error } = "HLS server error",
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

impl From<RecorderManagerError> for String {
    fn from(value: RecorderManagerError) -> Self {
        value.to_string()
    }
}

impl RecorderManager {
    pub fn new(app_handle: AppHandle, config: Arc<RwLock<Config>>) -> RecorderManager {
        RecorderManager {
            app_handle,
            config,
            recorders: Arc::new(DashMap::new()),
            hls_server_addr: Arc::new(RwLock::new(None)),
        }
    }

    /// starting HLS server
    pub async fn run_hls(&self) -> Result<(), RecorderManagerError> {
        let addr = SocketAddr::from(([127, 0, 0, 1], 0));
        let listener = TcpListener::bind(&addr).await?;
        let server_addr = self.start_hls_server(listener).await?;
        log::info!("HLS server started on {}", server_addr);
        self.hls_server_addr.write().await.replace(server_addr);
        Ok(())
    }

    pub async fn add_recorder(
        &self,
        webid: &str,
        db: &Arc<Database>,
        account: &AccountRow,
        room_id: u64,
    ) -> Result<(), RecorderManagerError> {
        // check existing recorder
        if self.recorders.contains_key(&room_id) {
            return Err(RecorderManagerError::AlreadyExisted { room_id });
        }
        let recorder = BiliRecorder::new(
            self.app_handle.clone(),
            webid,
            db,
            room_id,
            account,
            self.config.clone(),
        )
        .await?;
        self.recorders.insert(room_id, recorder);
        // run recorder
        let recorder = self.recorders.get(&room_id).unwrap();
        recorder.value().run().await;
        Ok(())
    }

    pub async fn remove_recorder(&self, room_id: u64) -> Result<(), RecorderManagerError> {
        let recorder = self.recorders.remove(&room_id);
        if recorder.is_none() {
            return Err(RecorderManagerError::NotFound { room_id });
        }
        recorder.unwrap().1.stop().await;
        // remove related cache folder
        let cache_folder = format!("{}/{}", self.config.read().await.cache, room_id);
        tokio::fs::remove_dir_all(cache_folder).await?;
        Ok(())
    }

    pub async fn clip(
        &self,
        output_path: &str,
        room_id: u64,
        d: f64,
    ) -> Result<String, RecorderManagerError> {
        let recorder = self.recorders.get(&room_id);
        if recorder.is_none() {
            return Err(RecorderManagerError::NotFound { room_id });
        }
        let recorder = recorder.unwrap();
        Ok(recorder.value().clip(room_id, d, output_path).await?)
    }

    pub async fn clip_range(
        &self,
        output_path: &str,
        room_id: u64,
        ts: u64,
        start: f64,
        end: f64,
    ) -> Result<String, RecorderManagerError> {
        let recorder = self.recorders.get(&room_id);
        if recorder.is_none() {
            return Err(RecorderManagerError::NotFound { room_id });
        }
        let recorder = recorder.unwrap();
        Ok(recorder
            .value()
            .clip_range(ts, start, end, output_path)
            .await?)
    }

    pub async fn get_recorder_list(&self) -> RecorderList {
        let mut summary = RecorderList {
            count: self.recorders.len(),
            recorders: Vec::new(),
        };

        for recorder in self.recorders.iter() {
            let recorder = recorder.value();
            let room_info = RecorderInfo {
                room_id: recorder.room_id,
                room_info: recorder.room_info.read().await.clone(),
                user_info: recorder.user_info.read().await.clone(),
                total_length: *recorder.ts_length.read().await,
                current_ts: *recorder.timestamp.read().await,
                live_status: *recorder.live_status.read().await,
            };
            summary.recorders.push(room_info);
        }

        summary.recorders.sort_by(|a, b| a.room_id.cmp(&b.room_id));

        summary
    }

    pub async fn get_recorder_info(&self, room_id: u64) -> Option<RecorderInfo> {
        if let Some(recorder) = self.recorders.get(&room_id) {
            let room_info = RecorderInfo {
                room_id: recorder.room_id,
                room_info: recorder.room_info.read().await.clone(),
                user_info: recorder.user_info.read().await.clone(),
                total_length: *recorder.ts_length.read().await,
                current_ts: *recorder.timestamp.read().await,
                live_status: *recorder.live_status.read().await,
            };
            Some(room_info)
        } else {
            None
        }
    }

    pub async fn get_archives(&self, room_id: u64) -> Result<Vec<RecordRow>, RecorderManagerError> {
        if let Some(recorder) = self.recorders.get(&room_id) {
            Ok(recorder.get_archives().await?)
        } else {
            Err(RecorderManagerError::NotFound { room_id })
        }
    }

    pub async fn get_archive(
        &self,
        room_id: u64,
        live_id: u64,
    ) -> Result<RecordRow, RecorderManagerError> {
        if let Some(recorder) = self.recorders.get(&room_id) {
            Ok(recorder.get_archive(live_id).await?)
        } else {
            Err(RecorderManagerError::NotFound { room_id })
        }
    }

    pub async fn delete_archive(&self, room_id: u64, ts: u64) -> Result<(), RecorderManagerError> {
        if let Some(recorder) = self.recorders.get(&room_id) {
            recorder.delete_archive(ts).await?
        }
        Err(RecorderManagerError::NotFound { room_id })
    }

    pub async fn get_danmu(
        &self,
        room_id: u64,
        live_id: u64,
    ) -> Result<Vec<DanmuEntry>, RecorderManagerError> {
        if let Some(recorder) = self.recorders.get(&room_id) {
            Ok(recorder.get_danmu_record(live_id).await)
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
                        // path_segs should be size 4: /21484828/{timestamp}/playlist.m3u8
                        if path_segs.len() != 4 {
                            return Ok::<_, Infallible>(
                                Response::builder()
                                    .status(400)
                                    .body(Body::from("Request Path Not Found"))
                                    .unwrap(),
                            );
                        }
                        // parse room id
                        let room_id = path_segs[1].parse::<u64>().unwrap();
                        let timestamp = path_segs[2].parse::<u64>().unwrap();
                        // if path is /room_id/{timestamp}/playlist.m3u8
                        if path_segs[3] == "playlist.m3u8" {
                            // get recorder
                            let recorder = recorders.get(&room_id);
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
                            let m3u8_content = recorder.value().generate_m3u8(timestamp).await;
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
                            let recorder = recorders.get(&room_id);
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
}
