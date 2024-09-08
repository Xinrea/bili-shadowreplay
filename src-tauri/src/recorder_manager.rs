use crate::recorder::BiliRecorder;
use crate::Config;
use dashmap::DashMap;
use hyper::{
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server,
};
use std::net::SocketAddr;
use std::{convert::Infallible, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::RwLock,
};

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct Summary {
    pub count: usize,
    pub rooms: Vec<RoomInfo>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct RoomInfo {
    pub room_id: u64,
    pub room_title: String,
    pub room_cover: String,
    pub room_keyframe: String,
    pub user_id: u64,
    pub user_name: String,
    pub user_sign: String,
    pub user_avatar: String,
    pub total_length: f64,
    pub live_status: bool,
}

pub struct RecorderManager {
    config: Arc<RwLock<Config>>,
    recorders: Arc<DashMap<u64, BiliRecorder>>,
    hls_server_addr: Arc<RwLock<Option<SocketAddr>>>,
}

impl RecorderManager {
    pub fn new(config: Arc<RwLock<Config>>) -> RecorderManager {
        RecorderManager {
            config,
            recorders: Arc::new(DashMap::new()),
            hls_server_addr: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn init(&self) {
        let config = self.config.read().await.clone();
        for room_id in config.rooms.iter() {
            let recorder = BiliRecorder::new(*room_id, self.config.clone()).await.unwrap();
            // run recorder
            recorder.run().await;
            self.recorders.insert(*room_id, recorder);
        }
        println!("RecorderManager initialized");
    }

    pub async fn run(&self) {
        let addr = SocketAddr::from(([127, 0, 0, 1], 0));
        let listener = TcpListener::bind(&addr).await.unwrap();

        let server_addr = self.start_hls_server(listener).await.unwrap();
        println!("HLS server started on http://{}", server_addr);
        self.hls_server_addr.write().await.replace(server_addr);
    }

    pub async fn add_recorder(&self, room_id: u64) -> Result<(), String> {
        // check existing recorder
        if self.recorders.contains_key(&room_id) {
            return Err(format!("Recorder {} already exists", room_id));
        }
        let recorder = BiliRecorder::new(room_id, self.config.clone()).await.unwrap();
        self.recorders.insert(room_id, recorder);
        // update config
        {
            let mut config = self.config.write().await; 
            config.rooms.push(room_id);
            config.save();
        }
        // run recorder
        let recorder = self.recorders.get(&room_id).unwrap();
        recorder.value().run().await;
        Ok(())
    }

    pub async fn remove_recorder(&self, room_id: u64) -> Result<(), String> {
        let recorder = self.recorders.remove(&room_id);
        if recorder.is_none() {
            return Err(format!("Recorder {} not found", room_id));
        }
        Ok(())
    }

    pub async fn clip(&self, room_id: u64, d: f64) -> Result<String, String> {
        let recorder = self.recorders.get(&room_id);
        if recorder.is_none() {
            return Err(format!("Recorder {} not found", room_id));
        }
        let recorder = recorder.unwrap();
        match recorder.value().clip(room_id, d).await {
            Ok(f) => Ok(f),
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn clip_range(&self, room_id: u64, start: f64, end: f64) -> Result<String, String> {
        let recorder = self.recorders.get(&room_id);
        if recorder.is_none() {
            return Err(format!("Recorder {} not found", room_id));
        }
        let recorder = recorder.unwrap();
        match recorder.value().clip_range(start, end).await {
            Ok(f) => Ok(f),
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn get_summary(&self) -> Summary {
        let mut summary = Summary {
            count: self.recorders.len(),
            rooms: Vec::new(),
        };

        for recorder in self.recorders.iter() {
            let recorder = recorder.value();
            let room_info = RoomInfo {
                room_id: recorder.room_id,
                room_title: recorder.room_title.clone(),
                room_cover: recorder.room_cover.clone(),
                room_keyframe: recorder.room_keyframe.clone(),
                user_id: recorder.user_id,
                user_name: recorder.user_name.clone(),
                user_sign: recorder.user_sign.clone(),
                user_avatar: recorder.user_avatar.clone(),
                total_length: *recorder.ts_length.read().await,
                live_status: *recorder.live_status.read().await,
            };
            summary.rooms.push(room_info);
        }

        summary.rooms.sort_by(|a, b| a.room_id.cmp(&b.room_id));

        summary
    }

    pub async fn update_cookies(&self, cookies: &str) {
        // update cookies for all recorders
        for recorder in self.recorders.iter() {
            recorder.value().update_cookies(cookies).await;
        }
    }

    async fn start_hls_server(&self, listener: TcpListener) -> Result<SocketAddr, hyper::Error> {
        let recorders = self.recorders.clone();
        let make_svc = make_service_fn(move |_conn| {
            let recorders = recorders.clone();
            async move {
                Ok::<_, Infallible>(service_fn(move |req: Request<Body>| {
                    let recorders = recorders.clone();
                    async move {
                        let path = req.uri().path();
                        let path_segs: Vec<&str> = path.split('/').collect();
                        // path_segs should at lease with size 3: /21484828/playlist.m3u8
                        if path_segs.len() != 3 {
                            return Ok::<_, Infallible>(Response::builder()
                                .status(404)
                                .body(Body::from("Path Not Found"))
                                .unwrap());
                        }
                        // parse room id
                        let room_id = path_segs[1].parse::<u64>().unwrap();
                        // if path is /room_id/playlist.m3u8
                        if path_segs[2] == "playlist.m3u8" {
                            // get recorder
                            let recorder = recorders.get(&room_id);
                            if recorder.is_none() {
                                return Ok::<_, Infallible>(Response::builder()
                                    .status(404)
                                    .body(Body::from("Recorder Not Found"))
                                    .unwrap());
                            }
                            let recorder = recorder.unwrap();
                            // response with recorder generated m3u8, which contains ts entries that cached in local
                            let m3u8_content = recorder.value().generate_m3u8().await;
                            Ok::<_, Infallible>(Response::builder()
                                .status(200)
                                .header("Content-Type", "application/vnd.apple.mpegurl")
                                .header("Access-Control-Allow-Origin", "*")
                                .header("Access-Control-Allow-Methods", "GET, OPTIONS")
                                .body(Body::from(m3u8_content))
                                .unwrap())
                        } else {
                            // try to find requested ts file in recorder's cache
                            // cache files are stored in {cache_dir}/{room_id}/{ts_file}
                            let ts_file = path_segs[2];
                            let recorder = recorders.get(&room_id);
                            if recorder.is_none() {
                                return Ok::<_, Infallible>(Response::builder()
                                    .status(404)
                                    .body(Body::from("Recorder Not Found"))
                                    .unwrap());
                            }
                            let recorder = recorder.unwrap();
                            let ts_file_path = recorder.value().get_ts_file_path(ts_file).await;
                            let ts_file_content = tokio::fs::read(ts_file_path).await;
                            if ts_file_content.is_err() {
                                return Ok::<_, Infallible>(Response::builder()
                                    .status(404)
                                    .body(Body::from("TS File Not Found"))
                                    .unwrap());
                            }
                            let ts_file_content = ts_file_content.unwrap();
                            Ok::<_, Infallible>(Response::builder()
                                .status(200)
                                .header("Content-Type", "video/MP2T")
                                .header("Access-Control-Allow-Origin", "*")
                                .header("Access-Control-Allow-Methods", "GET, OPTIONS")
                                .body(Body::from(ts_file_content))
                                .unwrap())
                        }
                    }
                }))
            }
        });

        let server = Server::from_tcp(listener.into_std().unwrap())?.serve(make_svc);
        let addr = server.local_addr();
        tokio::spawn(async move {
            if let Err(e) = server.await {
                eprintln!("HLS server error: {}", e);
            }
        });

        println!("HLS server running on http://{}", addr);
        Ok(addr)
    }

    pub async fn get_hls_server_addr(&self) -> Option<SocketAddr> {
        *self.hls_server_addr.read().await
    }
}
