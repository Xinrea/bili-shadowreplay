pub mod bilibili;
use bilibili::errors::BiliClientError;
use bilibili::BiliClient;
use chrono::prelude::*;
use ffmpeg_sidecar::{
    command::FfmpegCommand,
    event::{FfmpegEvent, LogLevel},
};
use futures::future::join_all;
use m3u8_rs::Playlist;
use notify_rust::Notification;
use regex::Regex;
use std::sync::Arc;
use std::thread;

use felgens::{ws_socket_object, FelgensError, WsStreamMessageType};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::sync::{Mutex, RwLock};

use crate::Config;

#[derive(Clone)]
pub struct TsEntry {
    pub url: String,
    pub sequence: u64,
    pub length: f64,
}

#[derive(Clone)]
pub struct BiliRecorder {
    client: Arc<RwLock<BiliClient>>,
    config: Arc<RwLock<Config>>,
    pub room_id: u64,
    pub room_title: String,
    pub room_cover: String,
    pub room_keyframe: String,
    pub user_id: u64,
    pub user_name: String,
    pub user_sign: String,
    pub user_avatar: String,
    pub m3u8_url: Arc<RwLock<String>>,
    pub live_status: Arc<RwLock<bool>>,
    pub latest_sequence: Arc<Mutex<u64>>,
    pub ts_length: Arc<RwLock<f64>>,
    ts_entries: Arc<Mutex<Vec<TsEntry>>>,
    quit: Arc<Mutex<bool>>,
    header: Arc<RwLock<Option<TsEntry>>>,
    stream_type: Arc<RwLock<StreamType>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StreamType {
    TS,
    FMP4,
}

impl BiliRecorder {
    pub async fn new(room_id: u64, config: Arc<RwLock<Config>>) -> Result<Self, BiliClientError> {
        let mut client = BiliClient::new()?;
        client.set_cookies(&config.read().await.cookies);
        let room_info = client.get_room_info(room_id).await?;
        let user_info = client.get_user_info(room_info.user_id).await?;
        let mut m3u8_url = String::from("");
        let mut live_status = false;
        let mut stream_type = StreamType::FMP4;
        if room_info.live_status == 1 {
            live_status = true;
            if let Ok((index_url, stream_type_now)) = client.get_play_url(room_info.room_id).await {
                m3u8_url = index_url;
                stream_type = stream_type_now;
            }
        }
        Ok(Self {
            client: Arc::new(RwLock::new(client)),
            config,
            room_id,
            room_title: room_info.room_title,
            room_cover: room_info.room_cover_url,
            room_keyframe: room_info.room_keyframe_url,
            user_id: room_info.user_id,
            user_name: user_info.user_name,
            user_sign: user_info.user_sign,
            user_avatar: user_info.user_avatar_url,
            m3u8_url: Arc::new(RwLock::new(m3u8_url)),
            live_status: Arc::new(RwLock::new(live_status)),
            latest_sequence: Arc::new(Mutex::new(0)),
            ts_length: Arc::new(RwLock::new(0.0)),
            ts_entries: Arc::new(Mutex::new(Vec::new())),
            quit: Arc::new(Mutex::new(false)),
            header: Arc::new(RwLock::new(None)),
            stream_type: Arc::new(RwLock::new(stream_type)),
        })
    }

    pub async fn update_cookies(&mut self, cookies: &str) {
        self.client.write().await.set_cookies(cookies);
    }

    pub async fn reset(&self) {
        *self.latest_sequence.lock().await = 0;
        *self.ts_length.write().await = 0.0;
        self.ts_entries.lock().await.clear();
        *self.header.write().await = None;
    }

    async fn check_status(&self) -> bool {
        if let Ok(room_info) = self.client.read().await.get_room_info(self.room_id).await {
            let live_status = room_info.live_status == 1;
            // Live status changed from offline to online, reset recorder and then update m3u8 url and stream type.
            self.reset().await;
            if let Ok((index_url, stream_type)) = self
                .client
                .read()
                .await
                .get_play_url(room_info.room_id)
                .await
            {
                self.m3u8_url.write().await.replace_range(.., &index_url);
                *self.stream_type.write().await = stream_type;
            }
            *self.live_status.write().await = live_status;
            live_status
        } else {
            *self.live_status.write().await = false;
            false
        }
    }

    pub async fn run(&self) {
        let self_clone = self.clone();
        thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                while !*self_clone.quit.lock().await {
                    if self_clone.check_status().await {
                        // Live status is ok, start recording.
                        while !*self_clone.quit.lock().await {
                            if let Err(e) = self_clone.update_entries().await {
                                println!("update entries error: {}", e);
                                break;
                            }
                        }
                    }
                    // Every 10s check live status.
                    thread::sleep(std::time::Duration::from_secs(10));
                }
                println!("recording thread {} quit.", self_clone.room_id);
            });
        });
        // Thread for danmaku
        let self_clone = self.clone();
        thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                self_clone.danmu().await;
            });
        });
    }

    async fn danmu(&self) {
        let (tx, rx) = mpsc::unbounded_channel();
        let cookies = self.config.read().await.cookies.clone();
        let uid = self.config.read().await.uid.parse().unwrap();
        let ws = ws_socket_object(tx, uid, self.room_id, cookies.as_str());
        if let Err(e) = tokio::select! {v = ws => v, v = self.recv(self.room_id,rx) => v} {
            println!("{}", e);
        }
    }

    async fn recv(
        &self,
        room: u64,
        mut rx: UnboundedReceiver<WsStreamMessageType>,
    ) -> Result<(), FelgensError> {
        while let Some(msg) = rx.recv().await {
            if let WsStreamMessageType::DanmuMsg(msg) = msg {
                if self.config.read().await.admin_uid.contains(&msg.uid) {
                    let content: String = msg.msg;
                    if content.starts_with("/clip") {
                        let mut duration = 60.0;
                        if content.len() > 5 {
                            let num_part = content.strip_prefix("/clip ").unwrap_or("60");
                            duration = num_part.parse::<u64>().unwrap_or(60) as f64;
                        }
                        if let Err(e) = self.clip(room, duration).await {
                            if let Err(e) = Notification::new()
                                .summary("BiliBili ShadowReplay")
                                .body(format!("生成切片失败: {} - {}s", room, duration).as_str())
                                .icon("bili-shadowreplay")
                                .show()
                            {
                                println!("notification error: {}", e);
                            }
                            println!("clip error: {}", e);
                        } else if let Err(e) = Notification::new()
                            .summary("BiliBili ShadowReplay")
                            .body(format!("生成切片成功: {} - {}s", room, duration).as_str())
                            .icon("bili-shadowreplay")
                            .show()
                        {
                            println!("notification error: {}", e);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn stop(&self) {
        *self.quit.lock().await = false;
    }

    async fn get_playlist(&self) -> Result<Playlist, BiliClientError> {
        let url = self.m3u8_url.read().await.clone();
        let mut index_content = self.client.read().await.get_index_content(&url).await?;
        if index_content.contains("Not Found") {
            // 404 try another time after update
            if self.check_status().await {
                index_content = self.client.read().await.get_index_content(&url).await?;
            } else {
                return Err(BiliClientError::InvalidResponse);
            }
        }
        m3u8_rs::parse_playlist_res(index_content.as_bytes())
            .map_err(|_| BiliClientError::InvalidPlaylist)
    }

    async fn get_header_url(&self) -> Result<String, BiliClientError> {
        let url = self.m3u8_url.read().await.clone();
        let mut index_content = self.client.read().await.get_index_content(&url).await?;
        if index_content.contains("Not Found") {
            // 404 try another time after update
            if self.check_status().await {
                index_content = self.client.read().await.get_index_content(&url).await?;
            } else {
                return Err(BiliClientError::InvalidResponse);
            }
        }
        let mut header_url = String::from("");
        let re = Regex::new(r"h.*\.m4s").unwrap();
        if let Some(captures) = re.captures(&index_content) {
            header_url = captures.get(0).unwrap().as_str().to_string();
        }
        Ok(header_url)
    }

    // {
    //   "format_name": "ts",
    //   "codec": [
    //     {
    //       "codec_name": "avc",
    //       "current_qn": 10000,
    //       "accept_qn": [
    //         10000,
    //         400,
    //         250,
    //         150
    //       ],
    //       "base_url": "/live-bvc/738905/live_51628309_47731828_bluray.m3u8?",
    //       "url_info": [
    //         {
    //           "host": "https://cn-jsyz-ct-03-51.bilivideo.com",
    //           "extra": "expires=1680532720&len=0&oi=3664564898&pt=h5&qn=10000&trid=100352dbcd4ec5494d6083d4a9a3d9f91aa7&sigparams=cdn,expires,len,oi,pt,qn,trid&cdn=cn-gotcha01&sign=829e59d93ef9ffff8e2aa3bb090f1280&sk=4207df3de646838b084f14f252be3aff94df00e145e0110c92421700c186a851&p2p_type=0&sl=6&free_type=0&mid=475210&sid=cn-jsyz-ct-03-51&chash=1&sche=ban&score=13&pp=rtmp&source=onetier&trace=a0c&site=c66c7195b197c2cf30e5715dbf2922b8&order=1",
    //           "stream_ttl": 3600
    //         }
    //       ],
    //       "hdr_qn": null,
    //       "dolby_type": 0,
    //       "attr_name": ""
    //     }
    //   ]
    // }
    // {
    //     "format_name": "fmp4",
    //     "codec": [
    //       {
    //         "codec_name": "avc",
    //         "current_qn": 10000,
    //         "accept_qn": [
    //           10000,
    //           400,
    //           250,
    //           150
    //         ],
    //         "base_url": "/live-bvc/738905/live_51628309_47731828_bluray/index.m3u8?",
    //         "url_info": [
    //           {
    //             "host": "https://cn-jsyz-ct-03-51.bilivideo.com",
    //             "extra": "expires=1680532720&len=0&oi=3664564898&pt=h5&qn=10000&trid=100752dbcd4ec5494d6083d4a9a3d9f91aa7&sigparams=cdn,expires,len,oi,pt,qn,trid&cdn=cn-gotcha01&sign=3d0930160c5870021ebbb457e4630fcf&sk=5bf07b9bbe6df2e0a6bc476fe3d9a642c8e387f5b7e5df7fa9e1b9d0abc8bd13&flvsk=4207df3de646838b084f14f252be3aff94df00e145e0110c92421700c186a851&p2p_type=0&sl=6&free_type=0&mid=475210&sid=cn-jsyz-ct-03-51&chash=1&sche=ban&bvchls=1&score=13&pp=rtmp&source=onetier&trace=a0c&site=c66c7195b197c2cf30e5715dbf2922b8&order=1",
    //             "stream_ttl": 3600
    //           },
    //           {
    //             "host": "https://d1--cn-gotcha208.bilivideo.com",
    //             "extra": "expires=1680532720&len=0&oi=3664564898&pt=h5&qn=10000&trid=100752dbcd4ec5494d6083d4a9a3d9f91aa7&sigparams=cdn,expires,len,oi,pt,qn,trid&cdn=cn-gotcha208&sign=b63815ac70b18420c64a661465f92962&sk=5bf07b9bbe6df2e0a6bc476fe3d9a642c8e387f5b7e5df7fa9e1b9d0abc8bd13&p2p_type=0&sl=6&free_type=0&mid=475210&pp=rtmp&source=onetier&trace=4&site=c66c7195b197c2cf30e5715dbf2922b8&order=2",
    //             "stream_ttl": 3600
    //           }
    //         ],
    //         "hdr_qn": null,
    //         "dolby_type": 0,
    //         "attr_name": ""
    //       }
    //     ]
    //   }
    async fn ts_url(&self, ts_url: &String) -> Result<String, BiliClientError> {
        // Construct url for ts and fmp4 stream.
        match *self.stream_type.read().await {
            StreamType::TS => {
                // Get host from m3u8 url
                let url = self.m3u8_url.read().await.clone();
                if let Some(host_part) = url.strip_prefix("https://") {
                    if let Some(host) = host_part.split('/').next() {
                        Ok(format!("https://{}/{}", host, ts_url))
                    } else {
                        Err(BiliClientError::InvalidUrl)
                    }
                } else {
                    Err(BiliClientError::InvalidUrl)
                }
            }
            StreamType::FMP4 => {
                let url = self.m3u8_url.read().await.clone();
                if let Some(prefix_part) = url.strip_suffix("index.m3u8") {
                    Ok(format!("{}{}", prefix_part, ts_url))
                } else {
                    Err(BiliClientError::InvalidUrl)
                }
            }
        }
    }

    async fn update_entries(&self) -> Result<(), BiliClientError> {
        let parsed = self.get_playlist().await;
        // Check header if None
        if self.header.read().await.is_none() && *self.stream_type.read().await == StreamType::FMP4
        {
            // Get url from EXT-X-MAP
            let header_url = self.get_header_url().await?;
            if header_url.is_empty() {
                return Err(BiliClientError::InvalidPlaylist);
            }
            let full_header_url = self.ts_url(&header_url).await?;
            let header = TsEntry {
                url: full_header_url.clone(),
                sequence: 0,
                length: 0.0,
            };
            // Download header
            if let Err(e) = self
                .client
                .read()
                .await
                .download_ts(
                    &self.config.read().await.cache,
                    self.room_id,
                    &full_header_url,
                )
                .await
            {
                println!("Error downloading header: {:?}", e);
            }
            *self.header.write().await = Some(header);
        }
        match parsed {
            Ok(Playlist::MasterPlaylist(pl)) => println!("Master playlist:\n{:?}", pl),
            Ok(Playlist::MediaPlaylist(pl)) => {
                let mut sequence = pl.media_sequence;
                let mut handles = Vec::new();
                for ts in pl.segments {
                    if sequence <= *self.latest_sequence.lock().await {
                        sequence += 1;
                        continue;
                    }
                    let mut ts_entry = TsEntry {
                        url: ts.uri,
                        sequence,
                        length: ts.duration as f64,
                    };
                    let client = self.client.clone();
                    let ts_url = self.ts_url(&ts_entry.url).await?;
                    ts_entry.url = ts_url.clone();
                    if ts_url.is_empty() {
                        continue;
                    }
                    let room_id = self.room_id;
                    let config = self.config.clone();
                    handles.push(tokio::task::spawn(async move {
                        if let Err(e) = client
                            .read()
                            .await
                            .download_ts(&config.read().await.cache, room_id, &ts_url)
                            .await
                        {
                            println!("download ts failed: {}", e);
                        }
                    }));
                    let mut entries = self.ts_entries.lock().await;
                    entries.push(ts_entry);
                    *self.latest_sequence.lock().await = sequence;
                    let mut total_length = self.ts_length.write().await;
                    *total_length += ts.duration as f64;
                    while *total_length > self.config.read().await.max_len as f64 {
                        *total_length -= entries[0].length;
                        if let Err(e) = std::fs::remove_file(
                            BiliClient::url_to_file_name(
                                &self.config.read().await.cache,
                                room_id,
                                &entries[0].url,
                            )
                            .1,
                        ) {
                            println!("remove file failed: {}", e);
                        }
                        entries.remove(0);
                    }
                    sequence += 1;
                }
                join_all(handles).await.into_iter().for_each(|e| {
                    if let Err(e) = e {
                        println!("download ts failed: {:?}", e);
                    }
                });
            }
            Err(_) => {
                return Err(BiliClientError::InvalidIndex);
            }
        }
        Ok(())
    }

    pub async fn clip(&self, room_id: u64, d: f64) -> Result<String, BiliClientError> {
        let mut duration = d;
        let mut to_combine = Vec::new();
        let header_copy = self.header.read().await.clone();
        let entry_copy = self.ts_entries.lock().await.clone();
        if entry_copy.is_empty() {
            return Err(BiliClientError::EmptyCache);
        }
        for e in entry_copy.iter().rev() {
            let length = e.length;
            to_combine.push(e);
            if duration <= length {
                break;
            }
            duration -= length;
        }
        to_combine.reverse();
        if *self.stream_type.read().await == StreamType::FMP4 {
            // add header to vec
            let header = header_copy.as_ref().unwrap();
            to_combine.insert(0, header);
        }
        let mut file_list = String::new();
        for e in to_combine {
            file_list +=
                &BiliClient::url_to_file_name(&self.config.read().await.cache, room_id, &e.url).1;
            file_list += "|";
        }
        let output_path = self.config.read().await.output.clone();
        std::fs::create_dir_all(&output_path).expect("create clips folder failed");
        let file_name = format!(
            "{}/[{}]{}_({})_{}.mp4",
            output_path,
            self.room_id,
            self.room_title,
            Utc::now().format("%Y-%m-%d-%H-%M-%S"),
            d
        );
        println!("{}", file_name);
        let args = format!("-i concat:{} -c copy", file_list);
        FfmpegCommand::new()
            .args(args.split(' '))
            .output(file_name.clone())
            .spawn()
            .unwrap()
            .iter()
            .unwrap()
            .for_each(|e| match e {
                FfmpegEvent::Log(LogLevel::Error, e) => println!("Error: {}", e),
                FfmpegEvent::Progress(p) => println!("Progress: {}", p.time),
                _ => {}
            });
        Ok(file_name)
    }
}
