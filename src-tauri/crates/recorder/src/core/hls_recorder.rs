use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::{path::PathBuf, sync::Arc};

use chrono::Utc;
use m3u8_rs::{MediaPlaylist, Playlist, VariantStream};
use reqwest::header::HeaderMap;
use std::time::Duration;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::core::playlist::{playlist_content_preview, HlsPlaylist};
use crate::core::{Codec, Format};
use crate::errors::RecorderError;
use crate::{core::HlsStream, events::RecorderEvent};
use ffmpeg_utils::{extract_video_metadata, VideoMetadata};

const UPDATE_TIMEOUT: Duration = Duration::from_secs(20);
const UPDATE_INTERVAL: Duration = Duration::from_secs(1);
const PLAYLIST_FILE_NAME: &str = "playlist.m3u8";
const DOWNLOAD_RETRY: u32 = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HlsVariantSelection {
    /// Preserve the first media variant selected for a recording.
    #[default]
    First,
    /// Track the highest available bandwidth; variant changes end the current
    /// recording segment so a higher-quality stream can start cleanly.
    HighestBandwidth,
}

/// A recorder for HLS streams
///
/// This recorder fetches, caches and serves TS entries, currently supporting `StreamType::FMP4, StreamType::TS`.
///
/// Segments will be downloaded to work_dir, and `playlist.m3u8` will be generated in work_dir.
#[derive(Clone)]
pub struct HlsRecorder {
    room_id: String,
    stream: Arc<HlsStream>,
    /// The media playlist selected for the current recording segment. The
    /// `First` policy keeps this pinned; `HighestBandwidth` refreshes it until
    /// a quality change starts a new segment.
    selected_stream: Arc<RwLock<Option<HlsStream>>>,
    selected_variant_key: Arc<RwLock<Option<String>>>,
    variant_selection: HlsVariantSelection,
    client: reqwest::Client,
    event_channel: broadcast::Sender<RecorderEvent>,
    work_dir: PathBuf,
    playlist: Arc<Mutex<HlsPlaylist>>,
    headers: HeaderMap,

    enabled: Arc<AtomicBool>,

    sequence: Arc<AtomicU64>,
    sequence_file: Arc<RwLock<File>>,
    updated_at: Arc<AtomicI64>,

    pre_metadata: Arc<RwLock<Option<VideoMetadata>>>,
}

impl HlsRecorder {
    pub async fn new(
        room_id: String,
        stream: Arc<HlsStream>,
        client: reqwest::Client,
        cookies: Option<String>,
        event_channel: broadcast::Sender<RecorderEvent>,
        work_dir: PathBuf,
        enabled: Arc<AtomicBool>,
    ) -> Result<Self, RecorderError> {
        Self::new_with_variant_selection(
            room_id,
            stream,
            client,
            cookies,
            event_channel,
            work_dir,
            enabled,
            HlsVariantSelection::First,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn new_with_variant_selection(
        room_id: String,
        stream: Arc<HlsStream>,
        client: reqwest::Client,
        cookies: Option<String>,
        event_channel: broadcast::Sender<RecorderEvent>,
        work_dir: PathBuf,
        enabled: Arc<AtomicBool>,
        variant_selection: HlsVariantSelection,
    ) -> Result<Self, RecorderError> {
        // try to create work_dir
        if !work_dir.exists() {
            std::fs::create_dir_all(&work_dir)?;
        }
        let playlist_path = work_dir.join(PLAYLIST_FILE_NAME);

        // set user agent
        let user_agent =
            crate::utils::user_agent_generator::UserAgentGenerator::new().generate(false);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "user-agent",
            user_agent
                .parse()
                .expect("generated user agent is a valid header value"),
        );
        if let Some(cookies) = cookies {
            headers.insert("cookie", crate::utils::header_value("cookie", &cookies)?);
        }

        let sequence_path = work_dir.join(".sequence");
        let mut sequence_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&sequence_path)
            .await
            .map_err(RecorderError::IoError)?;

        let mut sequence_buf = String::new();
        sequence_file
            .read_to_string(&mut sequence_buf)
            .await
            .map_err(RecorderError::IoError)?;
        let trimmed = sequence_buf.trim();
        let sequence = trimmed.parse::<u64>().unwrap_or(0);

        // If the file is newly created / empty, normalize it to "0"
        if trimmed.is_empty() {
            sequence_file
                .set_len(0)
                .await
                .map_err(RecorderError::IoError)?;
            sequence_file
                .seek(SeekFrom::Start(0))
                .await
                .map_err(RecorderError::IoError)?;
            sequence_file
                .write_all(b"0")
                .await
                .map_err(RecorderError::IoError)?;
            let _ = sequence_file.flush().await;
            sequence_file
                .seek(SeekFrom::Start(0))
                .await
                .map_err(RecorderError::IoError)?;
        }

        let playlist = HlsPlaylist::new(playlist_path).await?;

        Ok(Self {
            room_id,
            stream,
            selected_stream: Arc::new(RwLock::new(None)),
            selected_variant_key: Arc::new(RwLock::new(None)),
            variant_selection,
            client,
            event_channel,
            work_dir,
            playlist: Arc::new(Mutex::new(playlist)),
            headers,
            enabled,
            sequence: Arc::new(AtomicU64::new(sequence)),
            updated_at: Arc::new(AtomicI64::new(chrono::Utc::now().timestamp_millis())),
            pre_metadata: Arc::new(RwLock::new(None)),
            sequence_file: Arc::new(RwLock::new(sequence_file)),
        })
    }

    /// Start the recorder blockingly
    ///
    /// This will start the recorder and update the entries periodically.
    pub async fn start(&self) -> Result<(), RecorderError> {
        while self.enabled.load(Ordering::Relaxed) {
            let result = self.update_entries().await;
            if let Err(e) = result {
                match e {
                    RecorderError::ResolutionChanged { .. } => {
                        log::error!("Resolution changed: {}", e);
                        self.playlist.lock().await.close().await?;
                        return Err(e);
                    }
                    RecorderError::UpdateTimeout => {
                        log::error!(
                            "Source playlist is not updated for a long time, stop recording"
                        );
                        self.playlist.lock().await.close().await?;
                        return Err(e);
                    }
                    RecorderError::M3u8ParseFailed { .. } => {
                        log::error!("[{}]M3u8 parse failed: {}", self.room_id, e);
                        return Err(e);
                    }
                    RecorderError::StreamExpired { .. } => {
                        log::error!("[{}]Stream expired", self.room_id);
                        return Err(e);
                    }
                    _ => {
                        // Other errors are not critical, just log it
                        log::error!("[{}]Update entries error: {}", self.room_id, e);
                        return Err(e);
                    }
                }
            }

            tokio::time::sleep(UPDATE_INTERVAL).await;
        }

        Ok(())
    }

    pub async fn stop(&self) {
        self.enabled.store(false, Ordering::Relaxed);
    }

    async fn query_playlist(&self, stream: &HlsStream) -> Result<Playlist, RecorderError> {
        let url = stream.index();
        let response = self
            .client
            .get(url)
            .headers(self.headers.clone())
            .send()
            .await?;
        let bytes = response.bytes().await?;
        let (_, playlist) =
            m3u8_rs::parse_playlist(&bytes).map_err(|_| RecorderError::M3u8ParseFailed {
                content: playlist_content_preview(&bytes),
            })?;
        Ok(playlist)
    }

    async fn query_media_playlist(&self) -> Result<MediaPlaylist, RecorderError> {
        let selected_stream = self.selected_stream.read().await.clone();
        if self.variant_selection == HlsVariantSelection::HighestBandwidth {
            if let Some(selected_stream) = selected_stream {
                return self
                    .query_highest_bandwidth_media_playlist(&selected_stream)
                    .await;
            }
        } else if let Some(selected_stream) = selected_stream {
            match self.read_media_playlist(&selected_stream).await {
                Ok(playlist) => return Ok(playlist),
                Err(error) => {
                    let selected_key = self.selected_variant_key.read().await.clone();
                    let Some(selected_key) = selected_key else {
                        return Err(error);
                    };
                    log::warn!(
                        "Selected HLS variant is unavailable; refreshing its signed URL from the master playlist"
                    );
                    return self
                        .refresh_selected_variant(&selected_key)
                        .await
                        .or(Err(error));
                }
            }
        }

        match self.query_playlist(&self.stream).await? {
            Playlist::MediaPlaylist(playlist) => {
                *self.selected_stream.write().await = Some((*self.stream).clone());
                Ok(playlist)
            }
            Playlist::MasterPlaylist(playlist) => {
                // YouTube may add higher-quality variants after a live starts.
                // Its policy rechecks the master and ends this recording segment
                // when the best available variant changes; other platforms keep
                // their first media variant pinned.
                let variant = select_media_variant(&playlist.variants, self.variant_selection)
                    .ok_or_else(|| RecorderError::M3u8ParseFailed {
                        content: "No variants found".to_string(),
                    })?;
                let variant_url = resolve_variant_url(&self.stream.index(), &variant.uri)?;
                let selected_key = variant_identity(&variant_url);
                if self.variant_selection == HlsVariantSelection::HighestBandwidth
                    && self
                        .selected_variant_key
                        .read()
                        .await
                        .as_ref()
                        .is_some_and(|previous| previous != &selected_key)
                {
                    return Err(RecorderError::ResolutionChanged {
                        err: "Highest-bandwidth HLS variant changed; starting a new recording segment"
                            .to_string(),
                    });
                }
                let selected_stream = construct_stream_from_variant(
                    &self.stream.id,
                    &variant_url,
                    self.stream.format.clone(),
                    self.stream.codec.clone(),
                )
                .await?;
                let media_playlist = self.read_media_playlist(&selected_stream).await?;
                *self.selected_variant_key.write().await = Some(selected_key);
                *self.selected_stream.write().await = Some(selected_stream);
                Ok(media_playlist)
            }
        }
    }

    async fn query_highest_bandwidth_media_playlist(
        &self,
        selected_stream: &HlsStream,
    ) -> Result<MediaPlaylist, RecorderError> {
        if self.stream.is_expired() {
            return Err(RecorderError::StreamExpired {
                expire: self.stream.expire,
            });
        }

        let master = match self.query_playlist(&self.stream).await {
            Ok(Playlist::MasterPlaylist(master)) => master,
            Ok(Playlist::MediaPlaylist(_)) => {
                return self.read_media_playlist(selected_stream).await
            }
            Err(_error) => {
                log::warn!("HLS master refresh failed; continuing with the pinned media variant");
                return self.read_media_playlist(selected_stream).await;
            }
        };
        let variant =
            best_media_variant(&master.variants).ok_or_else(|| RecorderError::M3u8ParseFailed {
                content: "No variants found".to_string(),
            })?;
        let master_url = self.stream.index();
        let variant_url = resolve_variant_url(&master_url, &variant.uri)?;
        let selected_key = variant_identity(&variant_url);
        let previous_key = self.selected_variant_key.read().await.clone();
        if previous_key
            .as_ref()
            .is_some_and(|previous| previous != &selected_key)
        {
            return Err(RecorderError::ResolutionChanged {
                err: "Highest-bandwidth HLS variant changed; starting a new recording segment"
                    .to_string(),
            });
        }

        let updated_stream = construct_stream_from_variant(
            &self.stream.id,
            &variant_url,
            self.stream.format.clone(),
            self.stream.codec.clone(),
        )
        .await?;
        match self.read_media_playlist(&updated_stream).await {
            Ok(playlist) => {
                *self.selected_variant_key.write().await = Some(selected_key);
                *self.selected_stream.write().await = Some(updated_stream);
                Ok(playlist)
            }
            Err(_error) => {
                log::warn!(
                    "Refreshed HLS variant is unavailable; continuing with the pinned media variant"
                );
                self.read_media_playlist(selected_stream).await
            }
        }
    }

    async fn refresh_selected_variant(
        &self,
        selected_key: &str,
    ) -> Result<MediaPlaylist, RecorderError> {
        let master = self.query_playlist(&self.stream).await?;
        let Playlist::MasterPlaylist(master) = master else {
            return Err(RecorderError::M3u8ParseFailed {
                content: "Pinned HLS variant is unavailable".to_string(),
            });
        };
        let master_url = self.stream.index();
        let pinned_variant = master.variants.iter().find(|variant| {
            !variant.is_i_frame
                && resolve_variant_url(&master_url, &variant.uri)
                    .is_ok_and(|url| variant_identity(&url) == selected_key)
        });
        let variant = pinned_variant
            .or_else(|| select_media_variant(&master.variants, self.variant_selection));
        let variant = variant.ok_or_else(|| RecorderError::M3u8ParseFailed {
            content: "No variants found".to_string(),
        })?;
        if pinned_variant.is_none() {
            log::warn!("Pinned HLS variant disappeared; selecting the best available variant");
        }
        let variant_url = resolve_variant_url(&master_url, &variant.uri)?;
        let selected_stream = construct_stream_from_variant(
            &self.stream.id,
            &variant_url,
            self.stream.format.clone(),
            self.stream.codec.clone(),
        )
        .await?;
        let media_playlist = self.read_media_playlist(&selected_stream).await?;
        *self.selected_variant_key.write().await = Some(variant_identity(&variant_url));
        *self.selected_stream.write().await = Some(selected_stream);
        Ok(media_playlist)
    }

    async fn read_media_playlist(
        &self,
        stream: &HlsStream,
    ) -> Result<MediaPlaylist, RecorderError> {
        match self.query_playlist(stream).await? {
            Playlist::MediaPlaylist(playlist) => Ok(playlist),
            Playlist::MasterPlaylist(_) => Err(RecorderError::M3u8ParseFailed {
                content: "No media playlist found".to_string(),
            }),
        }
    }

    async fn update_entries(&self) -> Result<(), RecorderError> {
        let media_playlist = self.query_media_playlist().await?;
        let selected_stream = self
            .selected_stream
            .read()
            .await
            .clone()
            .unwrap_or_else(|| (*self.stream).clone());
        let playlist_sequence = media_playlist.media_sequence;
        let last_sequence = self.sequence.load(Ordering::Relaxed);
        let last_metadata = self.pre_metadata.read().await.clone();
        let mut updated = false;
        let mut duration_delta = 0.0;
        let mut size_delta = 0;
        for (i, segment) in media_playlist.segments.iter().enumerate() {
            let segment_sequence = playlist_sequence + i as u64;
            let segment_full_url = selected_stream.ts_url(&segment.uri);
            let filename = local_segment_filename(segment_sequence, &segment.uri);
            if segment_sequence <= last_sequence {
                continue;
            }

            let segment_path = self.work_dir.join(&filename);
            let Ok(size) = download(
                &self.client,
                &segment_full_url,
                &segment_path,
                DOWNLOAD_RETRY,
            )
            .await
            else {
                log::error!("Download failed: {:#?}", segment);
                return Err(RecorderError::IoError(std::io::Error::other(
                    "Download failed",
                )));
            };

            let mut segment = segment.clone();
            segment.uri = filename;
            if segment.program_date_time.is_none() {
                segment.program_date_time.replace(Utc::now().into());
            }

            // check if the stream is changed
            let segment_metadata = extract_video_metadata(&segment_path)
                .await
                .map_err(RecorderError::FfmpegError)?;

            // IMPORTANT: This handles bilibili ts stream segment, which might lack of SPS/PPS and need to be appended behind last segment
            if segment_metadata.seems_corrupted() {
                let mut playlist = self.playlist.lock().await;
                if playlist.is_empty().await {
                    // ignore this segment
                    log::error!(
                        "Segment is corrupted and has no previous segment, ignore: {}",
                        segment_path.display()
                    );
                    continue;
                }

                let Some(last_segment) = playlist.last_segment().await else {
                    log::error!(
                        "Playlist has no last segment, ignore: {}",
                        segment_path.display()
                    );
                    continue;
                };
                let last_segment_uri = last_segment.uri.clone();
                let last_segment_path = segment_path.with_file_name(last_segment_uri);
                // append segment data behind last segment data
                let mut last_segment_file = OpenOptions::new()
                    .append(true)
                    .open(&last_segment_path)
                    .await?;
                log::debug!(
                    "Appending segment data behind last segment: {}",
                    last_segment_path.display()
                );
                let mut segment_file = File::open(&segment_path).await?;
                let mut buffer = Vec::new();
                segment_file.read_to_end(&mut buffer).await?;
                last_segment_file.write_all(&buffer).await?;
                let _ = tokio::fs::remove_file(&segment_path).await;
                playlist.append_last_segment(segment.clone()).await?;

                duration_delta += segment_metadata.duration;
                size_delta += size;
                self.update_sequence(segment_sequence).await?;
                self.updated_at
                    .store(chrono::Utc::now().timestamp_millis(), Ordering::Relaxed);
                updated = true;
                continue;
            }

            if let Some(last_metadata) = &last_metadata {
                // Only a different resolution or codec makes the segments
                // unplayable as one recording; their length naturally differs
                // from segment to segment.
                if !last_metadata.same_stream_shape(&segment_metadata) {
                    return Err(RecorderError::ResolutionChanged {
                        err: "Resolution changed".to_string(),
                    });
                }
            } else {
                self.pre_metadata
                    .write()
                    .await
                    .replace(segment_metadata.clone());
            }

            let mut new_segment = segment.clone();
            new_segment.duration = segment_metadata.duration as f32;

            self.playlist.lock().await.add_segment(new_segment).await?;

            duration_delta += segment_metadata.duration;
            size_delta += size;
            self.update_sequence(segment_sequence).await?;
            self.updated_at
                .store(chrono::Utc::now().timestamp_millis(), Ordering::Relaxed);
            updated = true;
        }

        // Source playlist may not be updated for a long time, check if it's timeout
        let current_time = chrono::Utc::now().timestamp_millis();
        if self.updated_at.load(Ordering::Relaxed) + (UPDATE_TIMEOUT.as_millis() as i64)
            < current_time
        {
            return Err(RecorderError::UpdateTimeout);
        }

        if updated {
            let _ = self.event_channel.send(RecorderEvent::RecordUpdate {
                live_id: self.stream.id.clone(),
                duration_secs: duration_delta,
                cached_size_bytes: size_delta,
            });
        }

        if self.stream.is_expired() {
            return Err(RecorderError::StreamExpired {
                expire: self.stream.expire,
            });
        }

        Ok(())
    }

    async fn update_sequence(&self, sequence: u64) -> Result<(), RecorderError> {
        let mut file = self.sequence_file.write().await;
        persist_sequence(&mut file, sequence).await?;
        self.sequence.store(sequence, Ordering::Relaxed);
        Ok(())
    }
}

fn select_media_variant(
    variants: &[VariantStream],
    selection: HlsVariantSelection,
) -> Option<&VariantStream> {
    match selection {
        HlsVariantSelection::First => variants.iter().find(|variant| !variant.is_i_frame),
        HlsVariantSelection::HighestBandwidth => best_media_variant(variants),
    }
}

fn best_media_variant(variants: &[VariantStream]) -> Option<&VariantStream> {
    variants
        .iter()
        .filter(|variant| !variant.is_i_frame)
        .max_by_key(|variant| variant.average_bandwidth.unwrap_or(variant.bandwidth))
}

/// Use stable YouTube itags when available; otherwise use the resolved variant
/// path so signatures and expiry tokens can refresh without changing quality.
fn variant_identity(variant_url: &str) -> String {
    let Ok(url) = url::Url::parse(variant_url) else {
        return variant_url
            .split('?')
            .next()
            .unwrap_or(variant_url)
            .to_string();
    };
    let segments: Vec<_> = url.path_segments().into_iter().flatten().collect();
    if let Some(itag) = segments
        .windows(2)
        .find(|pair| pair[0] == "itag")
        .map(|pair| pair[1])
    {
        return format!("itag:{itag}");
    }
    url.path().to_string()
}

fn resolve_variant_url(master_url: &str, variant_uri: &str) -> Result<String, RecorderError> {
    let master = url::Url::parse(master_url).map_err(|_| RecorderError::M3u8ParseFailed {
        content: "Invalid master playlist URL".to_string(),
    })?;
    master
        .join(variant_uri)
        .map(|url| url.to_string())
        .map_err(|_| RecorderError::M3u8ParseFailed {
            content: "Invalid HLS variant URI".to_string(),
        })
}

/// HLS source playlists may use absolute URLs with query signatures as segment
/// URIs (YouTube does this). Store those chunks under stable local names and
/// rewrite the recorded playlist to reference them. Keep relative segment paths
/// unchanged for the platforms that already use local-friendly names.
fn local_segment_filename(sequence: u64, uri: &str) -> String {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        let path = uri.split('?').next().unwrap_or(uri);
        let extension = path
            .rsplit('/')
            .next()
            .and_then(|name| Path::new(name).extension())
            .and_then(|extension| extension.to_str())
            .filter(|extension| {
                !extension.is_empty()
                    && extension
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric())
            })
            .unwrap_or("ts");
        format!("segment-{sequence}.{extension}")
    } else {
        uri.split('?').next().unwrap_or(uri).to_string()
    }
}

async fn persist_sequence(file: &mut File, sequence: u64) -> std::io::Result<()> {
    file.set_len(0).await?;
    file.seek(SeekFrom::Start(0)).await?;
    file.write_all(sequence.to_string().as_bytes()).await?;
    file.flush().await
}

/// Download url content into fpath
async fn download_inner(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
) -> Result<u64, RecorderError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        let status = response.status();
        log::warn!("Download segment failed: {url}: {status}");
        return Err(RecorderError::InvalidResponseStatus { status });
    }
    let bytes = response.bytes().await?;
    let size = bytes.len() as u64;
    let mut file = tokio::fs::File::create(&path).await?;
    let mut content = std::io::Cursor::new(bytes.clone());
    tokio::io::copy(&mut content, &mut file).await?;
    Ok(size)
}

async fn download(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    retry: u32,
) -> Result<u64, RecorderError> {
    for i in 0..retry {
        let result = download_inner(client, url, path).await;
        if let Ok(size) = result {
            return Ok(size);
        }
        log::error!("Download failed, retry: {}", i);
        // sleep for 500 ms
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    Err(RecorderError::IoError(std::io::Error::other(
        "Download failed",
    )))
}

pub async fn construct_stream_from_variant(
    id: &str,
    variant_url: &str,
    format: Format,
    codec: Codec,
) -> Result<HlsStream, RecorderError> {
    // construct the real stream from variant
    // example: https://cn-jsnt-ct-01-07.bilivideo.com/live-bvc/930889/live_2124647716_1414766_bluray/index.m3u8?expires=1760808243&other=kldskf
    let (body, extra) = variant_url.split_once('?').unwrap_or((variant_url, ""));
    // body example: https://cn-jsnt-ct-01-07.bilivideo.com/live-bvc/930889/live_2124647716_1414766_bluray/index.m3u8

    // extract host, should be like: https://cn-jsnt-ct-01-07.bilivideo.com, which contains http schema
    let host = if let Some(schema_end) = body.find("://") {
        let after_schema = &body[schema_end + 3..];
        if let Some(path_start) = after_schema.find('/') {
            format!("{}{}", &body[..schema_end + 3], &after_schema[..path_start])
        } else {
            body.to_string()
        }
    } else {
        return Err(RecorderError::M3u8ParseFailed {
            content: "Invalid URL format: missing protocol".to_string(),
        });
    };

    // extract base, should be like: /live-bvc/930889/live_2124647716_1414766_bluray/index.m3u8
    let base = if let Some(schema_end) = body.find("://") {
        let after_schema = &body[schema_end + 3..];
        if let Some(path_start) = after_schema.find('/') {
            format!("/{}", &after_schema[path_start + 1..])
        } else {
            "/".to_string()
        }
    } else {
        return Err(RecorderError::M3u8ParseFailed {
            content: "Invalid URL format: missing protocol".to_string(),
        });
    };

    // Add '?' to base if there are query parameters, to match the expected format
    let base_with_query = if !extra.is_empty() {
        format!("{}?", base)
    } else {
        base
    };

    // try to match expire from extra with regex
    let expire_regex =
        regex::Regex::new(r"(?:expires=|/expire/)(\d+)").expect("expires regex is a valid literal");
    let expire = expire_regex
        .captures(extra)
        .or_else(|| expire_regex.captures(body))
        .and_then(|captures| captures[1].parse::<i64>().ok())
        .unwrap_or(0);

    let real_stream = HlsStream::new(
        id.to_string(),
        host,
        base_with_query,
        extra.to_string(),
        format,
        codec,
        expire,
    );

    Ok(real_stream)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::AtomicUsize;

    use crate::core::{Codec, Format};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    use super::*;

    #[tokio::test]
    async fn test_construct_stream_from_variant() {
        let stream = construct_stream_from_variant(
            "test",
            "https://hs.hls.huya.com/huyalive/156976698-156976698-674209784144068608-314076852-10057-A-0-1.m3u8?ratio=2000&wsSecret=7abc7dec8809146f31f92046eb044e3b&wsTime=68fa41ba&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103",
            Format::TS,
            Codec::Avc,
        ).await.unwrap();
        assert_eq!(stream.index(), "https://hs.hls.huya.com/huyalive/156976698-156976698-674209784144068608-314076852-10057-A-0-1.m3u8?ratio=2000&wsSecret=7abc7dec8809146f31f92046eb044e3b&wsTime=68fa41ba&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103");
        assert_eq!(stream.ts_url("1.ts"), "https://hs.hls.huya.com/huyalive/1.ts?ratio=2000&wsSecret=7abc7dec8809146f31f92046eb044e3b&wsTime=68fa41ba&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103");
        // According to HLS spec (RFC 8216), if segment URI contains query parameters,
        // use them as-is without merging with m3u8 query parameters
        assert_eq!(
            stream.ts_url("1.ts?expires=1760808243"),
            "https://hs.hls.huya.com/huyalive/1.ts?expires=1760808243"
        );
        assert_eq!(stream.host, "https://hs.hls.huya.com");
        assert_eq!(
            stream.base,
            "/huyalive/156976698-156976698-674209784144068608-314076852-10057-A-0-1.m3u8?"
        );
        assert_eq!(stream.extra, "ratio=2000&wsSecret=7abc7dec8809146f31f92046eb044e3b&wsTime=68fa41ba&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103");
        assert_eq!(stream.format, Format::TS);
        assert_eq!(stream.codec, Codec::Avc);
    }

    #[tokio::test]
    async fn construct_stream_reads_youtube_expiry_from_path() {
        let stream = construct_stream_from_variant(
            "youtube_live",
            "https://manifest.googlevideo.com/api/manifest/hls_variant/expire/1790184859/playlist.m3u8?token=abc",
            Format::TS,
            Codec::Avc,
        )
        .await
        .unwrap();
        assert_eq!(stream.expire, 1_790_184_859);
    }

    #[tokio::test]
    async fn first_variant_policy_stays_on_the_initial_media_playlist() {
        let server = MockServer::start().await;
        let first_master = r#"#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=100000,RESOLUTION=640x360
low.m3u8
"#;
        let upgraded_master = r#"#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=100000,RESOLUTION=640x360
low.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=1000000,RESOLUTION=1280x720
high.m3u8
"#;
        let master_requests = Arc::new(AtomicUsize::new(0));
        let request_count = Arc::clone(&master_requests);
        Mock::given(method("GET"))
            .and(path("/live/master.m3u8"))
            .respond_with(move |_request: &Request| {
                let response = if request_count.fetch_add(1, Ordering::Relaxed) == 0 {
                    first_master
                } else {
                    upgraded_master
                };
                ResponseTemplate::new(200).set_body_string(response)
            })
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/live/low.m3u8"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"#EXTM3U
#EXT-X-TARGETDURATION:2
#EXT-X-MEDIA-SEQUENCE:10
#EXTINF:2.0,
low-10.ts
"#,
            ))
            .expect(2)
            .mount(&server)
            .await;

        let stream = construct_stream_from_variant(
            "live",
            &format!("{}/live/master.m3u8", server.uri()),
            Format::TS,
            Codec::Avc,
        )
        .await
        .unwrap();
        let (events, _receiver) = broadcast::channel(1);
        let work_dir =
            std::env::temp_dir().join(format!("bsr-hls-first-variant-{}", uuid::Uuid::new_v4()));
        let recorder = HlsRecorder::new(
            "room".to_string(),
            Arc::new(stream),
            reqwest::Client::new(),
            None,
            events,
            work_dir.clone(),
            Arc::new(AtomicBool::new(true)),
        )
        .await
        .unwrap();

        assert_eq!(
            recorder
                .query_media_playlist()
                .await
                .unwrap()
                .media_sequence,
            10
        );
        assert_eq!(
            recorder
                .query_media_playlist()
                .await
                .unwrap()
                .media_sequence,
            10
        );
        assert_eq!(master_requests.load(Ordering::Relaxed), 1);
        server.verify().await;
        let _ = tokio::fs::remove_dir_all(work_dir).await;
    }

    #[tokio::test]
    async fn highest_bandwidth_change_starts_a_new_recording_segment() {
        let server = MockServer::start().await;
        let first_master = r#"#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=100000,RESOLUTION=640x360
low.m3u8
"#;
        let upgraded_master = r#"#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=100000,RESOLUTION=640x360
low.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=1000000,RESOLUTION=1280x720
high.m3u8
"#;
        let master_requests = Arc::new(AtomicUsize::new(0));
        let request_count = Arc::clone(&master_requests);
        Mock::given(method("GET"))
            .and(path("/live/master.m3u8"))
            .respond_with(move |_request: &Request| {
                let response = if request_count.fetch_add(1, Ordering::Relaxed) == 0 {
                    first_master
                } else {
                    upgraded_master
                };
                ResponseTemplate::new(200).set_body_string(response)
            })
            .expect(3)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/live/low.m3u8"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"#EXTM3U
#EXT-X-TARGETDURATION:2
#EXT-X-MEDIA-SEQUENCE:10
#EXTINF:2.0,
low-10.ts
"#,
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/live/high.m3u8"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"#EXTM3U
#EXT-X-TARGETDURATION:2
#EXT-X-MEDIA-SEQUENCE:100
#EXTINF:2.0,
high-100.ts
"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let stream = construct_stream_from_variant(
            "live",
            &format!("{}/live/master.m3u8", server.uri()),
            Format::TS,
            Codec::Avc,
        )
        .await
        .unwrap();
        let (events, _receiver) = broadcast::channel(1);
        let work_dir =
            std::env::temp_dir().join(format!("bsr-hls-variant-pin-{}", uuid::Uuid::new_v4()));
        let source_stream = Arc::new(stream);
        let recorder = HlsRecorder::new_with_variant_selection(
            "room".to_string(),
            source_stream.clone(),
            reqwest::Client::new(),
            None,
            events,
            work_dir.clone(),
            Arc::new(AtomicBool::new(true)),
            HlsVariantSelection::HighestBandwidth,
        )
        .await
        .unwrap();

        let first = recorder.query_media_playlist().await.unwrap();
        let second = recorder.query_media_playlist().await;

        assert_eq!(first.media_sequence, 10);
        assert!(matches!(
            second,
            Err(RecorderError::ResolutionChanged { .. })
        ));

        // A fresh recording segment re-reads the upgraded master and starts at
        // the now-highest available variant.
        let (next_events, _next_receiver) = broadcast::channel(1);
        let next_recorder = HlsRecorder::new_with_variant_selection(
            "room".to_string(),
            source_stream,
            reqwest::Client::new(),
            None,
            next_events,
            work_dir.clone(),
            Arc::new(AtomicBool::new(true)),
            HlsVariantSelection::HighestBandwidth,
        )
        .await
        .unwrap();
        assert_eq!(
            next_recorder
                .query_media_playlist()
                .await
                .unwrap()
                .media_sequence,
            100
        );
        assert_eq!(master_requests.load(Ordering::Relaxed), 3);
        server.verify().await;
        let _ = tokio::fs::remove_dir_all(work_dir).await;
    }

    #[tokio::test]
    async fn highest_bandwidth_policy_uses_pinned_playlist_when_master_fails() {
        let server = MockServer::start().await;
        let master_body = r#"#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=100000,RESOLUTION=640x360
low.m3u8
"#;
        let master_requests = Arc::new(AtomicUsize::new(0));
        let request_count = Arc::clone(&master_requests);
        Mock::given(method("GET"))
            .and(path("/live/master.m3u8"))
            .respond_with(move |_request: &Request| {
                if request_count.fetch_add(1, Ordering::Relaxed) == 0 {
                    ResponseTemplate::new(200).set_body_string(master_body)
                } else {
                    ResponseTemplate::new(500).set_body_string("temporary master failure")
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let media_requests = Arc::new(AtomicUsize::new(0));
        let request_count = Arc::clone(&media_requests);
        Mock::given(method("GET"))
            .and(path("/live/low.m3u8"))
            .respond_with(move |_request: &Request| {
                let sequence = if request_count.fetch_add(1, Ordering::Relaxed) == 0 {
                    10
                } else {
                    11
                };
                ResponseTemplate::new(200).set_body_string(format!(
                    r#"#EXTM3U
#EXT-X-TARGETDURATION:2
#EXT-X-MEDIA-SEQUENCE:{sequence}
#EXTINF:2.0,
low-{sequence}.ts
"#
                ))
            })
            .expect(2)
            .mount(&server)
            .await;

        let stream = construct_stream_from_variant(
            "live",
            &format!("{}/live/master.m3u8", server.uri()),
            Format::TS,
            Codec::Avc,
        )
        .await
        .unwrap();
        let (events, _receiver) = broadcast::channel(1);
        let work_dir =
            std::env::temp_dir().join(format!("bsr-hls-master-failure-{}", uuid::Uuid::new_v4()));
        let recorder = HlsRecorder::new_with_variant_selection(
            "room".to_string(),
            Arc::new(stream),
            reqwest::Client::new(),
            None,
            events,
            work_dir.clone(),
            Arc::new(AtomicBool::new(true)),
            HlsVariantSelection::HighestBandwidth,
        )
        .await
        .unwrap();

        assert_eq!(
            recorder
                .query_media_playlist()
                .await
                .unwrap()
                .media_sequence,
            10
        );
        assert_eq!(
            recorder
                .query_media_playlist()
                .await
                .unwrap()
                .media_sequence,
            11
        );
        assert_eq!(master_requests.load(Ordering::Relaxed), 2);
        assert_eq!(media_requests.load(Ordering::Relaxed), 2);
        server.verify().await;
        let _ = tokio::fs::remove_dir_all(work_dir).await;
    }

    #[tokio::test]
    async fn reselects_variant_only_after_pinned_variant_disappears() {
        let server = MockServer::start().await;
        let initial_master = r#"#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=100000,RESOLUTION=640x360
low.m3u8
"#;
        let updated_master = r#"#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=1000000,RESOLUTION=1280x720
high.m3u8
"#;
        let master_requests = Arc::new(AtomicUsize::new(0));
        let request_count = Arc::clone(&master_requests);
        Mock::given(method("GET"))
            .and(path("/live/master.m3u8"))
            .respond_with(move |_request: &Request| {
                let response = if request_count.fetch_add(1, Ordering::Relaxed) == 0 {
                    initial_master
                } else {
                    updated_master
                };
                ResponseTemplate::new(200).set_body_string(response)
            })
            .expect(2)
            .mount(&server)
            .await;
        let low_requests = Arc::new(AtomicUsize::new(0));
        let request_count = Arc::clone(&low_requests);
        Mock::given(method("GET"))
            .and(path("/live/low.m3u8"))
            .respond_with(move |_request: &Request| {
                if request_count.fetch_add(1, Ordering::Relaxed) == 0 {
                    ResponseTemplate::new(200).set_body_string(
                        r#"#EXTM3U
#EXT-X-TARGETDURATION:2
#EXT-X-MEDIA-SEQUENCE:10
#EXTINF:2.0,
low-10.ts
"#,
                    )
                } else {
                    ResponseTemplate::new(404)
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/live/high.m3u8"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"#EXTM3U
#EXT-X-TARGETDURATION:2
#EXT-X-MEDIA-SEQUENCE:100
#EXTINF:2.0,
high-100.ts
"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let stream = construct_stream_from_variant(
            "live",
            &format!("{}/live/master.m3u8", server.uri()),
            Format::TS,
            Codec::Avc,
        )
        .await
        .unwrap();
        let (events, _receiver) = broadcast::channel(1);
        let work_dir =
            std::env::temp_dir().join(format!("bsr-hls-variant-fallback-{}", uuid::Uuid::new_v4()));
        let recorder = HlsRecorder::new(
            "room".to_string(),
            Arc::new(stream),
            reqwest::Client::new(),
            None,
            events,
            work_dir.clone(),
            Arc::new(AtomicBool::new(true)),
        )
        .await
        .unwrap();

        assert_eq!(
            recorder
                .query_media_playlist()
                .await
                .unwrap()
                .media_sequence,
            10
        );
        assert_eq!(
            recorder
                .query_media_playlist()
                .await
                .unwrap()
                .media_sequence,
            100
        );
        server.verify().await;
        let _ = tokio::fs::remove_dir_all(work_dir).await;
    }

    #[test]
    fn absolute_segment_urls_get_unique_local_names() {
        assert_eq!(
            local_segment_filename(
                1528,
                "https://rr5.googlevideo.com/videoplayback?itag=91&sig=secret"
            ),
            "segment-1528.ts"
        );
        assert_eq!(
            local_segment_filename(1529, "https://cdn.test/chunk-1.m4s?token=secret"),
            "segment-1529.m4s"
        );
        assert_eq!(
            local_segment_filename(3, "nested/3.ts?expires=1"),
            "nested/3.ts"
        );
    }

    #[tokio::test]
    async fn persist_sequence_returns_write_errors() {
        let path = std::env::temp_dir().join(format!(
            "bili-shadowreplay-{}.sequence",
            uuid::Uuid::new_v4()
        ));
        fs::write(&path, "0").unwrap();

        let read_only_file = fs::File::open(&path).unwrap();
        let mut read_only_file = File::from_std(read_only_file);
        let result = persist_sequence(&mut read_only_file, 1).await;

        assert!(result.is_err());
        drop(read_only_file);
        fs::remove_file(path).unwrap();
    }
}
