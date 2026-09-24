use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::{path::PathBuf, sync::Arc};

use chrono::Utc;
use m3u8_rs::{DateRange, MediaPlaylist, MediaSegment, Playlist, QuotedOrUnquoted, VariantStream};
use reqwest::header::HeaderMap;
use std::time::Duration;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::core::playlist::{playlist_content_preview, HlsPlaylist};
use crate::core::{Codec, Format};
use crate::errors::RecorderError;
use crate::timeline::{append_skipped_ad_range, load_skipped_ad_ranges, AdTimeRange};
use crate::{core::HlsStream, events::RecorderEvent};
use ffmpeg_utils::{extract_video_metadata, VideoMetadata};

const UPDATE_TIMEOUT: Duration = Duration::from_secs(20);
const UPDATE_INTERVAL: Duration = Duration::from_secs(1);
const PLAYLIST_FILE_NAME: &str = "playlist.m3u8";
const DOWNLOAD_RETRY: u32 = 3;
/// Upper bound on the ad windows remembered across playlist reloads.
const MAX_AD_RANGES: usize = 128;

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
    ad_ranges: Arc<Mutex<Vec<AdTimeRange>>>,
    skipped_ad_ranges: Arc<Mutex<Vec<AdTimeRange>>>,
}

struct DownloadedSegment {
    segment: MediaSegment,
    path: PathBuf,
    size: u64,
}

fn is_twitch_ad_range(range: &DateRange) -> bool {
    range.id.to_ascii_lowercase().starts_with("stitched-ad-")
        || range
            .class
            .as_deref()
            .is_some_and(|class| class.eq_ignore_ascii_case("twitch-stitched-ad"))
        || range.x_prefixed.as_ref().is_some_and(|attributes| {
            attributes
                .keys()
                .any(|key| key.starts_with("X-TV-TWITCH-AD-"))
        })
}

fn twitch_ad_time_range(range: &DateRange) -> Option<AdTimeRange> {
    if !is_twitch_ad_range(range) {
        return None;
    }

    let start_ms = range.start_date.timestamp_millis();
    let end_ms = range
        .end_date
        .as_ref()
        .map(|end_date| end_date.timestamp_millis())
        .or_else(|| {
            // A stitched ad that is still playing has no `DURATION` yet; Twitch
            // announces its length as `PLANNED-DURATION` instead.
            range
                .duration
                .or(range.planned_duration)
                .filter(|duration| duration.is_finite() && *duration > 0.0)
                .map(|duration| start_ms.saturating_add((duration * 1000.0).ceil() as i64))
        })?;
    (end_ms > start_ms).then(|| AdTimeRange {
        id: range.id.clone(),
        start_ms,
        end_ms,
    })
}

fn is_twitch_ad_segment(segment: &MediaSegment, ad_ranges: &[AdTimeRange]) -> bool {
    if segment.daterange.as_ref().is_some_and(is_twitch_ad_range) {
        return true;
    }

    let Some(segment_time) = segment
        .program_date_time
        .as_ref()
        .map(|date_time| date_time.timestamp_millis())
    else {
        return false;
    };
    ad_ranges
        .iter()
        .any(|range| segment_time >= range.start_ms && segment_time < range.end_ms)
}

fn ad_range_for_segment(segment: &MediaSegment, ad_ranges: &[AdTimeRange]) -> Option<AdTimeRange> {
    let segment_time = segment
        .program_date_time
        .as_ref()
        .map(|date_time| date_time.timestamp_millis());
    segment_time
        .and_then(|segment_time| {
            ad_ranges
                .iter()
                .find(|range| segment_time >= range.start_ms && segment_time < range.end_ms)
                .cloned()
        })
        .or_else(|| segment.daterange.as_ref().and_then(twitch_ad_time_range))
}

fn upsert_ad_range(ranges: &mut Vec<AdTimeRange>, range: &AdTimeRange) -> bool {
    if let Some(existing) = ranges
        .iter_mut()
        .find(|existing| existing.id == range.id && existing.start_ms == range.start_ms)
    {
        if existing == range {
            return false;
        }
        *existing = range.clone();
        true
    } else {
        ranges.push(range.clone());
        true
    }
}

/// Split an HLS attribute list into its `key=value` attributes.
///
/// Quoted values may contain commas, so only a comma outside a quoted string
/// separates two attributes.
fn date_range_attributes(list: &str) -> HashMap<String, QuotedOrUnquoted> {
    let mut attributes = HashMap::new();
    let mut remainder = list;
    while !remainder.is_empty() {
        let mut quoted = false;
        let mut separator = remainder.len();
        for (index, character) in remainder.char_indices() {
            match character {
                '"' => quoted = !quoted,
                ',' if !quoted => {
                    separator = index;
                    break;
                }
                _ => {}
            }
        }
        let attribute = &remainder[..separator];
        // Step over the separator; an empty tail ends the loop.
        remainder = remainder[separator..].strip_prefix(',').unwrap_or("");

        let Some((key, value)) = attribute.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let value = value.trim();
        let parsed = if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
            QuotedOrUnquoted::Quoted(value[1..value.len() - 1].to_string())
        } else {
            QuotedOrUnquoted::Unquoted(value.to_string())
        };
        attributes.insert(key.to_string(), parsed);
    }
    attributes
}

/// Collect the Twitch ad windows from every `#EXT-X-DATERANGE` in a playlist.
///
/// `m3u8-rs` attaches a range to the segment that follows it and overwrites it
/// when several ranges appear in a row. Twitch writes `playlist-session`,
/// `stitched-ad` and `stream-source` together at the head of each weaver reload,
/// so the ad window is gone by the time the playlist has been parsed. Reading the
/// raw text keeps all of them.
fn raw_twitch_ad_ranges(content: &[u8]) -> Vec<AdTimeRange> {
    String::from_utf8_lossy(content)
        .lines()
        .filter_map(|line| line.strip_prefix("#EXT-X-DATERANGE:"))
        .filter_map(|attributes| {
            DateRange::from_hashmap(date_range_attributes(attributes))
                .ok()
                .and_then(|range| twitch_ad_time_range(&range))
        })
        .collect()
}

fn local_segment_filename(sequence: u64, uri: &str) -> String {
    let path = uri.split(['?', '#']).next().unwrap_or(uri);
    let basename = path.rsplit('/').next().unwrap_or(path);
    let extension = basename
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| {
            !extension.is_empty()
                && extension.len() <= 8
                && extension
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
        })
        .unwrap_or("ts");

    // Segment URIs can repeat while their query strings change. Use the media
    // sequence for every local name so later downloads never overwrite earlier
    // playlist entries.
    format!("segment-{sequence}.{extension}")
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

        let skipped_ad_ranges = match load_skipped_ad_ranges(&work_dir).await {
            Ok(ranges) => ranges,
            Err(error) => {
                log::warn!("Failed to load skipped Twitch ad ranges: {error}");
                Vec::new()
            }
        };

        let mut playlist = HlsPlaylist::new(playlist_path).await?;
        playlist.reopen().await?;
        // A resumed archive must retain the media shape of its last playable
        // segment; otherwise the first ad segment after a restart could become
        // the new baseline and cause the actual stream to be skipped.
        let last_segment_uri = playlist
            .last_segment()
            .await
            .map(|segment| segment.uri.clone());
        let pre_metadata = if let Some(last_segment_uri) = last_segment_uri {
            let path = work_dir.join(last_segment_uri);
            match extract_video_metadata(&path).await {
                Ok(metadata) if !metadata.seems_corrupted() => Some(metadata),
                _ => None,
            }
        } else {
            None
        };

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
            pre_metadata: Arc::new(RwLock::new(pre_metadata)),
            ad_ranges: Arc::new(Mutex::new(Vec::new())),
            skipped_ad_ranges: Arc::new(Mutex::new(skipped_ad_ranges)),
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
        self.observe_ad_ranges(&bytes).await?;
        let (_, playlist) =
            m3u8_rs::parse_playlist(&bytes).map_err(|_| RecorderError::M3u8ParseFailed {
                content: playlist_content_preview(&bytes),
            })?;
        Ok(playlist)
    }

    /// Remember the Twitch ad windows announced by a freshly fetched playlist.
    ///
    /// Ranges are accumulated because a reload only carries the ones that are
    /// still current, while ad segments keep arriving afterwards.
    async fn observe_ad_ranges(&self, content: &[u8]) -> Result<(), RecorderError> {
        for range in raw_twitch_ad_ranges(content) {
            {
                let mut ad_ranges = self.ad_ranges.lock().await;
                upsert_ad_range(&mut ad_ranges, &range);
            }

            // An ad range can be announced first with PLANNED-DURATION and later
            // updated with its final END-DATE. Keep the persisted time map in
            // sync, but only for ads that have actually caused a segment skip.
            let mut skipped_ranges = self.skipped_ad_ranges.lock().await;
            if skipped_ranges.iter().any(|skipped| {
                skipped.id == range.id && skipped.start_ms == range.start_ms && skipped != &range
            }) {
                append_skipped_ad_range(&self.work_dir, &range).await?;
                upsert_ad_range(&mut skipped_ranges, &range);
            }
        }
        Ok(())
    }

    async fn persist_skipped_ad_range(&self, range: &AdTimeRange) -> Result<(), RecorderError> {
        let mut skipped_ranges = self.skipped_ad_ranges.lock().await;
        let changed = skipped_ranges
            .iter()
            .find(|skipped| skipped.id == range.id && skipped.start_ms == range.start_ms)
            .is_none_or(|skipped| skipped != range);
        if changed {
            append_skipped_ad_range(&self.work_dir, range).await?;
            upsert_ad_range(&mut skipped_ranges, range);
        }
        Ok(())
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

    async fn skip_segment(&self, segment_path: &Path, sequence: u64) -> Result<(), RecorderError> {
        match tokio::fs::remove_file(segment_path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(RecorderError::IoError(error)),
        }
        self.update_sequence(sequence).await?;
        self.updated_at
            .store(Utc::now().timestamp_millis(), Ordering::Relaxed);
        Ok(())
    }

    /// Drop the accumulated ad windows that can no longer match a segment of
    /// this playlist and return the rest.
    async fn ad_ranges_for_playlist(&self, playlist: &MediaPlaylist) -> Vec<AdTimeRange> {
        let first_segment_time = playlist
            .segments
            .iter()
            .find_map(|segment| segment.program_date_time.as_ref())
            .map(|date_time| date_time.timestamp_millis());
        let mut ad_ranges = self.ad_ranges.lock().await;
        if let Some(first_segment_time) = first_segment_time {
            ad_ranges.retain(|range| range.end_ms > first_segment_time);
        }
        if ad_ranges.len() > MAX_AD_RANGES {
            let excess = ad_ranges.len() - MAX_AD_RANGES;
            ad_ranges.drain(..excess);
        }
        ad_ranges.clone()
    }

    async fn download_segment(
        &self,
        source_stream: &HlsStream,
        source_segment: &MediaSegment,
        sequence: u64,
    ) -> Result<DownloadedSegment, RecorderError> {
        let mut segment = source_segment.clone();
        let source_url = source_stream.ts_url(&segment.uri);
        let local_uri = local_segment_filename(sequence, &segment.uri);
        let path = self.work_dir.join(&local_uri);
        let size = download(&self.client, &source_url, &path, DOWNLOAD_RETRY).await?;

        segment.uri = local_uri;
        Ok(DownloadedSegment {
            segment,
            path,
            size,
        })
    }

    async fn download_if_not_twitch_ad(
        &self,
        source_stream: &HlsStream,
        segment: &MediaSegment,
        sequence: u64,
        ad_ranges: &[AdTimeRange],
    ) -> Result<Option<DownloadedSegment>, RecorderError> {
        if is_twitch_ad_segment(segment, ad_ranges) {
            log::info!("Skipping Twitch ad segment at sequence {sequence}");
            if let Some(range) = ad_range_for_segment(segment, ad_ranges) {
                self.persist_skipped_ad_range(&range).await?;
            }
            let path = self
                .work_dir
                .join(local_segment_filename(sequence, &segment.uri));
            self.skip_segment(&path, sequence).await?;
            return Ok(None);
        }

        self.download_segment(source_stream, segment, sequence)
            .await
            .map(Some)
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
        let ad_ranges = self.ad_ranges_for_playlist(&media_playlist).await;
        let mut last_metadata = self.pre_metadata.read().await.clone();
        let mut updated = false;
        let mut duration_delta = 0.0;
        let mut size_delta = 0;
        for (i, segment) in media_playlist.segments.iter().enumerate() {
            let segment_sequence = playlist_sequence + i as u64;
            if segment_sequence <= last_sequence {
                continue;
            }

            let downloaded = match self
                .download_if_not_twitch_ad(&selected_stream, segment, segment_sequence, &ad_ranges)
                .await
            {
                Ok(Some(downloaded)) => downloaded,
                Ok(None) => continue,
                Err(error) => {
                    log::error!(
                        "Failed to download HLS segment at sequence {segment_sequence}: {error}"
                    );
                    return Err(error);
                }
            };
            let segment_path = downloaded.path;
            let size = downloaded.size;
            let mut segment = downloaded.segment;
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

            if let Some(previous_metadata) = &last_metadata {
                // Only a different resolution or codec makes the segments
                // unplayable as one recording; their length naturally differs
                // from segment to segment.
                if !previous_metadata.same_stream_shape(&segment_metadata) {
                    return Err(RecorderError::ResolutionChanged {
                        err: "Resolution changed".to_string(),
                    });
                }
            } else {
                let metadata = segment_metadata.clone();
                *self.pre_metadata.write().await = Some(metadata.clone());
                last_metadata = Some(metadata);
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
        log::warn!("Download segment failed: {status}");
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
    async fn rewrites_long_absolute_segment_uris_to_local_playlist_names() {
        let server = MockServer::start().await;
        let body = b"mock Twitch media segment";
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.to_vec()))
            .mount(&server)
            .await;

        let long_token = "x".repeat(600);
        let source_uri = format!(
            "{}/v1/segment/{long_token}.ts?token={}",
            server.uri(),
            "y".repeat(300)
        );
        let source_filename = source_uri
            .split('?')
            .next()
            .unwrap()
            .rsplit('/')
            .next()
            .unwrap();
        assert!(source_filename.len() > 255);

        let (event_tx, _) = broadcast::channel(1);
        let work_dir = std::env::temp_dir().join(format!(
            "bili-shadowreplay-hls-long-uri-{}",
            uuid::Uuid::new_v4()
        ));
        let stream = Arc::new(HlsStream::new(
            "twitch-live".to_string(),
            server.uri(),
            "/master.m3u8".to_string(),
            String::new(),
            Format::TS,
            Codec::Avc,
            0,
        ));
        let recorder = HlsRecorder::new(
            "twitch-channel".to_string(),
            stream,
            reqwest::Client::new(),
            None,
            event_tx,
            work_dir.clone(),
            Arc::new(AtomicBool::new(true)),
        )
        .await
        .unwrap();
        let source_playlist_content =
            format!("#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.0,\n{source_uri}\n");
        let (_, source_playlist) = m3u8_rs::parse_playlist(source_playlist_content.as_bytes())
            .expect("signed Twitch media playlist should parse");
        let Playlist::MediaPlaylist(source_playlist) = source_playlist else {
            panic!("expected Twitch media playlist");
        };

        let downloaded = recorder
            .download_segment(&recorder.stream, &source_playlist.segments[0], 42)
            .await
            .unwrap();
        assert_eq!(downloaded.segment.uri, "segment-42.ts");
        assert_eq!(
            downloaded.path.file_name().unwrap().to_str().unwrap(),
            "segment-42.ts"
        );
        assert_eq!(tokio::fs::read(&downloaded.path).await.unwrap(), body);

        recorder
            .playlist
            .lock()
            .await
            .add_segment(downloaded.segment)
            .await
            .unwrap();
        let playlist = tokio::fs::read_to_string(work_dir.join(PLAYLIST_FILE_NAME))
            .await
            .unwrap();
        assert!(playlist.contains("segment-42.ts"));
        assert!(!playlist.contains("https://"));
        assert!(!playlist.contains(&long_token));

        tokio::fs::remove_dir_all(work_dir).await.unwrap();
    }

    /// Build a recorder whose index URL serves the mounted playlist.
    async fn twitch_recorder(server: &MockServer, label: &str) -> (HlsRecorder, PathBuf) {
        let (event_tx, _) = broadcast::channel(1);
        let work_dir = std::env::temp_dir().join(format!(
            "bili-shadowreplay-hls-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        let stream = Arc::new(HlsStream::new(
            "twitch-live".to_string(),
            server.uri(),
            "/master.m3u8".to_string(),
            String::new(),
            Format::TS,
            Codec::Avc,
            0,
        ));
        let recorder = HlsRecorder::new(
            "twitch-channel".to_string(),
            stream,
            reqwest::Client::new(),
            None,
            event_tx,
            work_dir.clone(),
            Arc::new(AtomicBool::new(true)),
        )
        .await
        .unwrap();
        (recorder, work_dir)
    }

    /// Serve `playlist` at the index URL and answer the program segment with a
    /// stub body. The ad segments must never be requested, which `server.verify()`
    /// asserts afterwards.
    async fn mount_twitch_playlist(server: &MockServer, playlist: String) {
        Mock::given(method("GET"))
            .and(path("/master.m3u8"))
            .respond_with(ResponseTemplate::new(200).set_body_string(playlist))
            .expect(1)
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/live/segment-1080p.ts"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"mock ts segment".to_vec()))
            .mount(server)
            .await;
        for ad_path in ["/ads/segment-480p-1.ts", "/ads/segment-480p-2.ts"] {
            Mock::given(method("GET"))
                .and(path(ad_path))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ad".to_vec()))
                .expect(0)
                .mount(server)
                .await;
        }
    }

    /// Fetch the playlist once and record every segment the ad filter keeps.
    async fn record_playlist_once(recorder: &HlsRecorder) -> String {
        let playlist = recorder.query_media_playlist().await.unwrap();
        let ad_ranges = recorder.ad_ranges_for_playlist(&playlist).await;
        for (i, segment) in playlist.segments.iter().enumerate() {
            let sequence = i as u64;
            let Some(downloaded) = recorder
                .download_if_not_twitch_ad(&recorder.stream, segment, sequence, &ad_ranges)
                .await
                .unwrap()
            else {
                continue;
            };
            recorder
                .playlist
                .lock()
                .await
                .add_segment(downloaded.segment)
                .await
                .unwrap();
            recorder.update_sequence(sequence).await.unwrap();
        }
        tokio::fs::read_to_string(recorder.work_dir.join(PLAYLIST_FILE_NAME))
            .await
            .unwrap()
    }

    /// Assert that only the 1080p program survived the ad filter.
    fn assert_only_program_recorded(saved: &str, work_dir: &Path) {
        assert!(saved.contains("segment-2.ts"));
        assert!(saved.contains("1080p program"));
        assert!(!saved.contains("480p ad"));
        assert!(!saved.contains("https://"));
        assert!(!work_dir.join("segment-0.ts").exists());
        assert!(!work_dir.join("segment-1.ts").exists());
        assert!(work_dir.join("segment-2.ts").exists());
    }

    #[tokio::test]
    async fn twitch_stitched_ad_range_skips_ad_playlist_segments_but_records_program() {
        let server = MockServer::start().await;
        let ad_first = format!("{}/ads/segment-480p-1.ts", server.uri());
        let ad_second = format!("{}/ads/segment-480p-2.ts", server.uri());
        let program = format!("{}/live/segment-1080p.ts", server.uri());
        mount_twitch_playlist(
            &server,
            format!(
                r#"#EXTM3U
#EXT-X-TARGETDURATION:2
#EXT-X-DATERANGE:ID="stitched-ad-test",CLASS="twitch-stitched-ad",START-DATE="2026-01-01T00:00:00Z",DURATION=4.0
#EXT-X-PROGRAM-DATE-TIME:2026-01-01T00:00:00Z
#EXTINF:2.0,480p ad
{ad_first}
#EXT-X-PROGRAM-DATE-TIME:2026-01-01T00:00:02Z
#EXTINF:2.0,480p ad
{ad_second}
#EXT-X-PROGRAM-DATE-TIME:2026-01-01T00:00:04Z
#EXTINF:2.0,1080p program
{program}
"#
            ),
        )
        .await;
        let (recorder, work_dir) = twitch_recorder(&server, "ad-range").await;

        let saved = record_playlist_once(&recorder).await;

        assert_only_program_recorded(&saved, &work_dir);
        assert_eq!(
            tokio::fs::read_to_string(work_dir.join(".sequence"))
                .await
                .unwrap(),
            "2"
        );

        server.verify().await;
        tokio::fs::remove_dir_all(work_dir).await.unwrap();
    }

    /// Twitch writes the session, ad and source ranges together at the head of
    /// every weaver reload. `m3u8-rs` keeps only the last range before a URI, so
    /// the ad window has to be recovered from the raw playlist text.
    #[tokio::test]
    async fn twitch_head_dateranges_survive_parser_overwrite_and_skip_ads() {
        let server = MockServer::start().await;
        let ad_first = format!("{}/ads/segment-480p-1.ts", server.uri());
        let ad_second = format!("{}/ads/segment-480p-2.ts", server.uri());
        let program = format!("{}/live/segment-1080p.ts", server.uri());
        let body = format!(
            r#"#EXTM3U
#EXT-X-TARGETDURATION:2
#EXT-X-DATERANGE:ID="playlist-session-1",CLASS="twitch-playlist-session",START-DATE="2026-01-01T00:00:00Z",DURATION=6.0
#EXT-X-DATERANGE:ID="stitched-ad-1",CLASS="twitch-stitched-ad",START-DATE="2026-01-01T00:00:00Z",PLANNED-DURATION=4.0,X-TV-TWITCH-AD-URL="https://ads.example/creative"
#EXT-X-DATERANGE:ID="source-1",CLASS="twitch-stream-source",START-DATE="2026-01-01T00:00:04Z",DURATION=2.0
#EXT-X-PROGRAM-DATE-TIME:2026-01-01T00:00:00Z
#EXTINF:2.0,480p ad
{ad_first}
#EXT-X-PROGRAM-DATE-TIME:2026-01-01T00:00:02Z
#EXTINF:2.0,480p ad
{ad_second}
#EXT-X-PROGRAM-DATE-TIME:2026-01-01T00:00:04Z
#EXTINF:2.0,1080p program
{program}
"#
        );

        // The two later ranges overwrite the ad range on the first segment, so a
        // parsed playlist no longer shows which segments are ads.
        let (_, parsed) = m3u8_rs::parse_playlist(body.as_bytes()).unwrap();
        let Playlist::MediaPlaylist(parsed) = parsed else {
            panic!("expected Twitch media playlist");
        };
        assert_eq!(parsed.segments.len(), 3);
        assert!(!parsed
            .segments
            .iter()
            .any(|segment| segment.daterange.as_ref().is_some_and(is_twitch_ad_range)));

        // The raw scan keeps the ad window and reads its length from
        // `PLANNED-DURATION`, because a still-playing ad has no `DURATION` yet.
        let ranges = raw_twitch_ad_ranges(body.as_bytes());
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].end_ms - ranges[0].start_ms, 4_000);

        mount_twitch_playlist(&server, body).await;
        let (recorder, work_dir) = twitch_recorder(&server, "head-daterange").await;

        let saved = record_playlist_once(&recorder).await;

        assert_only_program_recorded(&saved, &work_dir);
        assert_eq!(
            tokio::fs::read_to_string(work_dir.join(".sequence"))
                .await
                .unwrap(),
            "2"
        );
        assert_eq!(
            load_skipped_ad_ranges(&work_dir).await.unwrap(),
            vec![ranges[0].clone()]
        );

        server.verify().await;
        tokio::fs::remove_dir_all(work_dir).await.unwrap();
    }

    #[test]
    fn date_range_attributes_keep_commas_inside_quoted_values() {
        let attributes = date_range_attributes(
            r#"ID="stitched-ad-1",X-TV-TWITCH-AD-URL="https://ads.example/a,b",DURATION=2.5"#,
        );
        assert_eq!(attributes.len(), 3);
        assert_eq!(
            attributes.get("X-TV-TWITCH-AD-URL"),
            Some(&QuotedOrUnquoted::Quoted(
                "https://ads.example/a,b".to_string()
            ))
        );
        assert_eq!(
            attributes.get("DURATION"),
            Some(&QuotedOrUnquoted::Unquoted("2.5".to_string()))
        );
    }

    #[tokio::test]
    async fn skipping_a_segment_removes_it_and_advances_sequence() {
        let (event_tx, _) = broadcast::channel(1);
        let work_dir = std::env::temp_dir().join(format!(
            "bili-shadowreplay-hls-skip-segment-{}",
            uuid::Uuid::new_v4()
        ));
        let stream = Arc::new(HlsStream::new(
            "twitch-live".to_string(),
            "https://cdn.example.test".to_string(),
            "/master.m3u8".to_string(),
            String::new(),
            Format::TS,
            Codec::Avc,
            0,
        ));
        let recorder = HlsRecorder::new(
            "twitch-channel".to_string(),
            stream,
            reqwest::Client::new(),
            None,
            event_tx,
            work_dir.clone(),
            Arc::new(AtomicBool::new(true)),
        )
        .await
        .unwrap();
        let segment_path = work_dir.join("segment_101.ts");
        tokio::fs::write(&segment_path, b"advertisement segment")
            .await
            .unwrap();

        recorder.skip_segment(&segment_path, 101).await.unwrap();

        assert!(!segment_path.exists());
        assert_eq!(recorder.sequence.load(Ordering::Relaxed), 101);
        assert_eq!(
            tokio::fs::read_to_string(work_dir.join(".sequence"))
                .await
                .unwrap(),
            "101"
        );
        tokio::fs::remove_dir_all(work_dir).await.unwrap();
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
    fn segment_uris_get_unique_local_names() {
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
            "segment-3.ts"
        );
        assert_ne!(
            local_segment_filename(3, "chunk.ts?seq=1"),
            local_segment_filename(4, "chunk.ts?seq=2")
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
