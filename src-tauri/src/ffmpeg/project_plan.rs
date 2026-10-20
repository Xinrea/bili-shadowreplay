//! The segments a project (切片) is rendered in, one FFmpeg run each, before
//! they are joined without encoding again.
//!
//! A run per segment keeps what each FFmpeg run opens bounded however many
//! clips a project has (every clip is an input with its own demuxer, decoder
//! and stretch of command line), and lets segments render side by side.
//!
//! Cuts lie on the output's frame grid and on the sample grid of its sound, by
//! where they fall in the project, so the frames and samples of the segments
//! add up to the project's length, and nothing drifts from the subtitles,
//! danmaku and overlays laid out in project time. Segments end at the cuts
//! between clips; a clip too long for one is cut between its frames, and its
//! sound between samples, where it plays on without a seam.

/// The rate every clip's sound is resampled to.
pub const SAMPLE_RATE: f64 = 48000.0;

/// How much one segment may hold.
#[derive(Debug, Clone, Copy)]
pub struct SegmentLimits {
    /// Seconds of the project. A longer clip is cut into parts this long at most.
    pub seconds: f64,
    /// Parts of clips, each an input of the segment's FFmpeg run.
    pub parts: usize,
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
    pub frames: u64,
    pub samples: u64,
    pub parts: Vec<Part>,
}

impl Segment {
    fn empty(first_frame: u64) -> Self {
        Self {
            first_frame,
            frames: 0,
            samples: 0,
            parts: Vec::new(),
        }
    }

    /// Project time where the segment starts.
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

/// Split clips of `durations`, played back to back, into segments at `fps`.
pub fn plan_segments(durations: &[f64], fps: f64, limits: SegmentLimits) -> Vec<Segment> {
    let longest = ((limits.seconds * fps).floor() as u64).max(1);
    let mut segments = Vec::new();
    let mut current = Segment::empty(0);
    let mut start = 0.0;
    for (clip, duration) in durations.iter().enumerate() {
        let end = start + duration;
        let frames = at(end, fps) - at(start, fps);
        let samples = at(end, SAMPLE_RATE) - at(start, SAMPLE_RATE);
        start = end;
        let pieces = frames.div_ceil(longest).max(1);
        // A cut within the clip goes to the sample at the time of its frame,
        // counted from the clip's start like the frame.
        let cut = |piece: u64| {
            let frame = frames * piece / pieces;
            let sample = if piece == pieces {
                samples
            } else {
                at(frame as f64 / fps, SAMPLE_RATE).min(samples)
            };
            (frame, sample)
        };
        for piece in 0..pieces {
            let ((from, from_sample), (to, to_sample)) = (cut(piece), cut(piece + 1));
            let part = Part {
                clip,
                skip: from as f64 / fps,
                frames: to - from,
                samples: to_sample - from_sample,
            };
            let full =
                current.frames + part.frames > longest || current.parts.len() >= limits.parts;
            if full && !current.parts.is_empty() {
                let next = Segment::empty(current.first_frame + current.frames);
                segments.push(std::mem::replace(&mut current, next));
            }
            current.frames += part.frames;
            current.samples += part.samples;
            current.parts.push(part);
        }
    }
    if !current.parts.is_empty() {
        segments.push(current);
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: SegmentLimits = SegmentLimits {
        seconds: 60.0,
        parts: 16,
    };

    fn totals(segments: &[Segment]) -> (u64, u64) {
        segments.iter().fold((0, 0), |(frames, samples), segment| {
            (frames + segment.frames, samples + segment.samples)
        })
    }

    #[test]
    fn cuts_between_frames_do_not_add_up() {
        let segments = plan_segments(&[1.02; 20], 30.0, LIMITS);
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
        let segments = plan_segments(&durations, 30.0, LIMITS);
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
        assert_eq!(segments[1].end(30.0), 90.0);
        assert!(segments
            .iter()
            .all(|segment| segment.parts.iter().all(|part| part.skip == 0.0)));
    }

    #[test]
    fn a_long_clip_is_cut_between_frames_into_even_parts() {
        // 150.01 s at 29.97 FPS: 4496 frames in three parts.
        let segments = plan_segments(&[5.0, 150.01], 29.97, LIMITS);
        let parts: Vec<_> = segments.iter().flat_map(|segment| &segment.parts).collect();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0].clip, 0);
        assert!(parts[1..].iter().all(|part| part.clip == 1));
        // The 5 s clip and the first part fit one segment; each other part gets its own.
        assert_eq!(segments.len(), 3);
        let frames: Vec<_> = parts[1..].iter().map(|part| part.frames).collect();
        assert_eq!(frames, [1498, 1499, 1499]);
        // Every part starts in the clip where the one before ends, by frame
        // and by sample.
        let mut frame = 0;
        let mut sample = 0;
        for part in &parts[1..] {
            assert!((part.skip - frame as f64 / 29.97).abs() < 1e-9);
            assert_eq!(sample, (part.skip * SAMPLE_RATE).round() as u64);
            frame += part.frames;
            sample += part.samples;
        }
        assert_eq!(sample, (150.01 * SAMPLE_RATE).round() as u64);
        let (frames, samples) = totals(&segments);
        assert_eq!(frames, (155.01_f64 * 29.97).round() as u64);
        assert_eq!(samples, (155.01 * SAMPLE_RATE).round() as u64);
    }

    #[test]
    fn a_clip_shorter_than_a_frame_keeps_its_sound() {
        let segments = plan_segments(&[1.0, 0.01, 1.0], 30.0, LIMITS);
        assert_eq!(segments.len(), 1);
        let part = &segments[0].parts[1];
        assert_eq!((part.frames, part.samples), (0, 480));
        assert_eq!(totals(&segments), (60, 96480));
    }
}
