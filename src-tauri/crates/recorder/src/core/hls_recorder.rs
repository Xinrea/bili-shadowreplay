use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::{path::PathBuf, sync::Arc};

use chrono::Utc;
use m3u8_rs::{MediaPlaylist, Playlist};
use reqwest::header::HeaderMap;
use std::time::Duration;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::core::playlist::HlsPlaylist;
use crate::core::{Codec, Format};
use crate::errors::RecorderError;
use crate::ffmpeg::VideoMetadata;
use crate::{core::HlsStream, events::RecorderEvent};

const UPDATE_TIMEOUT: Duration = Duration::from_secs(20);
const UPDATE_INTERVAL: Duration = Duration::from_secs(1);
const PLAYLIST_FILE_NAME: &str = "playlist.m3u8";
const DOWNLOAD_RETRY: u32 = 3;
/// A recorder for HLS streams
///
/// This recorder fetches, caches and serves TS entries, currently supporting `StreamType::FMP4, StreamType::TS`.
///
/// Segments will be downloaded to work_dir, and `playlist.m3u8` will be generated in work_dir.
#[derive(Clone)]
pub struct HlsRecorder {
    room_id: String,
    stream: Arc<HlsStream>,
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
        // try to create work_dir
        if !work_dir.exists() {
            std::fs::create_dir_all(&work_dir)?;
        }
        let playlist_path = work_dir.join(PLAYLIST_FILE_NAME);

        // set user agent
        let user_agent =
            crate::utils::user_agent_generator::UserAgentGenerator::new().generate(false);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("user-agent", user_agent.parse().unwrap());
        if let Some(cookies) = cookies {
            headers.insert("cookie", cookies.parse().unwrap());
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
        let sequence = trimmed
            .parse::<u64>()
            .map_err(|_| RecorderError::InvalidValue)?;

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

        Ok(Self {
            room_id,
            stream,
            client,
            event_channel,
            work_dir,
            playlist: Arc::new(Mutex::new(HlsPlaylist::new(playlist_path).await)),
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
                content: String::from_utf8(bytes.to_vec()).unwrap(),
            })?;
        Ok(playlist)
    }

    async fn query_media_playlist(&self) -> Result<MediaPlaylist, RecorderError> {
        let playlist = self.query_playlist(&self.stream).await?;
        match playlist {
            Playlist::MediaPlaylist(playlist) => Ok(playlist),
            Playlist::MasterPlaylist(playlist) => {
                // just return the first variant
                match playlist.variants.first() {
                    Some(variant) => {
                        let real_stream = construct_stream_from_variant(
                            &self.stream.id,
                            &variant.uri,
                            self.stream.format.clone(),
                            self.stream.codec.clone(),
                        )
                        .await?;
                        let playlist = self.query_playlist(&real_stream).await?;
                        match playlist {
                            Playlist::MediaPlaylist(playlist) => Ok(playlist),
                            Playlist::MasterPlaylist(_) => Err(RecorderError::M3u8ParseFailed {
                                content: "No media playlist found".to_string(),
                            }),
                        }
                    }
                    None => Err(RecorderError::M3u8ParseFailed {
                        content: "No variants found".to_string(),
                    }),
                }
            }
        }
    }

    async fn update_entries(&self) -> Result<(), RecorderError> {
        let media_playlist = self.query_media_playlist().await?;
        let playlist_sequence = media_playlist.media_sequence;
        let last_sequence = self.sequence.load(Ordering::Relaxed);
        let last_metadata = self.pre_metadata.read().await.clone();
        let mut updated = false;
        let mut duration_delta = 0.0;
        let mut size_delta = 0;
        for (i, segment) in media_playlist.segments.iter().enumerate() {
            let segment_sequence = playlist_sequence + i as u64;
            let segment_full_url = self.stream.ts_url(&segment.uri);
            // to get filename, we need to remove the query parameters
            // for example: 1.ts?expires=1760808243
            // we need to remove the query parameters: 1.ts
            let filename = segment.uri.split('?').next().unwrap_or(&segment.uri);
            if segment_sequence <= last_sequence {
                continue;
            }

            let segment_path = self.work_dir.join(filename);
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
            if segment.program_date_time.is_none() {
                segment.program_date_time.replace(Utc::now().into());
            }

            // check if the stream is changed
            let segment_metadata = crate::ffmpeg::extract_video_metadata(&segment_path)
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

                let last_segment = playlist.last_segment().await;
                let last_segment_uri = last_segment.unwrap().uri.clone();
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
                self.update_sequence(segment_sequence).await;
                self.updated_at
                    .store(chrono::Utc::now().timestamp_millis(), Ordering::Relaxed);
                updated = true;
                continue;
            }

            if let Some(last_metadata) = &last_metadata {
                if last_metadata != &segment_metadata {
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
            self.update_sequence(segment_sequence).await;
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

    async fn update_sequence(&self, sequence: u64) {
        self.sequence.store(sequence, Ordering::Relaxed);
        // write to file
        let mut file = self.sequence_file.write().await;
        file.set_len(0).await.unwrap();
        file.seek(SeekFrom::Start(0)).await.unwrap();
        file.write_all(sequence.to_string().as_bytes())
            .await
            .unwrap();
        let _ = file.flush().await;
    }
}

/// Download url content into fpath
async fn download_inner(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
) -> Result<u64, RecorderError> {
    if !path.parent().unwrap().exists() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
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
    let expire_regex = regex::Regex::new(r"expires=(\d+)").unwrap();
    let expire = if let Some(captures) = expire_regex.captures(extra) {
        captures[1].parse::<i64>().unwrap_or(0)
    } else {
        0
    };

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
    use crate::core::{Codec, Format};

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
}
