//! Project (切片) timelines.
//!
//! A project plays its clips back to back on one main track. Each clip shows
//! `start..end` of a material. Subtitles are anchored to a clip in material
//! time, so they follow the picture when clips are moved or trimmed; the part
//! outside the clip's visible window is hidden rather than lost.

use recorder::danmu::DanmuEntry;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const TIMELINE_VERSION: u32 = 1;
/// Durations closer than this are treated as the whole material.
const FULL_MATERIAL_TOLERANCE: f64 = 0.05;

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

/// A clip together with where it starts on the project timeline.
pub struct PlacedClip<'a> {
    pub clip: &'a TimelineClip,
    pub offset: f64,
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

    /// Distinct materials in timeline order.
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

    pub fn material_ids(&self) -> Vec<i64> {
        let mut ids = Vec::new();
        for clip in &self.clips {
            if !ids.contains(&clip.material_id) {
                ids.push(clip.material_id);
            }
        }
        ids
    }

    pub fn uses_material(&self, material_id: i64) -> bool {
        self.clips
            .iter()
            .any(|clip| clip.material_id == material_id)
    }

    /// Drop the material's clips and the subtitles anchored to them.
    pub fn remove_material(&mut self, material_id: i64) -> bool {
        let removed: Vec<String> = self
            .clips
            .iter()
            .filter(|clip| clip.material_id == material_id)
            .map(|clip| clip.id.clone())
            .collect();
        if removed.is_empty() {
            return false;
        }
        self.clips.retain(|clip| clip.material_id != material_id);
        self.subtitles
            .retain(|subtitle| !removed.contains(&subtitle.clip_id));
        true
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
        (untouched && !self.danmu && !self.has_visible_subtitles()).then_some(clip.material_id)
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
    /// start) onto the project timeline.
    pub fn map_danmus<'a>(
        &self,
        material_danmus: impl Fn(i64) -> Option<&'a [DanmuEntry]>,
    ) -> Vec<DanmuEntry> {
        let mut mapped = Vec::new();
        for placed in self.placed() {
            let Some(danmus) = material_danmus(placed.clip.material_id) else {
                continue;
            };
            let start = (placed.clip.start * 1000.0).round() as i64;
            let end = (placed.clip.end * 1000.0).round() as i64;
            let offset = (placed.offset * 1000.0).round() as i64;
            mapped.extend(
                danmus
                    .iter()
                    .filter(|danmu| danmu.ts >= start && danmu.ts < end)
                    .map(|danmu| DanmuEntry {
                        ts: danmu.ts - start + offset,
                        ..danmu.clone()
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
        let mapped = timeline.map_danmus(|id| match id {
            1 => Some(&first[..]),
            2 => Some(&second[..]),
            _ => None,
        });
        let times: Vec<i64> = mapped.iter().map(|d| d.ts).collect();
        assert_eq!(times, vec![0, 9_999, 11_000]);
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
