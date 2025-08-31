use std::collections::HashMap;

use m3u8_rs::{parse_playlist_res, Playlist};

use crate::{HlsProvider, HlsSegment, HlsStreamError, StreamInfo};

pub struct DouyinProvider {
    room_id: String,
    user_agent: String,
    client: reqwest::Client,
    current_playlist_url: Option<String>,
    current_quality: String,
    available_qualities: HashMap<String, String>, // quality -> url
    target_duration: f64,
    live_id: String,
}

impl DouyinProvider {
    pub async fn new(room_id: &str, _auth: &str) -> Result<Self, HlsStreamError> {
        let user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36".to_string();

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent(&user_agent)
            .build()
            .map_err(HlsStreamError::NetworkError)?;

        let mut provider = Self {
            room_id: room_id.to_string(),
            user_agent,
            client,
            current_playlist_url: None,
            current_quality: String::new(),
            available_qualities: HashMap::new(),
            target_duration: 4.0, // Douyin typically uses 4s segments
            live_id: format!("douyin_{}", room_id),
        };

        // Initialize stream information
        provider.initialize().await?;
        Ok(provider)
    }

    async fn initialize(&mut self) -> Result<(), HlsStreamError> {
        log::debug!("Initializing Douyin provider for room {}", self.room_id);

        // Fetch room information
        let room_info = self.fetch_room_info().await?;

        // Parse stream URLs
        self.parse_stream_data(&room_info)?;

        // Set default quality
        if let Some((quality, url)) = self.available_qualities.iter().next() {
            self.current_quality = quality.clone();
            self.current_playlist_url = Some(url.clone());
            log::info!("Initialized with quality: {}", quality);
        } else {
            return Err(HlsStreamError::InitializationError(
                "No available stream found".to_string(),
            ));
        }

        Ok(())
    }

    async fn fetch_room_info(&self) -> Result<serde_json::Value, HlsStreamError> {
        // Douyin web live room API
        let url = format!(
            "https://live.douyin.com/webcast/room/web/enter/?aid=6383&room_id={}&enter_from=web_live&insert_task_id=&live_reason=&room_id_str={}",
            self.room_id, self.room_id
        );

        let response = self
            .client
            .get(&url)
            .header("Referer", "https://live.douyin.com/")
            .send()
            .await
            .map_err(HlsStreamError::NetworkError)?;

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(HlsStreamError::NetworkError)?;

        // Check if room is live
        let status = json
            .get("data")
            .and_then(|data| data.get("room"))
            .and_then(|room| room.get("status"))
            .and_then(|status| status.as_u64());

        if status != Some(2) {
            // 2 means live
            return Err(HlsStreamError::StreamOffline);
        }

        json.get("data")
            .cloned()
            .ok_or_else(|| HlsStreamError::ParseError("No room data found".to_string()))
    }

    fn parse_stream_data(&mut self, room_data: &serde_json::Value) -> Result<(), HlsStreamError> {
        let room = room_data
            .get("room")
            .ok_or_else(|| HlsStreamError::ParseError("No room info found".to_string()))?;

        let stream_url = room
            .get("stream_url")
            .and_then(|url| url.get("hls_pull_url_map"))
            .ok_or_else(|| HlsStreamError::ParseError("No HLS stream URL found".to_string()))?;

        // Parse different quality streams
        if let Some(origin) = stream_url.get("ORIGIN").and_then(|v| v.as_str()) {
            self.available_qualities
                .insert("原画".to_string(), origin.to_string());
        }

        if let Some(hd) = stream_url.get("HD").and_then(|v| v.as_str()) {
            self.available_qualities
                .insert("高清".to_string(), hd.to_string());
        }

        if let Some(sd) = stream_url.get("SD").and_then(|v| v.as_str()) {
            self.available_qualities
                .insert("标清".to_string(), sd.to_string());
        }

        if self.available_qualities.is_empty() {
            // Fallback: try to get any HLS URL from the stream data
            if let Some(flv_url) = stream_url.get("flv_pull_url").and_then(|v| v.as_str()) {
                // Convert FLV URL to potential HLS URL (this is a simplified approach)
                let hls_url = flv_url.replace(".flv", ".m3u8");
                self.available_qualities.insert("默认".to_string(), hls_url);
            } else {
                return Err(HlsStreamError::ParseError(
                    "No supported stream formats found".to_string(),
                ));
            }
        }

        log::debug!("Found {} quality options", self.available_qualities.len());
        Ok(())
    }

    fn resolve_segment_url(&self, segment_uri: &str, base_url: &str) -> String {
        if segment_uri.starts_with("http") {
            segment_uri.to_string()
        } else {
            let base = base_url
                .rsplit_once('/')
                .map(|(base, _)| base)
                .unwrap_or(base_url);
            format!("{}/{}", base, segment_uri)
        }
    }
}

#[async_trait::async_trait]
impl HlsProvider for DouyinProvider {
    async fn fetch_playlist(&self) -> Result<Vec<HlsSegment>, HlsStreamError> {
        let playlist_url = self
            .current_playlist_url
            .as_ref()
            .ok_or_else(|| HlsStreamError::ParseError("No playlist URL available".to_string()))?;

        log::debug!("Fetching Douyin playlist: {}", playlist_url);

        let response = self
            .client
            .get(playlist_url)
            .header("Referer", "https://live.douyin.com/")
            .send()
            .await
            .map_err(HlsStreamError::NetworkError)?;

        let content = response
            .text()
            .await
            .map_err(HlsStreamError::NetworkError)?;

        match parse_playlist_res(content.as_bytes()) {
            Ok(Playlist::MediaPlaylist(playlist)) => {
                let mut segments = Vec::new();

                for (i, segment) in playlist.segments.iter().enumerate() {
                    let segment_url = self.resolve_segment_url(&segment.uri, playlist_url);

                    let hls_segment = HlsSegment::new(
                        playlist.media_sequence + i as u64,
                        segment.duration.into(),
                        segment_url,
                    )
                    .with_discontinuity(segment.discontinuity)
                    .with_program_date_time(
                        segment.program_date_time.clone().map(|dt| dt.to_rfc3339()),
                    )
                    .with_byte_range(
                        segment
                            .byte_range
                            .clone()
                            .and_then(|br| br.offset.map(|offset| (offset, br.length))),
                    );

                    segments.push(hls_segment);
                }

                log::debug!("Parsed {} segments from Douyin playlist", segments.len());
                Ok(segments)
            }
            Ok(_) => Err(HlsStreamError::ParseError(
                "Not a media playlist".to_string(),
            )),
            Err(e) => Err(HlsStreamError::ParseError(format!(
                "M3U8 parse error: {}",
                e
            ))),
        }
    }

    async fn get_info(&self) -> Result<StreamInfo, HlsStreamError> {
        Ok(StreamInfo {
            live_id: self.live_id.clone(),
            current_quality: self.current_quality.clone(),
            available_qualities: self.available_qualities.keys().cloned().collect(),
            playlist_url: self.current_playlist_url.clone().unwrap_or_default(),
            target_duration: self.target_duration,
            is_live: true,
            sequence_start: 0,
        })
    }

    async fn change_quality(&mut self, quality: &str) -> Result<(), HlsStreamError> {
        if let Some(url) = self.available_qualities.get(quality) {
            self.current_quality = quality.to_string();
            self.current_playlist_url = Some(url.clone());
            log::info!("Changed Douyin quality to: {}", quality);
            Ok(())
        } else {
            Err(HlsStreamError::ParseError(format!(
                "Quality '{}' not available",
                quality
            )))
        }
    }

    async fn stop(&mut self) -> Result<(), HlsStreamError> {
        log::debug!("Stopping Douyin provider");
        self.current_playlist_url = None;
        Ok(())
    }
}
