//! Project (切片) timelines.
//!
//! A project plays its clips back to back on one main track. Each clip shows
//! `start..end` of a material. Subtitles are anchored to a clip in material
//! time, so they follow the picture when clips are moved or trimmed; the part
//! outside the clip's visible window is hidden rather than lost.
//!
//! Overlays sit on tracks above the main one, a higher track covering lower
//! ones: video tracks show other materials, sticker tracks texts and
//! stickers, in any order between the main track and the subtitles. Each is
//! anchored by where it starts, like a subtitle, and runs for its own length,
//! also over later clips.
//!
//! Audio items (music, voiceover, the sound of another material) sit on audio
//! tracks below the main one and are anchored the same way; all of them are
//! heard together with the clips' own sound.
//!
//! Overlays and audio items start over the main track but may run on past its
//! end: the project then lasts until the last of them ends, over black.

use recorder::danmu::DanmuEntry;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const TIMELINE_VERSION: u32 = 1;
/// Overlay spans this short, from rounding, are empty.
const EPSILON: f64 = 1e-6;
/// Loudest a clip, picture in picture or sound is turned up to.
pub const MAX_VOLUME: f64 = 1.5;

fn timeline_version() -> u32 {
    TIMELINE_VERSION
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    #[default]
    Hevc,
    H264,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncodingPriority {
    #[default]
    Speed,
    Quality,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "StoredRenderSettings")]
pub struct RenderSettings {
    pub codec: VideoCodec,
    pub priority: EncodingPriority,
    pub bitrate_kbps: u32,
    #[serde(flatten)]
    pub encoding: crate::ffmpeg::encoding::VideoEncodingSettings,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            codec: VideoCodec::Hevc,
            priority: EncodingPriority::Speed,
            bitrate_kbps: 10_000,
            encoding: Default::default(),
        }
    }
}

/// Read old projects without writing their obsolete quality-mode fields back.
#[derive(Deserialize)]
struct StoredRenderSettings {
    codec: Option<VideoCodec>,
    priority: Option<EncodingPriority>,
    quality: Option<String>,
    #[serde(default = "default_bitrate_kbps")]
    bitrate_kbps: u32,
    #[serde(flatten)]
    encoding: crate::ffmpeg::encoding::VideoEncodingSettings,
}

fn default_bitrate_kbps() -> u32 {
    RenderSettings::default().bitrate_kbps
}

impl From<StoredRenderSettings> for RenderSettings {
    fn from(stored: StoredRenderSettings) -> Self {
        let legacy = stored.codec.is_none() && stored.priority.is_none();
        let defaults = Self::default();
        Self {
            codec: stored.codec.unwrap_or(defaults.codec),
            priority: stored.priority.unwrap_or_else(|| {
                if legacy && stored.quality.as_deref() == Some("high") {
                    EncodingPriority::Quality
                } else {
                    defaults.priority
                }
            }),
            encoding: stored.encoding,
            bitrate_kbps: match stored.bitrate_kbps {
                0 if legacy => defaults.bitrate_kbps,
                bitrate => bitrate,
            },
        }
    }
}

impl RenderSettings {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(500..=100_000).contains(&self.bitrate_kbps) {
            return Err("视频码率须在 500–100000 Kbps 之间".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    #[serde(default = "timeline_version")]
    pub version: u32,
    #[serde(default)]
    pub clips: Vec<TimelineClip>,
    #[serde(default)]
    pub subtitles: Vec<TimelineSubtitle>,
    /// Tracks drawn over the main track, the bottom one first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<OverlayTrack>,
    /// The editor's `SubtitleStyle`, opaque to the backend: subtitles are burned
    /// from the ASS document the editor previews.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle_style: Option<serde_json::Value>,
    /// Burn the materials' danmaku when exporting.
    #[serde(default)]
    pub danmu: bool,
    #[serde(default, skip_serializing_if = "RenderSettings::is_default")]
    pub render_settings: RenderSettings,
    /// Tracks of sound below the main track, the top one first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio_tracks: Vec<AudioTrack>,
}

impl Default for Timeline {
    fn default() -> Self {
        Self {
            version: TIMELINE_VERSION,
            clips: Vec::new(),
            subtitles: Vec::new(),
            tracks: Vec::new(),
            subtitle_style: None,
            danmu: false,
            render_settings: RenderSettings::default(),
            audio_tracks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineClip {
    pub id: String,
    pub material_id: i64,
    /// In point within the material, in seconds.
    pub start: f64,
    /// Out point within the material, in seconds.
    pub end: f64,
    /// Of the material's own sound, 0 to [`MAX_VOLUME`].
    #[serde(default = "full_volume", skip_serializing_if = "is_full_volume")]
    pub volume: f64,
}

impl TimelineClip {
    pub fn duration(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineSubtitle {
    pub id: String,
    pub clip_id: String,
    /// Material time of the anchored clip, in seconds.
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// A track above the main one; its items do not overlap in time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlayTrack {
    pub id: String,
    /// What the track holds; absent on tracks from before tracks had kinds,
    /// which may hold both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<TrackKind>,
    #[serde(default)]
    pub items: Vec<Overlay>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    /// Video overlays.
    Video,
    /// Texts and stickers.
    Sticker,
}

/// Something drawn over the main track, anchored to the clip it starts over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Overlay {
    pub id: String,
    pub clip_id: String,
    /// Where it starts, in the anchor clip's material time.
    pub start: f64,
    /// Length in project time.
    pub duration: f64,
    #[serde(flatten)]
    pub content: OverlayContent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum OverlayContent {
    /// Drawn by the editor, which sends the picture to burn with an export.
    Text {
        element: serde_json::Value,
    },
    /// Drawn by the editor, like a text.
    Sticker {
        element: serde_json::Value,
    },
    Video(VideoOverlay),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoOverlay {
    pub material_id: i64,
    /// Material time shown at the overlay's start.
    pub source_start: f64,
    /// 0 to [`MAX_VOLUME`].
    #[serde(default = "full_volume")]
    pub volume: f64,
    /// 0 to 1; opaque when absent.
    #[serde(default = "full_volume", skip_serializing_if = "is_full_volume")]
    pub opacity: f64,
    pub element: VideoPlacement,
}

fn full_volume() -> f64 {
    1.0
}

fn is_full_volume(volume: &f64) -> bool {
    *volume == 1.0
}

fn is_zero(value: &f64) -> bool {
    *value == 0.0
}

/// Where a video overlay sits on the 1920×1080 picture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoPlacement {
    /// Center.
    pub x: f64,
    pub y: f64,
    /// The height follows the material's aspect ratio.
    pub width: f64,
    /// Degrees, clockwise.
    #[serde(default)]
    pub rotation: f64,
}

impl Overlay {
    pub fn video(&self) -> Option<&VideoOverlay> {
        match &self.content {
            OverlayContent::Video(video) => Some(video),
            _ => None,
        }
    }

    /// The kind of track it goes on.
    pub fn track_kind(&self) -> TrackKind {
        match self.content {
            OverlayContent::Video(_) => TrackKind::Video,
            OverlayContent::Text { .. } | OverlayContent::Sticker { .. } => TrackKind::Sticker,
        }
    }
}

/// A track below the main one; its items do not overlap in time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioTrack {
    pub id: String,
    #[serde(default)]
    pub items: Vec<AudioItem>,
}

/// Sound heard with the main track, anchored to the clip it starts over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioItem {
    pub id: String,
    pub clip_id: String,
    /// Where it starts, in the anchor clip's material time.
    pub start: f64,
    /// Length in project time.
    pub duration: f64,
    #[serde(flatten)]
    pub source: AudioSource,
    /// Time in the source heard at the item's start.
    #[serde(default)]
    pub source_start: f64,
    /// 0 to [`MAX_VOLUME`].
    #[serde(default = "full_volume")]
    pub volume: f64,
    /// Seconds the sound fades in over from where it is first heard, and out
    /// over until where it is last heard.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fade_in: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fade_out: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AudioSource {
    /// A file of the audio library.
    File { audio_id: i64 },
    /// The sound of a material, without its picture.
    Material { material_id: i64 },
}

impl AudioItem {
    pub fn material_id(&self) -> Option<i64> {
        match self.source {
            AudioSource::Material { material_id } => Some(material_id),
            AudioSource::File { .. } => None,
        }
    }

    pub fn audio_id(&self) -> Option<i64> {
        match self.source {
            AudioSource::File { audio_id } => Some(audio_id),
            AudioSource::Material { .. } => None,
        }
    }
}

/// A clip together with where it starts on the project timeline.
pub struct PlacedClip<'a> {
    pub clip: &'a TimelineClip,
    pub offset: f64,
}

/// The visible part of an overlay, in project time.
pub struct PlacedOverlay<'a> {
    pub overlay: &'a Overlay,
    pub start: f64,
    pub end: f64,
    /// How much is hidden before `start`, cut off with the head of its clip; a
    /// video overlay skips as much of its material.
    pub skip: f64,
}

/// The part of an audio item that is heard, in project time.
pub struct PlacedAudio<'a> {
    pub item: &'a AudioItem,
    pub start: f64,
    pub end: f64,
    /// How much is cut off before `start` with the head of its clip, which
    /// its source skips.
    pub skip: f64,
}

/// The visible part of something anchored at `start` of clip `clip_id` and
/// running `duration` on, maybe past the main track, as `(start, end, skip)`
/// in project time like a [`PlacedOverlay`]; `None` when none of it is.
fn anchored_span(
    placed: &[PlacedClip],
    clip_id: &str,
    start: f64,
    duration: f64,
) -> Option<(f64, f64, f64)> {
    let clip = placed.iter().find(|p| p.clip.id == clip_id)?;
    // Anchored past the out point, it was trimmed away with its clip.
    if start >= clip.clip.end {
        return None;
    }
    let from = clip.offset + start - clip.clip.start;
    let shown = from.max(clip.offset);
    let end = from + duration;
    (end - shown > EPSILON).then_some((shown, end, shown - from))
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

impl Timeline {
    pub fn parse(json: &str) -> Result<Self, String> {
        if json.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(json).map_err(|e| format!("时间轴数据无效: {e}"))
    }

    pub fn to_json(&self) -> String {
        // Serializing plain data cannot fail.
        serde_json::to_string(self).unwrap_or_default()
    }

    /// One full-length clip per material, in order.
    pub fn from_materials(materials: &[(i64, f64)]) -> Self {
        Self {
            clips: materials
                .iter()
                .map(|&(material_id, duration)| TimelineClip {
                    id: new_id(),
                    material_id,
                    start: 0.0,
                    end: duration,
                    volume: 1.0,
                })
                .collect(),
            ..Self::default()
        }
    }

    pub fn placed(&self) -> Vec<PlacedClip<'_>> {
        let mut offset = 0.0;
        self.clips
            .iter()
            .map(|clip| {
                let placed = PlacedClip { clip, offset };
                offset += clip.duration();
                placed
            })
            .collect()
    }

    /// How long the clips of the main track last together.
    pub fn main_duration(&self) -> f64 {
        self.clips.iter().map(TimelineClip::duration).sum()
    }

    /// How long the project lasts: until the end of the main track, or of
    /// the overlays and audio items running on past it.
    pub fn duration(&self) -> f64 {
        let overlays = self.placed_overlays().into_iter().map(|placed| placed.end);
        let audio = self.placed_audio().into_iter().map(|placed| placed.end);
        overlays.chain(audio).fold(self.main_duration(), f64::max)
    }

    /// Project-time span of a subtitle, or `None` when it is hidden.
    pub fn subtitle_span(&self, subtitle: &TimelineSubtitle) -> Option<(f64, f64)> {
        let placed = self.placed();
        let placed = placed.iter().find(|p| p.clip.id == subtitle.clip_id)?;
        let start = subtitle.start.max(placed.clip.start);
        let end = subtitle.end.min(placed.clip.end);
        (end - start > 0.0).then_some((
            placed.offset + start - placed.clip.start,
            placed.offset + end - placed.clip.start,
        ))
    }

    pub fn has_visible_subtitles(&self) -> bool {
        self.subtitles.iter().any(|subtitle| {
            !subtitle.text.trim().is_empty() && self.subtitle_span(subtitle).is_some()
        })
    }

    /// Visible overlays, the bottom track first and each track's items in
    /// order, which is the order they are drawn in.
    pub fn placed_overlays(&self) -> Vec<PlacedOverlay<'_>> {
        let placed = self.placed();
        self.tracks
            .iter()
            .flat_map(|track| &track.items)
            .filter_map(|overlay| {
                let (start, end, skip) =
                    anchored_span(&placed, &overlay.clip_id, overlay.start, overlay.duration)?;
                Some(PlacedOverlay {
                    overlay,
                    start,
                    end,
                    skip,
                })
            })
            .collect()
    }

    /// The audio items heard, the top track first and each track's items in
    /// order.
    pub fn placed_audio(&self) -> Vec<PlacedAudio<'_>> {
        let placed = self.placed();
        self.audio_tracks
            .iter()
            .flat_map(|track| &track.items)
            .filter_map(|item| {
                let (start, end, skip) =
                    anchored_span(&placed, &item.clip_id, item.start, item.duration)?;
                Some(PlacedAudio {
                    item,
                    start,
                    end,
                    skip,
                })
            })
            .collect()
    }

    pub fn has_visible_overlays(&self) -> bool {
        !self.placed_overlays().is_empty()
    }

    /// Bring the backend's own edits of a saved timeline into this one, which
    /// is being edited. Between `base`, the saved timeline the editor last
    /// knew, and `saved`, the saved one now, the backend may switch danmaku,
    /// remove the clips of a deleted material and fill clips that had none
    /// with recognized subtitles; everything else stays as edited here. The
    /// editor merges the same way when it hears of a change.
    pub fn merge_saved_edits(&mut self, base: &Timeline, saved: &Timeline) {
        self.danmu = saved.danmu;
        self.render_settings = saved.render_settings.clone();
        let remaining = saved.material_ids();
        for material_id in base.material_ids() {
            if !remaining.contains(&material_id) {
                self.remove_material(material_id);
            }
        }
        let remaining = saved.audio_ids();
        for audio_id in base.audio_ids() {
            if !remaining.contains(&audio_id) {
                self.remove_audio(audio_id);
            }
        }
        let filled: Vec<TimelineSubtitle> = saved
            .subtitles
            .iter()
            .filter(|subtitle| self.clips.iter().any(|clip| clip.id == subtitle.clip_id))
            .filter(|subtitle| {
                !base.subtitles.iter().any(|s| s.clip_id == subtitle.clip_id)
                    && !self.subtitles.iter().any(|s| s.clip_id == subtitle.clip_id)
            })
            .cloned()
            .collect();
        self.subtitles.extend(filled);
    }

    /// Distinct materials in timeline order: the main track's, then those of
    /// video overlays, then those heard on audio tracks.
    pub fn material_ids(&self) -> Vec<i64> {
        let overlays = self
            .tracks
            .iter()
            .flat_map(|track| &track.items)
            .filter_map(Overlay::video)
            .map(|video| video.material_id);
        let sounds = self.audio_items().filter_map(AudioItem::material_id);
        distinct(
            self.clips
                .iter()
                .map(|clip| clip.material_id)
                .chain(overlays)
                .chain(sounds),
        )
    }

    /// Distinct files of the audio library heard, in timeline order.
    pub fn audio_ids(&self) -> Vec<i64> {
        distinct(self.audio_items().filter_map(AudioItem::audio_id))
    }

    pub fn uses_audio(&self, audio_id: i64) -> bool {
        self.audio_ids().contains(&audio_id)
    }

    fn audio_items(&self) -> impl Iterator<Item = &AudioItem> {
        self.audio_tracks.iter().flat_map(|track| &track.items)
    }

    pub fn uses_material(&self, material_id: i64) -> bool {
        self.material_ids().contains(&material_id)
    }

    /// Drop the material's clips with the subtitles and overlays anchored to
    /// them, the video overlays showing it and the audio items playing its
    /// sound. Other audio items anchored to its clips move on to the clips
    /// that take their place.
    pub fn remove_material(&mut self, material_id: i64) -> bool {
        let removed: Vec<String> = self
            .clips
            .iter()
            .filter(|clip| clip.material_id == material_id)
            .map(|clip| clip.id.clone())
            .collect();
        let items = self.item_count();
        self.retain_audio(|item| item.material_id() != Some(material_id));
        self.reanchor_audio(&removed);
        self.clips.retain(|clip| clip.material_id != material_id);
        self.subtitles
            .retain(|subtitle| !removed.contains(&subtitle.clip_id));
        for track in &mut self.tracks {
            track.items.retain(|overlay| {
                !removed.contains(&overlay.clip_id)
                    && overlay
                        .video()
                        .is_none_or(|video| video.material_id != material_id)
            });
        }
        self.tracks.retain(|track| !track.items.is_empty());
        !removed.is_empty() || self.item_count() != items
    }

    /// Drop the audio items playing a file of the audio library.
    pub fn remove_audio(&mut self, audio_id: i64) -> bool {
        let items = self.item_count();
        self.retain_audio(|item| item.audio_id() != Some(audio_id));
        self.item_count() != items
    }

    fn retain_audio(&mut self, mut keep: impl FnMut(&AudioItem) -> bool) {
        for track in &mut self.audio_tracks {
            track.items.retain(&mut keep);
        }
        self.audio_tracks.retain(|track| !track.items.is_empty());
    }

    /// Move the audio items anchored to the clips of `removed`, which are
    /// about to go, onto the first clip after them that stays: where those
    /// clips were, playing on from where each was first heard. Those with no
    /// clip after them, or not heard at all, go.
    fn reanchor_audio(&mut self, removed: &[String]) {
        let placed = self.placed();
        let mut moves = Vec::new();
        for (track, items) in self.audio_tracks.iter().enumerate() {
            for (index, item) in items.items.iter().enumerate() {
                let Some(at) = placed.iter().position(|p| p.clip.id == item.clip_id) else {
                    continue;
                };
                if !removed.contains(&item.clip_id) {
                    continue;
                }
                let clip = &placed[at];
                let from = clip.offset + item.start - clip.clip.start;
                let skip = (clip.offset - from).max(0.0);
                let heard = item.start < clip.clip.end && item.duration - skip > EPSILON;
                let next = placed[at + 1..]
                    .iter()
                    .find(|p| !removed.contains(&p.clip.id))
                    .filter(|_| heard)
                    .map(|p| (p.clip.id.clone(), p.clip.start));
                moves.push((track, index, next, skip));
            }
        }
        let mut gone = Vec::new();
        for (track, index, next, skip) in moves {
            let item = &mut self.audio_tracks[track].items[index];
            match next {
                Some((clip_id, start)) => {
                    item.clip_id = clip_id;
                    item.start = start;
                    item.source_start += skip;
                    item.duration -= skip;
                }
                None => gone.push(item.id.clone()),
            }
        }
        self.retain_audio(|item| !gone.contains(&item.id));
    }

    fn item_count(&self) -> usize {
        let overlays: usize = self.tracks.iter().map(|track| track.items.len()).sum();
        overlays + self.audio_items().count()
    }

    /// Make `clips` the main track. Subtitles, overlays and sounds anchored to
    /// a clip whose id stays on it follow that clip; those of the others go
    /// with them.
    pub fn replace_clips(&mut self, clips: Vec<TimelineClip>) {
        self.clips = clips;
        let kept: Vec<String> = self.clips.iter().map(|clip| clip.id.clone()).collect();
        self.subtitles
            .retain(|subtitle| kept.contains(&subtitle.clip_id));
        for track in &mut self.tracks {
            track
                .items
                .retain(|overlay| kept.contains(&overlay.clip_id));
        }
        self.tracks.retain(|track| !track.items.is_empty());
        self.retain_audio(|item| kept.contains(&item.clip_id));
    }

    /// Anchor material-time cues to `clip_id`, replacing its existing subtitles.
    pub fn set_clip_subtitles(&mut self, clip_id: &str, cues: Vec<(f64, f64, String)>) {
        self.subtitles
            .retain(|subtitle| subtitle.clip_id != clip_id);
        self.subtitles
            .extend(cues.into_iter().map(|(start, end, text)| TimelineSubtitle {
                id: new_id(),
                clip_id: clip_id.to_string(),
                start,
                end,
                text,
            }));
    }

    /// Check clip ranges against the materials' lengths, and what audio
    /// items play against those of their sources: materials, or files of the
    /// audio library.
    pub fn validate(
        &self,
        material_duration: impl Fn(i64) -> Option<f64>,
        audio_duration: impl Fn(i64) -> Option<f64>,
    ) -> Result<(), String> {
        self.render_settings.validate()?;
        if self.clips.is_empty() {
            return Err("时间轴上没有片段".to_string());
        }
        for (index, clip) in self.clips.iter().enumerate() {
            let Some(duration) = material_duration(clip.material_id) else {
                return Err(format!("第 {} 个片段的素材不存在", index + 1));
            };
            let valid = clip.start.is_finite()
                && clip.end.is_finite()
                && clip.start >= 0.0
                && clip.end > clip.start
                // Durations of legacy materials are probed lazily and may be 0.
                && (duration <= 0.0 || clip.end <= duration + 0.5)
                && is_volume(clip.volume);
            if !valid {
                return Err(format!("第 {} 个片段的范围无效", index + 1));
            }
        }
        for placed in self.placed_audio() {
            let item = placed.item;
            let duration = match item.source {
                AudioSource::File { audio_id } => audio_duration(audio_id),
                AudioSource::Material { material_id } => material_duration(material_id),
            };
            let Some(duration) = duration else {
                return Err("音频轨上的声音文件不存在".to_string());
            };
            let start = item.source_start + placed.skip;
            let valid = start.is_finite()
                && start >= 0.0
                && (duration <= 0.0 || start + placed.end - placed.start <= duration + 0.5)
                && is_volume(item.volume)
                && [item.fade_in, item.fade_out]
                    .iter()
                    .all(|fade| fade.is_finite() && *fade >= 0.0);
            if !valid {
                return Err("音频轨上的声音范围无效".to_string());
            }
        }
        let misplaced = self.tracks.iter().any(|track| {
            track
                .kind
                .is_some_and(|kind| track.items.iter().any(|item| item.track_kind() != kind))
        });
        if misplaced {
            return Err("轨道上有不属于该轨道类型的内容".to_string());
        }
        for placed in self.placed_overlays() {
            let Some(video) = placed.overlay.video() else {
                continue;
            };
            let Some(duration) = material_duration(video.material_id) else {
                return Err("画中画的素材不存在".to_string());
            };
            let start = video.source_start + placed.skip;
            let placement = &video.element;
            let valid = start.is_finite()
                && start >= 0.0
                && (duration <= 0.0 || start + placed.end - placed.start <= duration + 0.5)
                && is_volume(video.volume)
                && [placement.x, placement.y, placement.rotation]
                    .iter()
                    .all(|value| value.is_finite())
                && placement.width.is_finite()
                && placement.width >= 1.0
                && (0.0..=1.0).contains(&video.opacity);
            if !valid {
                return Err("画中画的范围无效".to_string());
            }
        }
        Ok(())
    }

    /// Even an untouched material must be encoded: its source codec and bitrate
    /// cannot be assumed to match the requested output settings.
    pub fn single_full_material(&self, _material_duration: f64) -> Option<i64> {
        None
    }

    /// Identifies an export: a different hash means the output is stale.
    pub fn render_hash(&self, extra: &str) -> String {
        let mut hasher = Sha256::new();
        // Invalidate complete upload renders as well as individual segments
        // when switching to premultiplied native overlay composition.
        hasher.update(b"project-render-v3\0");
        hasher.update(self.to_json().as_bytes());
        hasher.update([0]);
        hasher.update(extra.as_bytes());
        hasher
            .finalize()
            .iter()
            .take(8)
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// Map each clip's visible material danmaku (`ts` in ms from the material
    /// start) onto the project timeline at `fps`.
    ///
    /// Each lands in the middle of the frame before the first one showing it,
    /// counted from its clip's first frame on the project's frame grid. Moved
    /// by whole frames with its clip, a danmaku then moves by as many frames,
    /// however its time is rounded on the way to an ASS document.
    pub fn map_danmus<'a>(
        &self,
        fps: f64,
        material_danmus: impl Fn(i64) -> Option<&'a [DanmuEntry]>,
    ) -> Vec<DanmuEntry> {
        let mut mapped = Vec::new();
        for placed in self.placed() {
            let Some(danmus) = material_danmus(placed.clip.material_id) else {
                continue;
            };
            let start = (placed.clip.start * 1000.0).round() as i64;
            let end = (placed.clip.end * 1000.0).round() as i64;
            let first_frame = (placed.offset * fps).round();
            mapped.extend(
                danmus
                    .iter()
                    .filter(|danmu| danmu.ts >= start && danmu.ts < end)
                    .map(|danmu| {
                        let into = danmu.ts as f64 / 1000.0 - placed.clip.start;
                        let frame = first_frame + (into * fps - EPSILON).ceil();
                        DanmuEntry {
                            ts: ((frame - 0.5) / fps * 1000.0).round().max(0.0) as i64,
                            ..danmu.clone()
                        }
                    }),
            );
        }
        mapped.sort_by_key(|danmu| danmu.ts);
        mapped
    }
}

fn is_volume(volume: f64) -> bool {
    (0.0..=MAX_VOLUME).contains(&volume)
}

/// `ids` without repeats, in order.
fn distinct(ids: impl Iterator<Item = i64>) -> Vec<i64> {
    let mut result = Vec::new();
    for id in ids {
        if !result.contains(&id) {
            result.push(id);
        }
    }
    result
}

/// Parse SRT into `(start, end, text)` cues in seconds.
pub fn srt_cues(content: &str) -> Vec<(f64, f64, String)> {
    let seconds = |time: &srtparse::Time| {
        time.hours as f64 * 3600.0
            + time.minutes as f64 * 60.0
            + time.seconds as f64
            + time.milliseconds as f64 / 1000.0
    };
    srtparse::from_str(content)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    (
                        seconds(&item.start_time),
                        seconds(&item.end_time),
                        item.text.clone(),
                    )
                })
                .filter(|(start, end, _)| end > start)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: &str, material_id: i64, start: f64, end: f64) -> TimelineClip {
        TimelineClip {
            id: id.into(),
            material_id,
            start,
            end,
            volume: 1.0,
        }
    }

    fn subtitle(clip_id: &str, start: f64, end: f64) -> TimelineSubtitle {
        TimelineSubtitle {
            id: new_id(),
            clip_id: clip_id.into(),
            start,
            end,
            text: "text".into(),
        }
    }

    fn danmu(ts: i64) -> DanmuEntry {
        DanmuEntry {
            ts,
            event_type: "danmu".into(),
            content: ts.to_string(),
            emoji_content: None,
            user_name: None,
            price: None,
            sc_duration: None,
        }
    }

    fn timeline() -> Timeline {
        Timeline {
            clips: vec![clip("a", 1, 10.0, 20.0), clip("b", 2, 0.0, 5.0)],
            ..Timeline::default()
        }
    }

    #[test]
    fn render_settings_defaults_and_legacy_migration() {
        use serde_json::json;

        let defaults = RenderSettings::default();
        assert_eq!(
            serde_json::to_value(&defaults).unwrap(),
            json!({
                "codec": "hevc",
                "priority": "speed",
                "bitrate_kbps": 10000,
                "render_backend": null,
                "encoder": null,
                "options": {}
            })
        );
        assert_eq!(
            serde_json::from_str::<RenderSettings>("{}").unwrap(),
            defaults
        );
        for quality in ["high", "balanced", "compact", "unknown"] {
            for bitrate in [None, Some(0), Some(500), Some(12_000), Some(100_000)] {
                let mut legacy = json!({ "quality": quality });
                if let Some(bitrate) = bitrate {
                    legacy["bitrate_kbps"] = json!(bitrate);
                }
                let migrated: RenderSettings = serde_json::from_value(legacy).unwrap();
                assert_eq!(migrated.codec, VideoCodec::Hevc);
                assert_eq!(
                    migrated.priority,
                    if quality == "high" {
                        EncodingPriority::Quality
                    } else {
                        EncodingPriority::Speed
                    }
                );
                assert_eq!(
                    migrated.bitrate_kbps,
                    bitrate.filter(|value| *value > 0).unwrap_or(10_000)
                );
                assert!(migrated.validate().is_ok());
                let serialized = serde_json::to_value(&migrated).unwrap();
                assert!(serialized.get("quality").is_none());
                assert_eq!(
                    serde_json::from_value::<RenderSettings>(serialized).unwrap(),
                    migrated
                );
            }
        }
        let explicit: RenderSettings = serde_json::from_value(json!({
            "codec": "h264", "priority": "speed", "quality": "high"
        }))
        .unwrap();
        assert_eq!(explicit.codec, VideoCodec::H264);
        assert_eq!(explicit.priority, EncodingPriority::Speed);
        assert_eq!(explicit.bitrate_kbps, 10_000);
    }

    #[test]
    fn current_render_settings_reject_invalid_bitrates() {
        use serde_json::json;

        for bitrate in [0, 1, 499, 100_001] {
            let settings: RenderSettings = serde_json::from_value(json!({
                "codec": "hevc", "priority": "speed", "bitrate_kbps": bitrate
            }))
            .unwrap();
            assert_eq!(settings.bitrate_kbps, bitrate);
            assert!(settings.validate().is_err());
        }
        // Either new field marks current settings; zero must not migrate.
        for current in [
            json!({ "codec": "hevc", "bitrate_kbps": 0 }),
            json!({ "priority": "quality", "bitrate_kbps": 0 }),
        ] {
            assert!(serde_json::from_value::<RenderSettings>(current)
                .unwrap()
                .validate()
                .is_err());
        }
        for bitrate in [
            json!(-1),
            json!(1.5),
            json!(10000.0),
            json!(null),
            json!("10000"),
            json!(u64::MAX),
        ] {
            assert!(serde_json::from_value::<RenderSettings>(json!({
                "codec": "hevc", "priority": "speed", "bitrate_kbps": bitrate
            }))
            .is_err());
        }
        for value in ["NaN", "Infinity", "1e999"] {
            assert!(serde_json::from_str::<RenderSettings>(&format!(
                r#"{{"codec":"hevc","priority":"speed","bitrate_kbps":{value}}}"#
            ))
            .is_err());
        }
        for bitrate_kbps in [500, 10_000, 100_000] {
            assert!(RenderSettings {
                bitrate_kbps,
                ..RenderSettings::default()
            }
            .validate()
            .is_ok());
        }
    }

    #[test]
    fn output_settings_survive_old_projects_and_editor_merges() {
        let mut edited =
            Timeline::parse(r#"{"clips":[{"id":"a","material_id":1,"start":0,"end":30}]}"#)
                .unwrap();
        assert_eq!(edited.render_settings, RenderSettings::default());
        assert_eq!(edited.single_full_material(30.0), None);
        let base = edited.clone();
        let mut saved = base.clone();
        saved.render_settings = RenderSettings {
            codec: VideoCodec::H264,
            priority: EncodingPriority::Quality,
            bitrate_kbps: 12_000,
            encoding: Default::default(),
        };
        edited.clips[0].end = 20.0;
        edited.merge_saved_edits(&base, &saved);
        assert_eq!(edited.clips[0].end, 20.0);
        assert_eq!(edited.render_settings, saved.render_settings);
        assert_eq!(saved.single_full_material(30.0), None);
        assert_ne!(base.render_hash(""), saved.render_hash(""));
        assert_eq!(Timeline::parse(&saved.to_json()).unwrap(), saved);
        saved.render_settings.bitrate_kbps = 100_001;
        assert!(saved.validate(|_| Some(30.0), |_| None).is_err());
    }

    #[test]
    fn clips_play_back_to_back() {
        let timeline = timeline();
        let offsets: Vec<f64> = timeline.placed().iter().map(|p| p.offset).collect();
        assert_eq!(offsets, vec![0.0, 10.0]);
        assert_eq!(timeline.duration(), 15.0);
    }

    #[test]
    fn subtitles_follow_their_clip_and_hide_outside_it() {
        let timeline = timeline();
        assert_eq!(
            timeline.subtitle_span(&subtitle("a", 12.0, 13.0)),
            Some((2.0, 3.0))
        );
        assert_eq!(
            timeline.subtitle_span(&subtitle("b", 1.0, 2.0)),
            Some((11.0, 12.0))
        );
        // Clipped to the visible window of the clip.
        assert_eq!(
            timeline.subtitle_span(&subtitle("a", 18.0, 25.0)),
            Some((8.0, 10.0))
        );
        assert_eq!(timeline.subtitle_span(&subtitle("a", 1.0, 5.0)), None);
        assert_eq!(timeline.subtitle_span(&subtitle("missing", 1.0, 5.0)), None);
    }

    #[test]
    fn danmaku_outside_clips_is_dropped_and_the_rest_is_shifted() {
        let timeline = timeline();
        let first = [danmu(9_000), danmu(10_000), danmu(19_999), danmu(20_000)];
        let second = [danmu(1_000)];
        let mapped = timeline.map_danmus(30.0, |id| match id {
            1 => Some(&first[..]),
            2 => Some(&second[..]),
            _ => None,
        });
        // Each half a frame before the first frame showing it: frames 0,
        // 300 and 330.
        let times: Vec<i64> = mapped.iter().map(|d| d.ts).collect();
        assert_eq!(times, vec![0, 9_983, 10_983]);
    }

    #[test]
    fn danmaku_moved_by_whole_frames_move_by_as_many_frames() {
        let first = [danmu(0)];
        let second = [danmu(1_234), danmu(2_000)];
        let frames = |lead: f64| {
            let timeline = Timeline {
                clips: vec![clip("a", 1, 0.0, lead), clip("b", 2, 0.0, 5.0)],
                ..Timeline::default()
            };
            let mapped = timeline.map_danmus(60.0, |id| match id {
                1 => Some(&first[..]),
                2 => Some(&second[..]),
                _ => None,
            });
            // The frame showing each first, as an ASS document has it, to the
            // centisecond.
            mapped[1..]
                .iter()
                .map(|danmu| {
                    let centiseconds = (danmu.ts as f64 / 10.0).round();
                    (centiseconds / 100.0 * 60.0).ceil() as i64
                })
                .collect::<Vec<_>>()
        };
        let before = frames(1.0);
        for moved in 1..=12 {
            let after = frames(1.0 + f64::from(moved) / 60.0);
            let shifted: Vec<_> = before
                .iter()
                .map(|frame| frame + i64::from(moved))
                .collect();
            assert_eq!(after, shifted, "moved by {moved} frames");
        }
    }

    #[test]
    fn even_an_untouched_single_material_requires_encoding() {
        let mut timeline = Timeline::from_materials(&[(7, 30.0)]);
        assert_eq!(timeline.single_full_material(30.0), None);
        timeline.danmu = true;
        assert_eq!(timeline.single_full_material(30.0), None);
        timeline.danmu = false;
        let clip_id = timeline.clips[0].id.clone();
        timeline.set_clip_subtitles(&clip_id, vec![(1.0, 2.0, "hi".into())]);
        assert_eq!(timeline.single_full_material(30.0), None);
        let trimmed = Timeline {
            clips: vec![clip("a", 7, 1.0, 30.0)],
            ..Timeline::default()
        };
        assert_eq!(trimmed.single_full_material(30.0), None);
    }

    #[test]
    fn replacing_clips_keeps_what_is_anchored_to_kept_clips_only() {
        let mut timeline = timeline();
        timeline.subtitles = vec![subtitle("a", 11.0, 12.0), subtitle("b", 1.0, 2.0)];
        timeline.tracks = vec![OverlayTrack {
            id: "t".into(),
            kind: Some(TrackKind::Sticker),
            items: vec![Overlay {
                id: "o".into(),
                clip_id: "b".into(),
                start: 0.0,
                duration: 1.0,
                content: OverlayContent::Text {
                    element: serde_json::Value::Null,
                },
            }],
        }];
        timeline.audio_tracks = vec![AudioTrack {
            id: "s".into(),
            items: vec![AudioItem {
                id: "i".into(),
                clip_id: "a".into(),
                start: 10.0,
                duration: 1.0,
                source: AudioSource::File { audio_id: 3 },
                source_start: 0.0,
                volume: 1.0,
                fade_in: 0.0,
                fade_out: 0.0,
            }],
        }];
        timeline.replace_clips(vec![clip("c", 3, 0.0, 4.0), clip("a", 1, 12.0, 15.0)]);
        let ids: Vec<&str> = timeline.clips.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["c", "a"]);
        assert_eq!(timeline.subtitles.len(), 1);
        assert_eq!(timeline.subtitles[0].clip_id, "a");
        assert!(timeline.tracks.is_empty());
        assert_eq!(timeline.audio_ids(), vec![3]);
    }

    #[test]
    fn removing_a_material_drops_its_clips_and_subtitles() {
        let mut timeline = timeline();
        timeline.subtitles = vec![subtitle("a", 11.0, 12.0), subtitle("b", 1.0, 2.0)];
        assert!(timeline.remove_material(1));
        assert_eq!(timeline.clips.len(), 1);
        assert_eq!(timeline.subtitles.len(), 1);
        assert_eq!(timeline.subtitles[0].clip_id, "b");
        assert!(!timeline.remove_material(1));
    }

    #[test]
    fn validation_rejects_missing_materials_and_bad_ranges() {
        let timeline = timeline();
        assert!(timeline.validate(|_| Some(30.0), |_| None).is_ok());
        assert!(timeline
            .validate(|id| (id == 1).then_some(30.0), |_| None)
            .is_err());
        assert!(timeline.validate(|_| Some(12.0), |_| None).is_err());
        assert!(Timeline::default()
            .validate(|_| Some(1.0), |_| None)
            .is_err());
    }

    #[test]
    fn hash_changes_with_the_timeline() {
        let timeline = timeline();
        let mut changed = timeline.clone();
        changed.clips[0].end = 19.0;
        assert_eq!(timeline.render_hash(""), timeline.render_hash(""));
        assert_ne!(timeline.render_hash(""), changed.render_hash(""));
        assert_ne!(timeline.render_hash(""), timeline.render_hash("danmu"));
    }

    fn text_overlay(id: &str, clip_id: &str, start: f64, duration: f64) -> Overlay {
        Overlay {
            id: id.into(),
            clip_id: clip_id.into(),
            start,
            duration,
            content: OverlayContent::Text {
                element: serde_json::json!({ "content": id }),
            },
        }
    }

    fn video_overlay(id: &str, clip_id: &str, start: f64, material_id: i64) -> Overlay {
        Overlay {
            id: id.into(),
            clip_id: clip_id.into(),
            start,
            duration: 3.0,
            content: OverlayContent::Video(VideoOverlay {
                material_id,
                source_start: 1.0,
                volume: 1.0,
                opacity: 1.0,
                element: VideoPlacement {
                    x: 1500.0,
                    y: 800.0,
                    width: 640.0,
                    rotation: 0.0,
                },
            }),
        }
    }

    fn with_tracks(tracks: Vec<Vec<Overlay>>) -> Timeline {
        Timeline {
            tracks: tracks
                .into_iter()
                .enumerate()
                .map(|(index, items)| OverlayTrack {
                    id: format!("t{index}"),
                    kind: None,
                    items,
                })
                .collect(),
            ..timeline()
        }
    }

    fn spans(timeline: &Timeline) -> Vec<(String, f64, f64, f64)> {
        timeline
            .placed_overlays()
            .iter()
            .map(|p| (p.overlay.id.clone(), p.start, p.end, p.skip))
            .collect()
    }

    #[test]
    fn overlays_keep_the_fields_only_the_editor_reads() {
        let json = r##"{"version":1,"clips":[],"subtitles":[],"tracks":[{"id":"t","items":[
            {"id":"a","clip_id":"c","start":1.5,"duration":2.0,"kind":"text",
             "element":{"kind":"text","id":0,"x":960,"y":540,"rotation":0,"content":"hi","fill":"#FFF","stroke":null}},
            {"id":"b","clip_id":"c","start":0.0,"duration":3.0,"kind":"video","material_id":4,
             "source_start":2.0,"volume":0.5,"element":{"x":10.0,"y":20.0,"width":640.0,"rotation":15.0}}]}],"danmu":false}"##;
        let timeline = Timeline::parse(json).unwrap();
        let items = &timeline.tracks[0].items;
        assert!(
            matches!(&items[0].content, OverlayContent::Text { element } if element["fill"] == "#FFF")
        );
        assert_eq!(
            items[1].video().map(|v| (v.material_id, v.volume)),
            Some((4, 0.5))
        );
        let reparsed: serde_json::Value = serde_json::from_str(&timeline.to_json()).unwrap();
        let original: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(reparsed, original);
        // Timelines without overlays are saved as before.
        assert!(!Timeline::default().to_json().contains("tracks"));
    }

    #[test]
    fn overlays_follow_their_clip_and_run_on_over_later_ones() {
        // a shows 10-20 at 0-10, b shows 0-5 at 10-15.
        let timeline = with_tracks(vec![
            vec![
                text_overlay("x", "a", 18.0, 4.0),
                text_overlay("tail", "a", 20.0, 1.0),
            ],
            vec![
                text_overlay("head", "a", 9.0, 3.0),
                text_overlay("end", "b", 4.0, 9.0),
            ],
        ]);
        assert_eq!(
            spans(&timeline),
            vec![
                ("x".to_string(), 8.0, 12.0, 0.0),
                // Its first second went with the trimmed head of clip a.
                ("head".to_string(), 0.0, 2.0, 1.0),
                // On past the end of the main track, which the project
                // lasts until.
                ("end".to_string(), 14.0, 23.0, 0.0),
            ]
        );
        assert_eq!(timeline.main_duration(), 15.0);
        assert_eq!(timeline.duration(), 23.0);
        assert!(timeline.has_visible_overlays());
        assert!(
            !with_tracks(vec![vec![text_overlay("x", "missing", 0.0, 1.0)]]).has_visible_overlays()
        );
    }

    #[test]
    fn video_overlays_count_as_used_materials_and_leave_with_them() {
        let mut timeline = with_tracks(vec![
            vec![
                text_overlay("on-a", "a", 11.0, 1.0),
                video_overlay("v3", "b", 0.0, 3),
            ],
            vec![video_overlay("v1", "b", 1.0, 1)],
        ]);
        assert_eq!(timeline.material_ids(), vec![1, 2, 3]);
        assert!(timeline.uses_material(3));
        assert!(timeline.remove_material(3));
        assert_eq!(timeline.material_ids(), vec![1, 2]);
        // Clip a goes with the overlay anchored to it, and the video of
        // material 1, which empties the top track.
        assert!(timeline.remove_material(1));
        assert!(timeline.tracks.is_empty());
        assert!(!timeline.remove_material(1));
    }

    #[test]
    fn overlays_need_rendering_and_video_overlays_valid_ranges() {
        let mut timeline = Timeline::from_materials(&[(7, 30.0)]);
        let clip_id = timeline.clips[0].id.clone();
        timeline.tracks = vec![OverlayTrack {
            id: "t".into(),
            kind: None,
            items: vec![text_overlay("x", &clip_id, 1.0, 2.0)],
        }];
        assert_eq!(timeline.single_full_material(30.0), None);

        let mut video = with_tracks(vec![vec![video_overlay("v", "a", 10.0, 3)]]);
        assert!(video.validate(|_| Some(30.0), |_| None).is_ok());
        // Shows 1-4 of material 3, which is only 3.2 s long.
        assert!(video
            .validate(|id| Some(if id == 3 { 3.2 } else { 30.0 }), |_| None)
            .is_err());
        assert!(video
            .validate(|id| (id != 3).then_some(30.0), |_| None)
            .is_err());
        let set_volume = |timeline: &mut Timeline, volume: f64| {
            if let OverlayContent::Video(v) = &mut timeline.tracks[0].items[0].content {
                v.volume = volume;
            }
        };
        let mut loud = video.clone();
        set_volume(&mut loud, MAX_VOLUME);
        assert!(loud.validate(|_| Some(30.0), |_| None).is_ok());
        set_volume(&mut loud, MAX_VOLUME + 0.1);
        assert!(loud.validate(|_| Some(30.0), |_| None).is_err());
        if let OverlayContent::Video(v) = &mut video.tracks[0].items[0].content {
            v.element.width = 0.0;
        }
        assert!(video.validate(|_| Some(30.0), |_| None).is_err());
    }

    #[test]
    fn tracks_hold_only_what_their_kind_does() {
        let mut timeline = with_tracks(vec![vec![
            video_overlay("v", "a", 10.0, 3),
            text_overlay("x", "a", 15.0, 2.0),
        ]]);
        // Tracks from before kinds may hold both.
        assert!(timeline.validate(|_| Some(30.0), |_| None).is_ok());
        timeline.tracks[0].kind = Some(TrackKind::Video);
        assert!(timeline.validate(|_| Some(30.0), |_| None).is_err());
        timeline.tracks[0].items.pop();
        assert!(timeline.validate(|_| Some(30.0), |_| None).is_ok());
        let json = timeline.to_json();
        assert!(json.contains(r#""kind":"video","items""#));
        assert_eq!(Timeline::parse(&json).unwrap(), timeline);
    }

    fn sound(id: &str, clip_id: &str, start: f64, duration: f64, source: AudioSource) -> AudioItem {
        AudioItem {
            id: id.into(),
            clip_id: clip_id.into(),
            start,
            duration,
            source,
            source_start: 1.0,
            volume: 0.5,
            fade_in: 0.0,
            fade_out: 0.0,
        }
    }

    fn music(id: &str, clip_id: &str, start: f64, duration: f64) -> AudioItem {
        sound(
            id,
            clip_id,
            start,
            duration,
            AudioSource::File { audio_id: 9 },
        )
    }

    fn with_audio(tracks: Vec<Vec<AudioItem>>) -> Timeline {
        Timeline {
            audio_tracks: tracks
                .into_iter()
                .enumerate()
                .map(|(index, items)| AudioTrack {
                    id: format!("a{index}"),
                    items,
                })
                .collect(),
            ..timeline()
        }
    }

    fn heard(timeline: &Timeline) -> Vec<(String, f64, f64, f64)> {
        timeline
            .placed_audio()
            .iter()
            .map(|p| (p.item.id.clone(), p.start, p.end, p.skip))
            .collect()
    }

    #[test]
    fn audio_and_clip_volumes_are_saved_and_timelines_without_them_as_before() {
        let json = r#"{"version":1,"clips":[{"id":"c","material_id":1,"start":0.0,"end":9.0,"volume":0.25}],
            "subtitles":[],"danmu":false,"audio_tracks":[{"id":"t","items":[
            {"id":"m","clip_id":"c","start":1.0,"duration":4.0,"kind":"file","audio_id":3,
             "source_start":2.0,"volume":0.5,"fade_in":1.5,"fade_out":0.5},
            {"id":"v","clip_id":"c","start":6.0,"duration":2.0,"kind":"material","material_id":4,
             "source_start":0.0,"volume":1.0}]}]}"#;
        let timeline = Timeline::parse(json).unwrap();
        assert_eq!(timeline.clips[0].volume, 0.25);
        let items = &timeline.audio_tracks[0].items;
        assert_eq!(items[0].source, AudioSource::File { audio_id: 3 });
        assert_eq!((items[0].fade_in, items[0].fade_out), (1.5, 0.5));
        assert_eq!(items[1].material_id(), Some(4));
        let reparsed: serde_json::Value = serde_json::from_str(&timeline.to_json()).unwrap();
        let original: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(reparsed, original);
        // Clips at full volume and timelines without audio tracks are saved
        // as before, so their render hashes stay the same.
        let plain = Timeline::from_materials(&[(1, 9.0)]).to_json();
        assert!(!plain.contains("volume") && !plain.contains("audio_tracks"));
        let clip = r#"{"id":"c","material_id":1,"start":0.0,"end":9.0}"#;
        let clip: TimelineClip = serde_json::from_str(clip).unwrap();
        assert_eq!(clip.volume, 1.0);
    }

    #[test]
    fn audio_follows_its_clip_like_an_overlay() {
        // a shows 10-20 at 0-10, b shows 0-5 at 10-15.
        let timeline = with_audio(vec![
            vec![music("x", "a", 18.0, 4.0)],
            vec![music("head", "a", 9.0, 3.0), music("end", "b", 4.0, 9.0)],
            vec![music("gone", "a", 20.0, 1.0)],
        ]);
        assert_eq!(
            heard(&timeline),
            vec![
                ("x".to_string(), 8.0, 12.0, 0.0),
                ("head".to_string(), 0.0, 2.0, 1.0),
                ("end".to_string(), 14.0, 23.0, 0.0),
            ]
        );
        assert_eq!(timeline.duration(), 23.0);
    }

    #[test]
    fn audio_of_a_removed_clip_plays_on_over_the_clip_after_it() {
        // a shows 10-20 at 0-10, b shows 0-5 at 10-15.
        let mut timeline = with_audio(vec![
            vec![
                // Heard from 5 s into clip a.
                music("inside", "a", 15.0, 8.0),
                // Its first 2 s are cut off with the head of clip a.
                music("head", "a", 8.0, 5.0),
                music("on-b", "b", 1.0, 2.0),
            ],
            vec![sound(
                "of-a",
                "b",
                0.0,
                3.0,
                AudioSource::Material { material_id: 1 },
            )],
        ]);
        assert!(timeline.remove_material(1));
        let items = &timeline.audio_tracks[0].items;
        let placed: Vec<_> = items
            .iter()
            .map(|item| {
                (
                    item.id.as_str(),
                    item.clip_id.as_str(),
                    item.start,
                    item.source_start,
                    item.duration,
                )
            })
            .collect();
        assert_eq!(
            placed,
            vec![
                ("inside", "b", 0.0, 1.0, 8.0),
                ("head", "b", 0.0, 3.0, 3.0),
                ("on-b", "b", 1.0, 1.0, 2.0),
            ]
        );
        // The sound of material 1 went with it, and its track too.
        assert_eq!(timeline.audio_tracks.len(), 1);

        // With no clip after it, the sound goes with its clip.
        let mut last = with_audio(vec![vec![music("x", "b", 1.0, 2.0)]]);
        assert!(last.remove_material(2));
        assert!(last.audio_tracks.is_empty());
    }

    #[test]
    fn audio_files_and_materials_heard_are_used_and_leave_with_them() {
        let mut timeline = with_audio(vec![vec![
            music("x", "a", 11.0, 1.0),
            sound("y", "b", 0.0, 1.0, AudioSource::Material { material_id: 5 }),
        ]]);
        assert_eq!(timeline.material_ids(), vec![1, 2, 5]);
        assert_eq!(timeline.audio_ids(), vec![9]);
        assert!(timeline.uses_audio(9) && !timeline.uses_audio(5));
        assert!(timeline.remove_audio(9));
        assert!(!timeline.remove_audio(9));
        assert!(timeline.remove_material(5));
        assert!(timeline.audio_tracks.is_empty());
    }

    #[test]
    fn audio_files_deleted_by_the_backend_leave_the_edited_timeline() {
        let base = with_audio(vec![vec![music("x", "a", 11.0, 1.0)]]);
        let saved = timeline();
        let mut edited = base.clone();
        edited.audio_tracks[0].items[0].volume = 1.0;
        edited.merge_saved_edits(&base, &saved);
        assert!(edited.audio_tracks.is_empty());
    }

    #[test]
    fn audio_needs_rendering_and_valid_ranges_and_volumes() {
        let mut timeline = Timeline::from_materials(&[(7, 30.0)]);
        timeline.clips[0].volume = 0.5;
        assert_eq!(timeline.single_full_material(30.0), None);
        timeline.clips[0].volume = 1.0;
        let clip_id = timeline.clips[0].id.clone();
        timeline.audio_tracks = vec![AudioTrack {
            id: "t".into(),
            items: vec![music("x", &clip_id, 1.0, 2.0)],
        }];
        assert_eq!(timeline.single_full_material(30.0), None);

        let materials = |_| Some(30.0);
        let audio = with_audio(vec![vec![music("x", "a", 10.0, 3.0)]]);
        assert!(audio.validate(materials, |_| Some(4.0)).is_ok());
        // Plays 1-4 of a file 3.2 s long.
        assert!(audio.validate(materials, |_| Some(3.2)).is_err());
        assert!(audio.validate(materials, |_| None).is_err());
        let mut loud = audio.clone();
        loud.audio_tracks[0].items[0].volume = MAX_VOLUME;
        assert!(loud.validate(materials, |_| Some(4.0)).is_ok());
        loud.audio_tracks[0].items[0].volume = MAX_VOLUME + 0.1;
        assert!(loud.validate(materials, |_| Some(4.0)).is_err());
        let mut fade = audio.clone();
        fade.audio_tracks[0].items[0].fade_in = f64::NAN;
        assert!(fade.validate(materials, |_| Some(4.0)).is_err());
        let mut clip = audio.clone();
        clip.clips[1].volume = -0.1;
        assert!(clip.validate(materials, |_| Some(4.0)).is_err());
        // The sound of a material is checked against the material.
        let of_material = with_audio(vec![vec![sound(
            "y",
            "a",
            10.0,
            3.0,
            AudioSource::Material { material_id: 3 },
        )]]);
        assert!(of_material
            .validate(|id| Some(if id == 3 { 3.2 } else { 30.0 }), |_| None)
            .is_err());
    }

    #[test]
    fn srt_is_parsed_into_seconds() {
        let cues = srt_cues(
            "1\n00:00:01,500 --> 00:00:02,000\nHello\n\n2\n00:01:00,000 --> 00:01:01,000\nWorld\n",
        );
        assert_eq!(
            cues,
            vec![
                (1.5, 2.0, "Hello".to_string()),
                (60.0, 61.0, "World".to_string())
            ]
        );
    }
}
