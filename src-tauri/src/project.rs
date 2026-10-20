//! Project (切片) timelines.
//!
//! A project plays its clips back to back on one main track. Each clip shows
//! `start..end` of a material. Subtitles are anchored to a clip in material
//! time, so they follow the picture when clips are moved or trimmed; the part
//! outside the clip's visible window is hidden rather than lost.
//!
//! Overlays (texts, stickers and videos) sit on tracks above the main one,
//! a higher track covering lower ones. Each is anchored by where it starts,
//! like a subtitle, and runs for its own length, also over later clips.

use recorder::danmu::DanmuEntry;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const TIMELINE_VERSION: u32 = 1;
/// Durations closer than this are treated as the whole material.
const FULL_MATERIAL_TOLERANCE: f64 = 0.05;
/// Overlay spans this short, from rounding, are empty.
const EPSILON: f64 = 1e-6;

fn timeline_version() -> u32 {
    TIMELINE_VERSION
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
    #[serde(default)]
    pub items: Vec<Overlay>,
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
    /// 0 to 1.
    #[serde(default = "full_volume")]
    pub volume: f64,
    pub element: VideoPlacement,
}

fn full_volume() -> f64 {
    1.0
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

    pub fn duration(&self) -> f64 {
        self.clips.iter().map(TimelineClip::duration).sum()
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
        let total = self.duration();
        let mut result = Vec::new();
        for overlay in self.tracks.iter().flat_map(|track| &track.items) {
            let Some(clip) = placed.iter().find(|p| p.clip.id == overlay.clip_id) else {
                continue;
            };
            // Anchored past the out point, it was trimmed away with its clip.
            if overlay.start >= clip.clip.end {
                continue;
            }
            let from = clip.offset + overlay.start - clip.clip.start;
            let start = from.max(clip.offset);
            let end = (from + overlay.duration).min(total);
            if end - start > EPSILON {
                result.push(PlacedOverlay {
                    overlay,
                    start,
                    end,
                    skip: start - from,
                });
            }
        }
        result
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
        let remaining = saved.material_ids();
        for material_id in base.material_ids() {
            if !remaining.contains(&material_id) {
                self.remove_material(material_id);
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
    /// video overlays.
    pub fn material_ids(&self) -> Vec<i64> {
        let mut ids = Vec::new();
        let overlays = self
            .tracks
            .iter()
            .flat_map(|track| &track.items)
            .filter_map(Overlay::video)
            .map(|video| video.material_id);
        for id in self
            .clips
            .iter()
            .map(|clip| clip.material_id)
            .chain(overlays)
        {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids
    }

    pub fn uses_material(&self, material_id: i64) -> bool {
        self.material_ids().contains(&material_id)
    }

    /// Drop the material's clips with the subtitles and overlays anchored to
    /// them, and the video overlays showing it.
    pub fn remove_material(&mut self, material_id: i64) -> bool {
        let removed: Vec<String> = self
            .clips
            .iter()
            .filter(|clip| clip.material_id == material_id)
            .map(|clip| clip.id.clone())
            .collect();
        let overlays = self.overlay_count();
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
        !removed.is_empty() || self.overlay_count() != overlays
    }

    fn overlay_count(&self) -> usize {
        self.tracks.iter().map(|track| track.items.len()).sum()
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

    /// Check clip ranges against the materials' lengths.
    pub fn validate(&self, material_duration: impl Fn(i64) -> Option<f64>) -> Result<(), String> {
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
                && (duration <= 0.0 || clip.end <= duration + 0.5);
            if !valid {
                return Err(format!("第 {} 个片段的范围无效", index + 1));
            }
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
                && [placement.x, placement.y, placement.rotation, video.volume]
                    .iter()
                    .all(|value| value.is_finite())
                && placement.width.is_finite()
                && placement.width >= 1.0;
            if !valid {
                return Err("画中画的范围无效".to_string());
            }
        }
        Ok(())
    }

    /// The material when the whole project is one untouched material, which
    /// can be uploaded as-is.
    pub fn single_full_material(&self, material_duration: f64) -> Option<i64> {
        let [clip] = self.clips.as_slice() else {
            return None;
        };
        let untouched = clip.start <= FULL_MATERIAL_TOLERANCE
            && material_duration > 0.0
            && clip.end >= material_duration - FULL_MATERIAL_TOLERANCE;
        (untouched && !self.danmu && !self.has_visible_subtitles() && !self.has_visible_overlays())
            .then_some(clip.material_id)
    }

    /// Identifies an export: a different hash means the output is stale.
    pub fn render_hash(&self, extra: &str) -> String {
        let mut hasher = Sha256::new();
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
    fn only_an_untouched_single_material_skips_export() {
        let mut timeline = Timeline::from_materials(&[(7, 30.0)]);
        assert_eq!(timeline.single_full_material(30.0), Some(7));
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
        assert!(timeline.validate(|_| Some(30.0)).is_ok());
        assert!(timeline.validate(|id| (id == 1).then_some(30.0)).is_err());
        assert!(timeline.validate(|_| Some(12.0)).is_err());
        assert!(Timeline::default().validate(|_| Some(1.0)).is_err());
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
                // Cut short at the end of the project.
                ("end".to_string(), 14.0, 15.0, 0.0),
            ]
        );
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
            items: vec![text_overlay("x", &clip_id, 1.0, 2.0)],
        }];
        assert_eq!(timeline.single_full_material(30.0), None);

        let mut video = with_tracks(vec![vec![video_overlay("v", "a", 10.0, 3)]]);
        assert!(video.validate(|_| Some(30.0)).is_ok());
        // Shows 1-4 of material 3, which is only 3.2 s long.
        assert!(video
            .validate(|id| Some(if id == 3 { 3.2 } else { 30.0 }))
            .is_err());
        assert!(video.validate(|id| (id != 3).then_some(30.0)).is_err());
        if let OverlayContent::Video(v) = &mut video.tracks[0].items[0].content {
            v.element.width = 0.0;
        }
        assert!(video.validate(|_| Some(30.0)).is_err());
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
