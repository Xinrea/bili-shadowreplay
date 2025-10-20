use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::{path::PathBuf, sync::Arc};

use m3u8_rs::{MediaPlaylist, Playlist};
use reqwest::header::HeaderMap;
use std::time::Duration;
use tokio::fs::File;
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::core::playlist::HlsPlaylist;
use crate::core::{Codec, Format};
use crate::errors::RecorderError;
use crate::ffmpeg::VideoMetadata;
use crate::{core::HlsStream, events::RecorderEvent};

const UPDATE_TIMEOUT: Duration = Duration::from_secs(10);
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
    stream: Arc<HlsStream>,
    client: reqwest::Client,
    event_channel: broadcast::Sender<RecorderEvent>,
    work_dir: PathBuf,
    playlist: Arc<Mutex<HlsPlaylist>>,
    headers: HeaderMap,

    quit: Arc<AtomicBool>,

    sequence: Arc<AtomicU64>,
    updated_at: Arc<AtomicI64>,

    cached_duration_secs: Arc<AtomicU64>,
    cached_size_bytes: Arc<AtomicU64>,

    pre_metadata: Arc<RwLock<Option<VideoMetadata>>>,
}

impl HlsRecorder {
    pub async fn new(
        stream: Arc<HlsStream>,
        client: reqwest::Client,
        event_channel: broadcast::Sender<RecorderEvent>,
        work_dir: PathBuf,
    ) -> Self {
        // try to create work_dir
        if !work_dir.exists() {
            std::fs::create_dir_all(&work_dir).unwrap();
        }
        let playlist_path = work_dir.join(PLAYLIST_FILE_NAME);

        // set user agent
        let user_agent = crate::user_agent_generator::UserAgentGenerator::new().generate();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("user-agent", user_agent.parse().unwrap());
        Self {
            stream,
            client,
            event_channel,
            work_dir,
            playlist: Arc::new(Mutex::new(HlsPlaylist::new(playlist_path).await)),
            headers,
            quit: Arc::new(AtomicBool::new(false)),
            sequence: Arc::new(AtomicU64::new(0)),
            updated_at: Arc::new(AtomicI64::new(chrono::Utc::now().timestamp_millis())),
            cached_duration_secs: Arc::new(AtomicU64::new(0)),
            cached_size_bytes: Arc::new(AtomicU64::new(0)),
            pre_metadata: Arc::new(RwLock::new(None)),
        }
    }

    /// Start the recorder blockingly
    ///
    /// This will start the recorder and update the entries periodically.
    pub async fn start(&self) -> Result<(), RecorderError> {
        while !self.quit.load(Ordering::Relaxed) {
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
                    _ => {
                        // Other errors are not critical, just log it
                        log::error!("Update entries error: {}", e);
                    }
                }
            }

            tokio::time::sleep(UPDATE_INTERVAL).await;
        }

        Ok(())
    }

    pub async fn stop(&self) {
        self.quit.store(true, Ordering::Relaxed);
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
            m3u8_rs::parse_playlist(&bytes).map_err(|e| RecorderError::M3u8ParseFailed {
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
        for (i, segment) in media_playlist.segments.iter().enumerate() {
            let segment_sequence = playlist_sequence + i as u64;
            if segment_sequence <= last_sequence {
                continue;
            }

            let segment_full_url = self.stream.ts_url(&segment.uri);
            // to get filename, we need to remove the query parameters
            // for example: 1.ts?expires=1760808243
            // we need to remove the query parameters: 1.ts
            let filename = segment.uri.split('?').next().unwrap_or(&segment.uri);
            let segment_path = self.work_dir.join(filename);
            let size = download(
                &self.client,
                &segment_full_url,
                &segment_path,
                DOWNLOAD_RETRY,
            )
            .await?;

            // check if the stream is changed
            let segment_metadata = crate::ffmpeg::extract_video_metadata(&segment_path)
                .await
                .map_err(RecorderError::FfmpegError)?;

            // if resolution is in metadata
            if !segment_metadata.seems_corrupted() {
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
            }

            let mut new_segment = segment.clone();
            new_segment.duration = segment_metadata.duration as f32;

            self.playlist.lock().await.add_segment(new_segment).await?;

            self.cached_duration_secs
                .fetch_add(segment_metadata.duration as u64, Ordering::Relaxed);
            self.cached_size_bytes.fetch_add(size, Ordering::Relaxed);
            self.sequence.store(segment_sequence, Ordering::Relaxed);
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
                duration_secs: self.cached_duration_secs.load(Ordering::Relaxed),
                cached_size_bytes: self.cached_size_bytes.load(Ordering::Relaxed),
            });
        }

        Ok(())
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

async fn construct_stream_from_variant(
    id: &str,
    variant_url: &str,
    format: Format,
    codec: Codec,
) -> Result<HlsStream, RecorderError> {
    // construct the real stream from variant
    // example: https://cn-jsnt-ct-01-07.bilivideo.com/live-bvc/930889/live_2124647716_1414766_bluray/index.m3u8?expires=1760808243
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

    let real_stream = HlsStream::new(
        id.to_string(),
        host,
        base_with_query,
        extra.to_string(),
        format,
        codec,
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
            "https://cn-jsnt-ct-01-07.bilivideo.com/live-bvc/930889/live_2124647716_1414766_bluray/index.m3u8?expires=1760808243",
            Format::TS,
            Codec::Avc,
        ).await.unwrap();
        assert_eq!(stream.index(), "https://cn-jsnt-ct-01-07.bilivideo.com/live-bvc/930889/live_2124647716_1414766_bluray/index.m3u8?expires=1760808243");
        assert_eq!(stream.ts_url("1.ts"), "https://cn-jsnt-ct-01-07.bilivideo.com/live-bvc/930889/live_2124647716_1414766_bluray/1.ts?expires=1760808243");
        assert_eq!(stream.host, "https://cn-jsnt-ct-01-07.bilivideo.com");
        assert_eq!(
            stream.base,
            "/live-bvc/930889/live_2124647716_1414766_bluray/index.m3u8?"
        );
        assert_eq!(stream.extra, "expires=1760808243");
        assert_eq!(stream.format, Format::TS);
        assert_eq!(stream.codec, Codec::Avc);
    }
}
