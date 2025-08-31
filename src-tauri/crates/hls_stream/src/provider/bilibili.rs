use std::collections::HashMap;

use m3u8_rs::{parse_playlist_res, Playlist};

use crate::{HlsProvider, HlsSegment, HlsStreamError, StreamInfo};

pub struct BilibiliProvider {
    room_id: String,
    cookies: String,
    client: reqwest::Client,
    current_playlist_url: Option<String>,
    current_quality: String,
    available_qualities: HashMap<String, String>, // quality -> url
    target_duration: f64,
    live_id: String,
}

impl BilibiliProvider {
    pub async fn new(room_id: &str, cookies: &str) -> Result<Self, HlsStreamError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .map_err(HlsStreamError::NetworkError)?;

        let mut provider = Self {
            room_id: room_id.to_string(),
            cookies: cookies.to_string(),
            client,
            current_playlist_url: None,
            current_quality: String::new(),
            available_qualities: HashMap::new(),
            target_duration: 6.0,
            live_id: format!("bili_{}", room_id),
        };

        // Initialize stream information
        provider.initialize().await?;
        Ok(provider)
    }

    async fn initialize(&mut self) -> Result<(), HlsStreamError> {
        log::debug!("Initializing Bilibili provider for room {}", self.room_id);

        // Check if room is live first
        let room_info = self.fetch_room_info().await?;
        if room_info.get("live_status").and_then(|v| v.as_u64()) != Some(1) {
            return Err(HlsStreamError::StreamOffline);
        }

        // Get play information
        let play_info = self.fetch_play_info().await?;

        // Parse available qualities
        self.parse_qualities(&play_info)?;

        // Set default quality (highest available)
        if let Some((quality, url)) = self.available_qualities.iter().next() {
            self.current_quality = quality.clone();
            self.current_playlist_url = Some(url.clone());
            log::info!("Initialized with quality: {}", quality);
        } else {
            return Err(HlsStreamError::InitializationError(
                "No available qualities found".to_string(),
            ));
        }

        Ok(())
    }

    async fn fetch_room_info(&self) -> Result<serde_json::Value, HlsStreamError> {
        let url = format!(
            "https://api.live.bilibili.com/xlive/web-room/v1/index/getInfoByRoom?room_id={}",
            self.room_id
        );

        let response = self
            .client
            .get(&url)
            .header("Cookie", &self.cookies)
            .header("Referer", "https://live.bilibili.com/")
            .send()
            .await
            .map_err(HlsStreamError::NetworkError)?;

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(HlsStreamError::NetworkError)?;

        if json.get("code").and_then(|v| v.as_u64()) != Some(0) {
            return Err(HlsStreamError::ParseError(
                "Room info request failed".to_string(),
            ));
        }

        json.get("data")
            .cloned()
            .ok_or_else(|| HlsStreamError::ParseError("No room data found".to_string()))
    }

    async fn fetch_play_info(&self) -> Result<serde_json::Value, HlsStreamError> {
        let url = format!(
            "https://api.live.bilibili.com/xlive/web-room/v2/index/getRoomPlayInfo?room_id={}&protocol=0,1&format=0,1,2&codec=0,1&qn=10000&platform=web&ptype=8",
            self.room_id
        );

        let response = self
            .client
            .get(&url)
            .header("Cookie", &self.cookies)
            .header(
                "Referer",
                &format!("https://live.bilibili.com/{}", self.room_id),
            )
            .send()
            .await
            .map_err(HlsStreamError::NetworkError)?;

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(HlsStreamError::NetworkError)?;

        if json.get("code").and_then(|v| v.as_u64()) != Some(0) {
            return Err(HlsStreamError::ParseError(
                "Play info request failed".to_string(),
            ));
        }

        json.get("data")
            .cloned()
            .ok_or_else(|| HlsStreamError::ParseError("No play data found".to_string()))
    }

    fn parse_qualities(&mut self, play_info: &serde_json::Value) -> Result<(), HlsStreamError> {
        let play_url_info = play_info
            .get("playurl_info")
            .and_then(|v| v.get("playurl"))
            .and_then(|v| v.get("stream"))
            .and_then(|v| v.as_array())
            .ok_or_else(|| HlsStreamError::ParseError("Invalid play URL structure".to_string()))?;

        for stream in play_url_info {
            let format_info = stream
                .get("format")
                .and_then(|v| v.as_array())
                .ok_or_else(|| {
                    HlsStreamError::ParseError("Invalid format structure".to_string())
                })?;

            for format in format_info {
                if format.get("format_name").and_then(|v| v.as_str()) == Some("fmp4") {
                    let codec_info =
                        format
                            .get("codec")
                            .and_then(|v| v.as_array())
                            .ok_or_else(|| {
                                HlsStreamError::ParseError("Invalid codec structure".to_string())
                            })?;

                    for codec in codec_info {
                        let url_info = codec
                            .get("url_info")
                            .and_then(|v| v.as_array())
                            .ok_or_else(|| {
                                HlsStreamError::ParseError("Invalid URL info structure".to_string())
                            })?;

                        for url_data in url_info {
                            let host =
                                url_data
                                    .get("host")
                                    .and_then(|v| v.as_str())
                                    .ok_or_else(|| {
                                        HlsStreamError::ParseError("No host found".to_string())
                                    })?;
                            let extra = url_data.get("extra").and_then(|v| v.as_str()).ok_or_else(
                                || HlsStreamError::ParseError("No extra found".to_string()),
                            )?;

                            let base_url =
                                codec.get("base_url").and_then(|v| v.as_str()).ok_or_else(
                                    || HlsStreamError::ParseError("No base URL found".to_string()),
                                )?;

                            let full_url = format!("https://{}{}{}", host, base_url, extra);

                            // Use quality description or fallback to codec
                            let quality_desc = codec
                                .get("current_qn")
                                .and_then(|v| v.as_u64())
                                .map(|qn| match qn {
                                    10000 => "原画",
                                    400 => "蓝光",
                                    250 => "超清",
                                    150 => "高清",
                                    80 => "流畅",
                                    _ => "未知",
                                })
                                .unwrap_or("原画");

                            self.available_qualities
                                .insert(quality_desc.to_string(), full_url);
                            log::debug!("Found quality: {} -> URL available", quality_desc);
                            break; // Use first available URL
                        }
                        break; // Use first codec
                    }
                    break; // Use fmp4 format
                }
            }
        }

        if self.available_qualities.is_empty() {
            return Err(HlsStreamError::ParseError(
                "No supported stream formats found".to_string(),
            ));
        }

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

    /// Parse BILI-AUX offset from unknown tags
    /// Format: #BILI-AUX:<HEX_OFFSET>|<EXTRA_DATA>
    fn parse_bili_aux_offset(&self, segment: &m3u8_rs::MediaSegment) -> Option<i64> {
        for tag in &segment.unknown_tags {
            if tag.tag == "BILI-AUX" {
                if let Some(rest) = &tag.rest {
                    // Split by '|' and take the first part (hex offset)
                    let parts: Vec<&str> = rest.split('|').collect();
                    if let Some(hex_offset) = parts.first() {
                        // Parse hex string to i64
                        if let Ok(offset) = i64::from_str_radix(hex_offset, 16) {
                            return Some(offset);
                        } else {
                            log::warn!("Failed to parse BILI-AUX hex offset: {}", hex_offset);
                        }
                    }
                } else {
                    log::warn!("BILI-AUX tag has no content");
                }
                break;
            }
        }
        None
    }
}

#[async_trait::async_trait]
impl HlsProvider for BilibiliProvider {
    async fn fetch_playlist(&self) -> Result<Vec<HlsSegment>, HlsStreamError> {
        let playlist_url = self
            .current_playlist_url
            .as_ref()
            .ok_or_else(|| HlsStreamError::ParseError("No playlist URL available".to_string()))?;

        log::debug!("Fetching playlist: {}", playlist_url);

        let response = self
            .client
            .get(playlist_url)
            .header(
                "Referer",
                &format!("https://live.bilibili.com/{}", self.room_id),
            )
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

                    let mut hls_segment = HlsSegment::new(
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

                    // Parse BILI-AUX timing information
                    if let Some(bili_aux_offset) = self.parse_bili_aux_offset(segment) {
                        hls_segment = hls_segment.with_metadata(
                            "bili_aux_offset".to_string(),
                            serde_json::Value::Number(serde_json::Number::from(bili_aux_offset)),
                        );
                        log::debug!(
                            "Segment {} BILI-AUX offset: {}ms",
                            playlist.media_sequence + i as u64,
                            bili_aux_offset
                        );
                    }

                    segments.push(hls_segment);
                }

                log::debug!("Parsed {} segments from playlist", segments.len());
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
            log::info!("Changed quality to: {}", quality);
            Ok(())
        } else {
            Err(HlsStreamError::ParseError(format!(
                "Quality '{}' not available",
                quality
            )))
        }
    }

    async fn stop(&mut self) -> Result<(), HlsStreamError> {
        log::debug!("Stopping Bilibili provider");
        self.current_playlist_url = None;
        Ok(())
    }
}
