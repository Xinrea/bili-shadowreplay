//! The segments a project (切片) is rendered in, one FFmpeg run each, before
//! they are joined without encoding again.
//!
//! A run per segment keeps what each FFmpeg run opens bounded however many
//! clips a project has (every clip is an input with its own demuxer, decoder
//! and stretch of command line), and lets segments render side by side.
//!
//! Cuts lie on the output's frame grid by where they fall in the project, so
//! the frames of the segments add up to the project's length, and nothing
//! drifts from the subtitles, danmaku and overlays laid out in project time.
//! Sound is counted in samples from each segment's start instead: it then does
//! not depend on where the segment is, and what that leaves over adds up to
//! microseconds.
//!
//! Segments end at the cuts between clips, before clips picked by their id
//! and wherever the limits call for it. A segment is then found again after
//! edits elsewhere, as long as they move it by whole frames, and an edit only
//! regroups the clips up to the next picked clip. A clip too long for one
//! segment is cut between its frames into segments of its own, where it plays
//! on without a seam.

/// The rate every clip's sound is resampled to.
pub const SAMPLE_RATE: f64 = 48000.0;
/// A segment starts at about one clip in this many, picked by its id.
const ANCHOR_EVERY: u64 = 6;

/// How much one segment may hold.
#[derive(Debug, Clone, Copy)]
pub struct SegmentLimits {
    /// Seconds of the project. A longer clip is cut into parts this long at most.
    pub seconds: f64,
    /// Parts of clips, each an input of the segment's FFmpeg run.
    pub parts: usize,
}

/// A clip as the plan sees it.
pub struct PlanClip<'a> {
    /// The same however the timeline is edited around the clip.
    pub id: &'a str,
    pub duration: f64,
}

/// The part of a clip that a segment shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    /// Index of the clip.
    pub clip: usize,
    /// Seconds into the clip where the part starts.
    pub skip: f64,
    pub frames: u64,
    pub samples: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// The frame of the project the segment starts with.
    pub first_frame: u64,
    /// Project time of the cut the segment starts at, its sound's start.
    pub cut: f64,
    pub frames: u64,
    pub samples: u64,
    pub parts: Vec<Part>,
}

impl Segment {
    fn empty(first_frame: u64, cut: f64) -> Self {
        Self {
            first_frame,
            cut,
            frames: 0,
            samples: 0,
            parts: Vec::new(),
        }
    }

    /// Project time where the segment's first frame is.
    pub fn start(&self, fps: f64) -> f64 {
        self.first_frame as f64 / fps
    }

    /// Project time where the segment ends.
    pub fn end(&self, fps: f64) -> f64 {
        (self.first_frame + self.frames) as f64 / fps
    }
}

/// The step of `rate` per second nearest to `time`.
fn at(time: f64, rate: f64) -> u64 {
    (time * rate).round() as u64
}

/// Whether a segment starts at the clip with `id`, however the clips around
/// it change.
fn anchored(id: &str) -> bool {
    // FNV-1a, the same in every run, unlike the standard library's hasher.
    let hash = id.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    hash % ANCHOR_EVERY == 0
}

/// Split `clips`, played back to back, into segments at `fps`.
pub fn plan_segments(clips: &[PlanClip], fps: f64, limits: SegmentLimits) -> Vec<Segment> {
    let longest = ((limits.seconds * fps).floor() as u64).max(1);
    let mut segments = Vec::new();
    let mut current: Option<Segment> = None;
    let (mut start, mut first_frame) = (0.0, 0);
    for (index, clip) in clips.iter().enumerate() {
        let end = start + clip.duration;
        let frames = at(end, fps) - at(start, fps);
        let pieces = frames.div_ceil(longest).max(1);
        let full = current.as_ref().is_some_and(|segment| {
            segment.frames + frames > longest || segment.parts.len() >= limits.parts
        });
        if pieces > 1 || full || anchored(clip.id) {
            segments.extend(current.take());
        }
        for piece in 0..pieces {
            let (from, to) = (frames * piece / pieces, frames * (piece + 1) / pieces);
            let skip = from as f64 / fps;
            let length = if piece + 1 == pieces {
                clip.duration - skip
            } else {
                (to - from) as f64 / fps
            };
            let segment =
                current.get_or_insert_with(|| Segment::empty(first_frame + from, start + skip));
            let offset = start + skip - segment.cut;
            let samples = at(offset + length, SAMPLE_RATE) - at(offset, SAMPLE_RATE);
            segment.frames += to - from;
            segment.samples += samples;
            segment.parts.push(Part {
                clip: index,
                skip,
                frames: to - from,
                samples,
            });
            if pieces > 1 {
                segments.extend(current.take());
            }
        }
        start = end;
        first_frame += frames;
    }
    segments.extend(current);
    segments
}

/// `count` ids, none of them picked to start a segment.
#[cfg(test)]
pub(super) fn plain_ids(count: usize) -> Vec<String> {
    (0..)
        .map(|index| format!("clip{index}"))
        .filter(|id| !anchored(id))
        .take(count)
        .collect()
}

/// An id picked to start a segment, the first from `anchor{after}` on.
#[cfg(test)]
pub(super) fn anchor_id(after: usize) -> String {
    (after..)
        .map(|index| format!("anchor{index}"))
        .find(|id| anchored(id))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: SegmentLimits = SegmentLimits {
        seconds: 60.0,
        parts: 16,
    };

    fn plan(ids: &[String], durations: &[f64], fps: f64) -> Vec<Segment> {
        let clips: Vec<_> = ids
            .iter()
            .zip(durations)
            .map(|(id, duration)| PlanClip {
                id,
                duration: *duration,
            })
            .collect();
        plan_segments(&clips, fps, LIMITS)
    }

    fn totals(segments: &[Segment]) -> (u64, u64) {
        segments.iter().fold((0, 0), |(frames, samples), segment| {
            (frames + segment.frames, samples + segment.samples)
        })
    }

    /// What each segment shows, by clip id.
    fn shown(ids: &[String], segments: &[Segment]) -> Vec<Vec<(String, u64, u64, u64)>> {
        segments
            .iter()
            .map(|segment| {
                segment
                    .parts
                    .iter()
                    .map(|part| {
                        let skip = (part.skip * 1e6).round() as u64;
                        (ids[part.clip].clone(), skip, part.frames, part.samples)
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn cuts_between_frames_do_not_add_up() {
        let segments = plan(&plain_ids(20), &[1.02; 20], 30.0);
        let parts: Vec<_> = segments.iter().flat_map(|segment| &segment.parts).collect();
        // 30.6 frames each: some clips get 31 and some 30, as their cuts fall.
        let frames: Vec<_> = parts.iter().take(3).map(|part| part.frames).collect();
        assert_eq!(frames, [31, 30, 31]);
        assert!(parts.iter().all(|part| part.samples == 48960));
        assert_eq!(totals(&segments), (612, 979_200));
    }

    #[test]
    fn clips_are_grouped_up_to_the_limits() {
        // 16 clips of 2 s fill a segment's parts; 25 s clips its time.
        let mut durations = vec![2.0; 20];
        durations.extend([25.0, 25.0, 25.0]);
        let segments = plan(&plain_ids(23), &durations, 30.0);
        let clips: Vec<Vec<usize>> = segments
            .iter()
            .map(|segment| segment.parts.iter().map(|part| part.clip).collect())
            .collect();
        assert_eq!(
            clips,
            [(0..16).collect(), vec![16, 17, 18, 19, 20, 21], vec![22]]
        );
        // Each segment starts where the one before ends.
        let starts: Vec<_> = segments.iter().map(|segment| segment.first_frame).collect();
        assert_eq!(starts, [0, 960, 2700]);
        assert_eq!(segments[1].start(30.0), 32.0);
        assert_eq!(segments[1].cut, 32.0);
        assert_eq!(segments[1].end(30.0), 90.0);
        assert!(segments
            .iter()
            .all(|segment| segment.parts.iter().all(|part| part.skip == 0.0)));
    }

    #[test]
    fn picked_clips_start_segments() {
        let mut ids = plain_ids(6);
        ids[2] = anchor_id(0);
        ids[5] = anchor_id(100);
        let segments = plan(&ids, &[3.0; 6], 30.0);
        let clips: Vec<Vec<usize>> = segments
            .iter()
            .map(|segment| segment.parts.iter().map(|part| part.clip).collect())
            .collect();
        assert_eq!(clips, [vec![0, 1], vec![2, 3, 4], vec![5]]);
    }

    #[test]
    fn an_edit_regroups_only_up_to_the_next_picked_clip() {
        let mut ids = plain_ids(30);
        for (index, after) in [(4, 0), (12, 100), (21, 200)] {
            ids[index] = anchor_id(after);
        }
        let durations: Vec<f64> = (0..30).map(|index| 3.0 + (index * 7 % 11) as f64).collect();
        let before = shown(&ids, &plan(&ids, &durations, 30.0));

        // Clip 7 a whole number of frames shorter.
        let mut trimmed = durations.clone();
        trimmed[7] -= 0.1;
        let after = shown(&ids, &plan(&ids, &trimmed, 30.0));
        let unchanged =
            |a: &[Vec<_>], b: &[Vec<_>]| a.iter().filter(|segment| b.contains(segment)).count();
        // All but the segment holding clip 7 are found again.
        assert_eq!(unchanged(&before, &after), before.len() - 1);

        // A clip inserted after clip 7 changes its segment's grouping only.
        let mut inserted_ids = ids.clone();
        inserted_ids.insert(8, plain_ids(31)[30].clone());
        let mut inserted = durations.clone();
        inserted.insert(8, 40.0);
        let after = shown(&inserted_ids, &plan(&inserted_ids, &inserted, 30.0));
        let first_after_edit = before
            .iter()
            .position(|segment| segment[0].0 == ids[12])
            .unwrap();
        for segment in &before[first_after_edit..] {
            assert!(after.contains(segment), "{segment:?}");
        }
        for segment in &before[..1] {
            assert!(after.contains(segment), "{segment:?}");
        }
    }

    #[test]
    fn sound_is_counted_from_each_segment_start() {
        let mut ids = plain_ids(3);
        ids[2] = anchor_id(0);
        let first = plan(&ids, &[2.0, 3.0, 4.2], 30.0);
        // Moved a little: the frames of the last segment may change, its
        // sound does not.
        let moved = plan(&ids, &[2.0137, 3.0, 4.2], 30.0);
        assert_eq!(first.len(), 2);
        assert_eq!(first[1].samples, moved[1].samples);
        assert_eq!(first[1].samples, 201_600);
        assert_eq!(first[1].cut, 5.0);
    }

    #[test]
    fn a_long_clip_is_cut_between_frames_into_segments_of_its_own() {
        // 150.01 s at 29.97 FPS: 4496 frames in three parts.
        let segments = plan(&plain_ids(2), &[5.0, 150.01], 29.97);
        assert_eq!(segments.len(), 4);
        assert_eq!(segments[0].parts.len(), 1);
        let parts: Vec<_> = segments[1..]
            .iter()
            .map(|segment| &segment.parts[0])
            .collect();
        assert!(parts.iter().all(|part| part.clip == 1));
        let frames: Vec<_> = parts.iter().map(|part| part.frames).collect();
        assert_eq!(frames, [1498, 1499, 1499]);
        // Every part starts in the clip where the one before ends, by frame
        // and, to a sample, by sound.
        let (mut frame, mut sample) = (0, 0);
        for (segment, part) in segments[1..].iter().zip(&parts) {
            assert!((part.skip - frame as f64 / 29.97).abs() < 1e-9);
            assert!((sample as f64 - part.skip * SAMPLE_RATE).abs() <= 1.0);
            assert!((segment.cut - 5.0 - part.skip).abs() < 1e-9);
            frame += part.frames;
            sample += part.samples;
        }
        assert_eq!(sample, (150.01 * SAMPLE_RATE).round() as u64);
        let (frames, _) = totals(&segments);
        assert_eq!(frames, (155.01_f64 * 29.97).round() as u64);
    }

    #[test]
    fn a_clip_shorter_than_a_frame_keeps_its_sound() {
        let segments = plan(&plain_ids(3), &[1.0, 0.01, 1.0], 30.0);
        assert_eq!(segments.len(), 1);
        let part = &segments[0].parts[1];
        assert_eq!((part.frames, part.samples), (0, 480));
        assert_eq!(totals(&segments), (60, 96480));
    }
}
