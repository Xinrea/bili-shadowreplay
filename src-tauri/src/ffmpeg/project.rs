//! Render a project (切片) timeline into one video.
//!
//! The timeline is rendered in segments of up to a minute (see
//! `project_plan`), each by its own FFmpeg run, a few at a time, into a video
//! file and a WAV file of its sound. The videos are then joined without
//! encoding them again, and the sound is encoded once for the whole project.
//!
//! In a segment, every part of a clip is a separate input seeked with
//! `-ss/-t`, so FFmpeg only decodes what is used even when the material is
//! hours long. Parts are normalized to one canvas, frame rate and audio
//! format, joined with `concat`, and then overlays, subtitles and danmaku are
//! drawn on them, in that order. Video overlays are inputs of their own like
//! clips, and their sound is mixed into the clips'. The frames carry project
//! time while they are drawn on, so the overlays' spans, the subtitles and the
//! danmaku apply unchanged in every segment.
//!
//! A project's segments are kept in its segment cache by a key of everything
//! they are made of, with times from their first frame. A later render uses
//! those it finds again, with edits elsewhere in the timeline that move them by
//! whole frames, and renders only the others.

use std::collections::HashMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use ffmpeg_utils::ffmpeg_command;
use futures::{stream, TryStreamExt};
use recorder::danmu::DanmuEntry;
use sha2::{Digest, Sha256};

use crate::danmu2ass::{Danmu2AssOptions, ImageDanmu, R2L_TIME};
use crate::progress::progress_reporter::{ProgressReporterTrait, StageProgress};

use super::{
    danmu, hwaccel,
    project_plan::{plan_segments, PlanClip, Segment, SegmentLimits, SAMPLE_RATE},
    runner::{run_command, ProgressMode, ProgressTotal, TempArtifact},
};

/// Output frame rate when no clip reports one.
const DEFAULT_FPS: f64 = 30.0;
const MAX_FPS: f64 = 60.0;
const SEGMENT_LIMITS: SegmentLimits = SegmentLimits {
    seconds: 60.0,
    parts: 16,
};
/// Segments rendering at once. Encoders use several threads already, but the
/// filters drawing on the picture mostly one, and consumer GPUs allow only a
/// few encoding sessions.
const PARALLEL_SEGMENTS: usize = 2;
/// Inputs are read from this long before their in point: a decoder starting
/// there, as an AAC one does, first needs what comes before.
const PREROLL: f64 = 1.0;
/// A video overlay is read from at least this long before the end of its
/// file, so that its last frame is there to hold after the file ends.
const LAST_FRAME_MARGIN: f64 = 1.0;
/// Overlaps this short, from rounding, do not count.
const EPSILON: f64 = 1e-6;
/// The stage segments report their progress in.
const RENDER_STAGE: &str = "导出切片";
/// Part of every segment's key: change it with what a segment's graph draws,
/// so that segments rendered before are not used again.
const RENDER_VERSION: u32 = 1;

pub struct RenderClip {
    /// The same however the timeline is edited around the clip.
    pub id: String,
    pub path: PathBuf,
    /// In point within the file, in seconds.
    pub start: f64,
    pub duration: f64,
    pub has_audio: bool,
    /// Frame rate of the file, `0.0` when unknown.
    pub fps: f64,
    /// Length of the file's video, `0.0` when unknown.
    pub file_duration: f64,
}

/// Something laid over the joined clips from `start` to `end`, project time.
pub struct RenderOverlay {
    pub start: f64,
    pub end: f64,
    pub layer: OverlayLayer,
}

pub enum OverlayLayer {
    /// A PNG the editor drew, placed with its top left at `x`, `y`.
    Image {
        png: Vec<u8>,
        x: f64,
        y: f64,
    },
    Video(VideoLayer),
}

pub struct VideoLayer {
    pub path: PathBuf,
    /// In point within the file, in seconds.
    pub source_start: f64,
    /// Length of the file's video, `0.0` when unknown.
    pub duration: f64,
    pub has_audio: bool,
    /// Center on the 1920×1080 picture.
    pub x: f64,
    pub y: f64,
    /// The height follows the file's aspect ratio.
    pub width: f64,
    /// Degrees, clockwise.
    pub rotation: f64,
    /// 0 to 1.
    pub volume: f64,
}

pub struct ProjectRender<'a> {
    pub clips: Vec<RenderClip>,
    /// Drawn in order, so each covers those before it.
    pub overlays: Vec<RenderOverlay>,
    /// ASS document in project time, exactly as previewed by the editor.
    pub subtitle_ass: Option<String>,
    /// Font files the preview used for the subtitles.
    pub fonts_directory: Option<&'a Path>,
    /// Danmaku in project time; empty to burn none.
    pub danmus: Vec<DanmuEntry>,
    pub danmu_options: Danmu2AssOptions,
    pub output: &'a Path,
    /// Where the project's segments are kept for later renders, which use
    /// those unchanged again. Without it they are rendered in a temporary
    /// folder.
    pub segment_cache: Option<&'a Path>,
}

fn output_fps(clips: &[RenderClip], danmu: bool) -> f64 {
    project_frame_rate(clips.iter().map(|clip| clip.fps), danmu)
}

/// The frame rate a project of clips whose files have `rates` is rendered at,
/// with danmaku burned or not: the fastest within limits. The editor puts
/// edits on its frames.
pub fn project_frame_rate(rates: impl IntoIterator<Item = f64>, danmu: bool) -> f64 {
    let fps = rates
        .into_iter()
        .filter(|fps| fps.is_finite() && *fps > 0.0)
        .fold(0.0, f64::max);
    let fps = if fps > 0.0 {
        fps.min(MAX_FPS)
    } else {
        DEFAULT_FPS
    };
    if danmu {
        danmu::danmu_output_fps(fps)
    } else {
        // Keep the usual rational rates (29.97, 59.94) exact enough for ffmpeg.
        (fps * 1000.0).round() / 1000.0
    }
}

/// Where the input of a part of `clip` starting `skip` seconds into it is
/// seeked to in its file, and how long before the part that is. Near the end
/// of the file that is read from its last frames, to hold the last one.
fn part_input(clip: &RenderClip, skip: f64) -> (f64, f64) {
    let start = clip.start + skip;
    let from = if clip.file_duration > 0.0 {
        start.min(clip.file_duration - LAST_FRAME_MARGIN)
    } else {
        start
    };
    let seek = (from - PREROLL).max(0.0);
    (seek, start - seek)
}

/// Filter graph joining the parts of `segment` into `[joined]` (video) and
/// `[{audio}]`.
///
/// Each part is padded and cut to its length in the plan, so a file a little
/// shorter than its clip cannot shift the subtitles and danmaku of the clips
/// after it. Frame `n` of a part shows its file `n / fps` past the part's in
/// point, however the part was cut from its clip, so the parts of one clip
/// play on without a seam. Pictures and sound are joined apart: one concat
/// would stretch every part to its longer stream, and those differences add up.
fn concat_filter(clips: &[RenderClip], segment: &Segment, fps: f64, audio: &str) -> String {
    let mut filter = String::new();
    let mut pictures = String::new();
    let mut sounds = String::new();
    for (index, part) in segment.parts.iter().enumerate() {
        let clip = &clips[part.clip];
        let (_, preroll) = part_input(clip, part.skip);
        let (frames, samples) = (part.frames, part.samples);
        // Moved `lead` later, the in point is frame `first` of the grid. The
        // frames before it are kept until it, so that a file ending before the
        // part still has its last frame to hold.
        let first = (preroll * fps).ceil() as u64;
        let lead = first as f64 / fps - preroll;
        let padding = (first + frames) as f64 / fps;
        filter.push_str(&format!(
            "[{index}:v]setpts=PTS+{lead:.6}/TB,fps={fps}:start_time=0,\
             tpad=stop_mode=clone:stop_duration={padding:.6},\
             trim=start_frame={first}:end_frame={},setpts=PTS-STARTPTS,{},setsar=1,\
             format=yuv420p[pv{index}];\n",
            first + frames,
            hwaccel::H264_SCALE_PAD_FILTER,
        ));
        if clip.has_audio {
            filter.push_str(&format!(
                "[{index}:a]aresample={SAMPLE_RATE}:async=1:first_pts=0,atrim=start_sample={},\
                 aformat=sample_fmts=fltp:channel_layouts=stereo,asetpts=PTS-STARTPTS,\
                 apad,atrim=end_sample={samples}[pa{index}];\n",
                (preroll * SAMPLE_RATE).round() as u64,
            ));
        } else {
            filter.push_str(&format!(
                "anullsrc=r={SAMPLE_RATE}:cl=stereo,atrim=end_sample={samples},\
                 aformat=sample_fmts=fltp:channel_layouts=stereo[pa{index}];\n"
            ));
        }
        pictures.push_str(&format!("[pv{index}]"));
        sounds.push_str(&format!("[pa{index}]"));
    }
    // Numbered from the segment's first frame, the frames carry project time,
    // which the overlays, subtitles and danmaku are laid out in. Concat would
    // also time a part of one frame as lasting none.
    let (count, first) = (segment.parts.len(), segment.first_frame);
    filter.push_str(&format!(
        "{pictures}concat=n={count}:v=1:a=0,setpts=(N+{first})/{fps}/TB[joined];\n\
         {sounds}concat=n={count}:v=0:a=1[{audio}];\n"
    ));
    filter
}

/// The part of an overlay within a segment.
struct SegmentOverlay<'a> {
    /// Index among the project's overlays.
    index: usize,
    overlay: &'a RenderOverlay,
    /// Where it is shown within the segment, project time.
    start: f64,
    end: f64,
}

impl SegmentOverlay<'_> {
    /// Where the input of a video overlay is seeked to in its file, and how
    /// much of the input comes before what is shown. Near the end of the file
    /// that is read from its last frames, to hold the last one.
    fn seek(&self, layer: &VideoLayer) -> (f64, f64) {
        let offset = layer.source_start + self.start - self.overlay.start;
        let from = if layer.duration > 0.0 {
            offset.min(layer.duration - LAST_FRAME_MARGIN)
        } else {
            offset
        };
        let seek = (from - PREROLL).max(0.0);
        (seek, offset - seek)
    }
}

/// The overlays shown during project time `start..end`, in order.
fn segment_overlays(overlays: &[RenderOverlay], start: f64, end: f64) -> Vec<SegmentOverlay<'_>> {
    overlays
        .iter()
        .enumerate()
        .filter_map(|(index, overlay)| {
            let (from, to) = (overlay.start.max(start), overlay.end.min(end));
            (to - from > EPSILON).then_some(SegmentOverlay {
                index,
                overlay,
                start: from,
                end: to,
            })
        })
        .collect()
}

/// Image danmaku on screen during project time `start..end`.
fn segment_danmaku(images: &[ImageDanmu], start: f64, end: f64) -> Vec<ImageDanmu> {
    images
        .iter()
        .filter(|image| image.start < end && image.start + R2L_TIME > start)
        .cloned()
        .collect()
}

/// An overlay graph laid over `[{video}]`, with the inputs of `overlays`
/// starting at `first_input`.
struct OverlayGraph {
    filter: String,
    /// The video with every overlay on it.
    video: String,
    /// The sound of each video overlay, as long as the segment with silence
    /// around it.
    audio: Vec<String>,
}

/// Lay `overlays` over a segment starting at project time `start` and
/// `samples` long.
fn overlay_filter(
    overlays: &[SegmentOverlay],
    first_input: usize,
    fps: f64,
    (start, samples): (f64, u64),
    video: &str,
) -> OverlayGraph {
    let mut filter = String::new();
    let mut current = video.to_string();
    let mut audio = Vec::new();
    for (index, shown) in overlays.iter().enumerate() {
        let input = first_input + index;
        let (from, to) = (shown.start, shown.end);
        let enable = format!("enable='gte(t,{from:.6})*lt(t,{to:.6})'");
        match &shown.overlay.layer {
            OverlayLayer::Image { x, y, .. } => {
                filter.push_str(&format!(
                    "[{input}:v]format=rgba[ov{index}];\n\
                     [{current}][ov{index}]overlay=x={}:y={}:{enable}[layer{index}];\n",
                    x.round() as i64,
                    y.round() as i64,
                ));
            }
            OverlayLayer::Video(layer) => {
                let (_, skip) = shown.seek(layer);
                let length = to - from;
                // Even, as the encoder's chroma planes need.
                let width = ((layer.width / 2.0).round() * 2.0).max(2.0) as i64;
                let rotate = if layer.rotation.abs() > 0.01 {
                    let angle = layer.rotation.to_radians();
                    // Transparent corners: `none` would leave them unpainted.
                    format!(",rotate={angle:.6}:c=black@0:ow=rotw({angle:.6}):oh=roth({angle:.6})")
                } else {
                    String::new()
                };
                // Shifted to its place on the timeline; the clips show through
                // before it starts. A file ending early holds its last frame
                // until the overlay ends, as the preview does.
                filter.push_str(&format!(
                    "[{input}:v]fps={fps}:start_time=0,\
                     tpad=stop_mode=clone:stop_duration={:.6},trim=start={skip:.6}:duration={length:.6},\
                     setpts=PTS-STARTPTS,scale={width}:-2,setsar=1,\
                     format=yuva420p{rotate},setpts=PTS+{from:.6}/TB[ov{index}];\n\
                     [{current}][ov{index}]overlay=x={:.3}-overlay_w/2:y={:.3}-overlay_h/2:\
                     eof_action=repeat:{enable}[layer{index}];\n",
                    skip + length,
                    layer.x,
                    layer.y,
                ));
                if layer.has_audio && layer.volume > 0.0 {
                    let delay = ((from - start) * 1000.0).round() as i64;
                    filter.push_str(&format!(
                        "[{input}:a]aresample={SAMPLE_RATE}:async=1:first_pts=0,atrim=start_sample={},\
                         aformat=sample_fmts=fltp:channel_layouts=stereo,asetpts=PTS-STARTPTS,\
                         atrim=duration={length:.6},volume={:.3},adelay={delay}|{delay},\
                         apad,atrim=end_sample={samples}[oa{index}];\n",
                        (skip * SAMPLE_RATE).round() as u64,
                        layer.volume.clamp(0.0, 1.0),
                    ));
                    audio.push(format!("oa{index}"));
                }
            }
        }
        current = format!("layer{index}");
    }
    OverlayGraph {
        filter,
        video: current,
        audio,
    }
}

/// Mix `[main]` with the overlays' sound into `[outa]`. Every input lasts the
/// whole segment, so amix scales each by as much and the volume filter
/// restores them: overlays add to the clips' sound instead of lowering it.
fn mix_filter(main: &str, overlays: &[String]) -> String {
    let inputs: String = std::iter::once(main)
        .chain(overlays.iter().map(String::as_str))
        .map(|label| format!("[{label}]"))
        .collect();
    let count = overlays.len() + 1;
    format!(
        "{inputs}amix=inputs={count}:duration=first:dropout_transition=0,volume={count}[outa];\n"
    )
}

/// The overlay picture's file in the render's directory, a name of our own.
fn overlay_image_name(index: usize) -> String {
    format!("overlay{index}.png")
}

fn segment_video_name(key: &str) -> String {
    format!("{key}.mp4")
}

fn segment_sound_name(key: &str) -> String {
    format!("{key}.wav")
}

/// How far a segment has rendered, and how fast while it renders.
#[derive(Clone, Copy, Default)]
struct Rendered {
    seconds: f64,
    speed: Option<f64>,
}

/// Reports the segments rendering at once as one stage of the whole project.
#[derive(Clone)]
struct SegmentProgress<'a, R> {
    reporter: &'a R,
    segments: Arc<Mutex<Vec<Rendered>>>,
    total: f64,
    /// The segment this reports, and its length.
    index: usize,
    length: f64,
}

impl<'a, R: ProgressReporterTrait> SegmentProgress<'a, R> {
    fn new(reporter: &'a R, segments: usize, total: f64) -> Self {
        Self {
            reporter,
            segments: Arc::new(Mutex::new(vec![Rendered::default(); segments])),
            total,
            index: 0,
            length: 0.0,
        }
    }

    fn segment(&self, index: usize, length: f64) -> Self {
        Self {
            index,
            length,
            ..self.clone()
        }
    }

    /// Note how far this segment is, and return how far all are together
    /// and how fast those rendering go.
    fn record(&self, seconds: f64, speed: Option<f64>) -> Rendered {
        let mut segments = self.segments.lock().unwrap_or_else(PoisonError::into_inner);
        segments[self.index] = Rendered {
            seconds: seconds.min(self.length),
            speed,
        };
        let speeds: Vec<f64> = segments
            .iter()
            .filter_map(|segment| segment.speed)
            .collect();
        Rendered {
            seconds: segments.iter().map(|segment| segment.seconds).sum(),
            speed: (!speeds.is_empty()).then(|| speeds.iter().sum()),
        }
    }

    async fn finished(&self) {
        self.progress(StageProgress {
            stage: RENDER_STAGE.to_string(),
            processed: self.length,
            total: Some(self.length),
            speed: None,
        })
        .await;
    }
}

#[async_trait]
impl<R: ProgressReporterTrait> ProgressReporterTrait for SegmentProgress<'_, R> {
    async fn update(&self, content: &str) {
        self.reporter.update(content).await;
    }

    async fn progress(&self, progress: StageProgress) {
        let all = self.record(progress.processed, progress.speed);
        self.reporter
            .progress(StageProgress {
                stage: progress.stage,
                processed: all.seconds,
                total: Some(self.total),
                speed: all.speed,
            })
            .await;
    }

    // The export reports its end, not each segment.
    async fn finish(&self, _success: bool, _message: &str) {}
}

/// An ASS document: its events with their times, and the rest of it.
struct AssDocument {
    /// Everything but the events, in order.
    header: String,
    events: Vec<AssEvent>,
}

struct AssEvent {
    start: f64,
    end: f64,
    /// The line as written.
    line: String,
    /// The line without its times: what it shows.
    content: String,
}

/// Seconds of an ASS time, `H:MM:SS.CC`.
fn ass_time(value: &str) -> Option<f64> {
    let mut parts = value.trim().split(':');
    let (hours, minutes, seconds) = (parts.next()?, parts.next()?, parts.next()?);
    let hours: f64 = hours.parse().ok()?;
    let minutes: f64 = minutes.parse().ok()?;
    let seconds: f64 = seconds.parse().ok()?;
    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

impl AssDocument {
    fn parse(text: &str) -> Self {
        let mut header = String::new();
        let mut events = Vec::new();
        let mut in_events = false;
        // Where Start and End are among an event's fields, and how many there are.
        let mut fields: Option<(usize, usize, usize)> = None;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                in_events = trimmed.eq_ignore_ascii_case("[Events]");
            } else if in_events {
                if let Some(format) = trimmed.strip_prefix("Format:") {
                    let names: Vec<&str> = format.split(',').map(str::trim).collect();
                    let position = |name: &str| {
                        names
                            .iter()
                            .position(|field| field.eq_ignore_ascii_case(name))
                    };
                    fields = position("Start")
                        .zip(position("End"))
                        .map(|(start, end)| (start, end, names.len()));
                } else if let (Some(dialogue), Some((start, end, count))) =
                    (trimmed.strip_prefix("Dialogue:"), fields)
                {
                    let values: Vec<&str> = dialogue.trim_start().splitn(count, ',').collect();
                    let time = |index: usize| values.get(index).and_then(|value| ass_time(value));
                    if let (Some(from), Some(to)) = (time(start), time(end)) {
                        let content: Vec<&str> = values
                            .iter()
                            .enumerate()
                            .filter(|(index, _)| *index != start && *index != end)
                            .map(|(_, value)| *value)
                            .collect();
                        events.push(AssEvent {
                            start: from,
                            end: to,
                            line: line.to_string(),
                            content: content.join(","),
                        });
                        continue;
                    }
                }
            }
            header.push_str(line);
            header.push('\n');
        }
        Self { header, events }
    }

    /// The events shown during project time `start..end`.
    fn during(&self, start: f64, end: f64) -> impl Iterator<Item = &AssEvent> {
        self.events
            .iter()
            .filter(move |event| event.start < end && event.end > start)
    }

    /// The document with only the events shown during `start..end`.
    fn excerpt(&self, start: f64, end: f64) -> String {
        let mut text = self.header.clone();
        for event in self.during(start, end) {
            text.push_str(&event.line);
            text.push('\n');
        }
        text
    }

    /// Note in `key` what the document shows on `frames` frames from project
    /// time `start`: for each event what it shows on which of them, and the
    /// header with the styles they use, if any is shown.
    fn describe(&self, key: &mut String, start: f64, fps: f64, frames: u64) {
        // The first frame at or after `time`.
        let frame = |time: f64| {
            let frame = ((time - start) * fps - EPSILON).ceil().max(0.0) as u64;
            frame.min(frames)
        };
        let mut shown = String::new();
        for event in self.during(start, start + frames as f64 / fps) {
            let (first, last) = (frame(event.start), frame(event.end));
            if first < last {
                let _ = writeln!(shown, "{first} {last} {}", event.content);
            }
        }
        if !shown.is_empty() {
            key.push_str(&self.header);
            key.push_str(&shown);
        }
    }
}

/// A hex digest of `bytes`, long enough to tell any two apart.
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A file as it is now: its path, length and when it last changed.
async fn file_identity(path: &Path) -> String {
    let changed = |metadata: &std::fs::Metadata| {
        metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_secs())
    };
    match tokio::fs::metadata(path).await {
        Ok(metadata) => format!(
            "{} {} {}",
            path.display(),
            metadata.len(),
            changed(&metadata)
        ),
        Err(_) => path.display().to_string(),
    }
}

fn is_digest(text: &str) -> bool {
    text.len() == 32 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Whether `name` is a segment's video or sound file, named by its key.
fn is_segment_file(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .and_then(|name| {
            name.strip_suffix(".mp4")
                .or_else(|| name.strip_suffix(".wav"))
        })
        .is_some_and(is_digest)
}

/// Whether `name` is a decoded danmaku image, named by its URL.
fn is_image_file(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .and_then(|name| name.strip_prefix("image-")?.strip_suffix(".png"))
        .is_some_and(is_digest)
}

/// Removes what a render writes in its directory besides segments and danmaku
/// images when it ends, done or not: graphs, subtitles, pictures and lists.
struct Scratch<'a>(&'a Path);

impl Drop for Scratch<'_> {
    fn drop(&mut self) {
        let Ok(entries) = std::fs::read_dir(self.0) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            if !is_segment_file(&name) && !is_image_file(&name) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

struct Subtitles {
    /// Options of their `ass` filter.
    options: String,
    document: AssDocument,
}

struct Danmaku {
    document: AssDocument,
    /// Image danmaku of the whole project.
    images: Vec<ImageDanmu>,
    /// A digest of each image's file.
    pictures: HashMap<PathBuf, String>,
}

/// What the segments of one render share.
struct Render<'a> {
    job: &'a ProjectRender<'a>,
    /// FFmpeg's working directory, with the files the graphs read.
    directory: &'a Path,
    fps: f64,
    encoder: &'static str,
    /// What each input file is now, by path.
    files: HashMap<PathBuf, String>,
    /// A digest of each overlay's picture, by its index.
    pictures: Vec<Option<String>>,
    subtitles: Option<Subtitles>,
    danmaku: Option<Danmaku>,
}

impl Render<'_> {
    fn file(&self, path: &Path) -> &str {
        self.files.get(path).map_or("", String::as_str)
    }

    /// The key of `segment`: everything its files are made of, with times
    /// from its first frame, so that it is the same for the segment moved by
    /// whole frames and its files are used again.
    fn key(&self, segment: &Segment, overlays: &[SegmentOverlay]) -> String {
        let (job, fps) = (self.job, self.fps);
        let start = segment.start(fps);
        let mut key = format!(
            "{RENDER_VERSION} {fps} {} {}\n{} {}\n",
            self.encoder,
            hwaccel::quality_args_for_encoder(self.encoder).join(" "),
            segment.frames,
            segment.samples,
        );
        for part in &segment.parts {
            let clip = &job.clips[part.clip];
            let _ = writeln!(
                key,
                "clip {} {:.6} {:.6} {} {} {}",
                self.file(&clip.path),
                clip.start + part.skip,
                clip.file_duration,
                clip.has_audio,
                part.frames,
                part.samples,
            );
        }
        for shown in overlays {
            let (from, to) = (shown.start - start, shown.end - start);
            let _ = match &shown.overlay.layer {
                OverlayLayer::Image { x, y, .. } => writeln!(
                    key,
                    "picture {from:.6} {to:.6} {} {x} {y}",
                    self.pictures[shown.index].as_deref().unwrap_or_default(),
                ),
                OverlayLayer::Video(layer) => writeln!(
                    key,
                    "video {from:.6} {to:.6} {} {:.6} {:.6} {} {} {} {} {} {}",
                    self.file(&layer.path),
                    layer.source_start + shown.start - shown.overlay.start,
                    layer.duration,
                    layer.has_audio,
                    layer.x,
                    layer.y,
                    layer.width,
                    layer.rotation,
                    layer.volume,
                ),
            };
        }
        if let Some(subtitles) = &self.subtitles {
            key.push_str("subtitles\n");
            subtitles
                .document
                .describe(&mut key, start, fps, segment.frames);
        }
        if let Some(danmaku) = &self.danmaku {
            let options = &job.danmu_options;
            let _ = writeln!(key, "danmaku {} {}", options.font_size, options.opacity);
            danmaku
                .document
                .describe(&mut key, start, fps, segment.frames);
            for image in segment_danmaku(&danmaku.images, start, segment.end(fps)) {
                // By its first frame, like the text: its time is rounded to
                // the millisecond.
                let frame = ((image.start - start) * fps - EPSILON).ceil();
                let _ = writeln!(
                    key,
                    "{frame} {:.3} {:.3} {:.3} {}",
                    image.top,
                    image.image.width,
                    image.image.height,
                    danmaku
                        .pictures
                        .get(&image.image.path)
                        .map_or("", String::as_str),
                );
            }
        }
        digest(key.as_bytes())
    }

    /// Render segment `index` into the video and sound files named by `key`.
    async fn segment<R: ProgressReporterTrait>(
        &self,
        index: usize,
        segment: &Segment,
        key: &str,
        reporter: &R,
    ) -> Result<(), String> {
        let (job, fps) = (self.job, self.fps);
        let (start, end) = (segment.start(fps), segment.end(fps));
        let overlays = segment_overlays(&job.overlays, start, end);
        let graph = overlay_filter(
            &overlays,
            segment.parts.len(),
            fps,
            (start, segment.samples),
            "joined",
        );
        let joined_audio = if graph.audio.is_empty() {
            "outa"
        } else {
            "maina"
        };
        let mut filter = concat_filter(&job.clips, segment, fps, joined_audio);
        filter.push_str(&graph.filter);
        if !graph.audio.is_empty() {
            filter.push_str(&mix_filter(joined_audio, &graph.audio));
        }
        let mut current = graph.video;
        if let Some(subtitles) = &self.subtitles {
            filter.push_str(&format!(
                "[{current}]ass={}[subtitled];\n",
                subtitles.options
            ));
            current = "subtitled".to_string();
        }
        let mut image_inputs = Vec::new();
        if let Some(danmaku) = &self.danmaku {
            // Only the danmaku on screen during the segment, for it to read.
            let danmu_name = format!("danmu{index}.ass");
            tokio::fs::write(
                self.directory.join(&danmu_name),
                danmaku.document.excerpt(start, end),
            )
            .await
            .map_err(|error| error.to_string())?;
            filter.push_str(&format!("[{current}]ass={danmu_name}[danmu0];\n"));
            let (chain, slots) = danmu::image_overlay_chain(
                &segment_danmaku(&danmaku.images, start, end),
                job.danmu_options.opacity,
                segment.parts.len() + overlays.len(),
                "danmu",
            );
            filter.push_str(&chain.filter);
            image_inputs = chain.inputs;
            current = format!("danmu{slots}");
        }
        let format = if hwaccel::is_vaapi_encoder(self.encoder) {
            hwaccel::vaapi_filter_suffix()
        } else {
            "format=yuv420p"
        };
        // Exactly as many frames as planned: `overlay` goes on as long as any
        // of its inputs does, and a file ending well before its clip has no
        // frames for the parts after its end. Then back to the segment's own
        // time.
        filter.push_str(&format!(
            "[{current}]tpad=stop_mode=clone:stop_duration={:.6},trim=end_frame={},\
             setpts=PTS-STARTPTS,{format}[outv]\n",
            end - start,
            segment.frames,
        ));
        let filter_name = format!("segment{index}.txt");
        tokio::fs::write(self.directory.join(&filter_name), &filter)
            .await
            .map_err(|error| error.to_string())?;

        let mut command = ffmpeg_command();
        command.current_dir(self.directory).args(["-nostdin", "-y"]);
        for part in &segment.parts {
            let clip = &job.clips[part.clip];
            let (seek, preroll) = part_input(clip, part.skip);
            let length = (part.frames as f64 / fps).max(part.samples as f64 / SAMPLE_RATE);
            let input = std::path::absolute(&clip.path).map_err(|error| error.to_string())?;
            command
                .args(["-ss", &format!("{seek:.6}")])
                .args(["-t", &format!("{:.6}", preroll + length)])
                .arg("-i")
                .arg(input);
        }
        for shown in &overlays {
            match &shown.overlay.layer {
                OverlayLayer::Image { .. } => {
                    command
                        .args(["-threads", "1", "-i"])
                        .arg(overlay_image_name(shown.index));
                }
                OverlayLayer::Video(layer) => {
                    let (seek, skip) = shown.seek(layer);
                    let input =
                        std::path::absolute(&layer.path).map_err(|error| error.to_string())?;
                    command
                        .args(["-ss", &format!("{seek:.6}")])
                        .args(["-t", &format!("{:.6}", skip + shown.end - shown.start)])
                        .arg("-i")
                        .arg(input);
                }
            }
        }
        for image in image_inputs {
            // A single PNG frame is repeated by overlay framesync.
            command.args(["-threads", "1", "-i"]).arg(image);
        }
        command
            .arg(danmu::filter_complex_file_option().await)
            .arg(&filter_name)
            .args(["-map", "[outv]"]);
        hwaccel::apply_x264_encoder_only(&mut command, self.encoder);
        hwaccel::apply_x264_quality_args(&mut command, self.encoder);
        // Written under other names and renamed once complete, so that a
        // segment's files are there only when they are whole. One left
        // without the other, by a render stopped between the two renames,
        // goes first.
        let (video, sound) = (segment_video_name(key), segment_sound_name(key));
        for name in [&video, &sound] {
            let _ = tokio::fs::remove_file(self.directory.join(name)).await;
        }
        let (partial_video, partial_sound) = (format!("{key}.part.mp4"), format!("{key}.part.wav"));
        let guards = [
            TempArtifact::new(self.directory.join(&partial_video)),
            TempArtifact::new(self.directory.join(&partial_sound)),
        ];
        command
            .args(["-r", &fps.to_string()])
            .arg(&partial_video)
            .args(["-map", "[outa]", "-c:a", "pcm_s16le"])
            .arg(&partial_sound);
        log::info!("Render project segment {index}: {command:?}");
        log::debug!("Project segment {index} filter graph:\n{filter}");
        run_command(
            command,
            ProgressMode::stage(RENDER_STAGE, ProgressTotal::Known(end - start)),
            "Render project segment",
            Some(reporter),
        )
        .await
        .map_err(|error| error.to_string())?;
        for (from, to) in [(partial_sound, sound), (partial_video, video)] {
            tokio::fs::rename(self.directory.join(from), self.directory.join(to))
                .await
                .map_err(|error| error.to_string())?;
        }
        for guard in guards {
            guard.keep();
        }
        Ok(())
    }
}

/// Join the rendered segments, with the files named by `keys`, into
/// `output`, encoding their sound as one.
async fn join_segments<R: ProgressReporterTrait>(
    reporter: &R,
    directory: &Path,
    segments: &[Segment],
    keys: &[String],
    fps: f64,
    output: &Path,
) -> Result<(), String> {
    // Durations in the microseconds the demuxer reads, from the frames'
    // times, so that they add up to them.
    let micros = |frame: u64| (frame as f64 / fps * 1e6).round() as u64;
    let mut videos = String::new();
    let mut sounds = String::new();
    for (segment, key) in segments.iter().zip(keys) {
        let duration = micros(segment.first_frame + segment.frames) - micros(segment.first_frame);
        videos.push_str(&format!(
            "file '{}'\nduration {}.{:06}\n",
            segment_video_name(key),
            duration / 1_000_000,
            duration % 1_000_000,
        ));
        sounds.push_str(&format!("file '{}'\n", segment_sound_name(key)));
    }
    for (name, list) in [("videos.txt", videos), ("sounds.txt", sounds)] {
        tokio::fs::write(directory.join(name), list)
            .await
            .map_err(|error| error.to_string())?;
    }
    let total = segments.iter().map(|segment| segment.frames).sum::<u64>() as f64 / fps;
    let mut command = ffmpeg_command();
    command
        .current_dir(directory)
        .args(["-nostdin", "-y"])
        .args([
            "-f",
            "concat",
            "-i",
            "videos.txt",
            "-f",
            "concat",
            "-i",
            "sounds.txt",
        ])
        .args(["-map", "0:v", "-map", "1:a", "-c:v", "copy"])
        .args(["-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"])
        .arg(output);
    log::info!("Join project segments: {command:?}");
    run_command(
        command,
        ProgressMode::stage("合并片段", ProgressTotal::Known(total)),
        "Join project segments",
        Some(reporter),
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn render_project<R: ProgressReporterTrait>(
    reporter: &R,
    job: ProjectRender<'_>,
) -> Result<(), String> {
    render_in_segments(reporter, &job, SEGMENT_LIMITS).await?;
    Ok(())
}

/// How many segments a render had, and how many of them it found rendered.
#[derive(Debug, Clone, Copy, PartialEq)]
struct RenderStats {
    segments: usize,
    reused: usize,
}

async fn render_in_segments<R: ProgressReporterTrait>(
    reporter: &R,
    job: &ProjectRender<'_>,
    limits: SegmentLimits,
) -> Result<RenderStats, String> {
    let encoder = hwaccel::get_x264_encoder().await;
    render_with_encoder(reporter, job, limits, encoder).await
}

async fn render_with_encoder<R: ProgressReporterTrait>(
    reporter: &R,
    job: &ProjectRender<'_>,
    limits: SegmentLimits,
    encoder: &'static str,
) -> Result<RenderStats, String> {
    if job.clips.is_empty() {
        return Err("时间轴上没有片段".to_string());
    }
    let output = std::path::absolute(job.output).map_err(|error| error.to_string())?;
    // Generated files use fixed names in a directory that is FFmpeg's working
    // directory, so no user path is interpolated into the graphs: the
    // project's segment cache, which keeps segments for the next render, or
    // else a temporary folder beside the output, on a disk with room for it.
    let (directory, _temporary) = match job.segment_cache {
        Some(cache) => {
            let cache = std::path::absolute(cache).map_err(|error| error.to_string())?;
            tokio::fs::create_dir_all(&cache)
                .await
                .map_err(|error| error.to_string())?;
            (cache, None)
        }
        None => {
            let temporary = tempfile::Builder::new()
                .prefix(".bili-project-")
                .tempdir_in(output.parent().ok_or("导出路径无效")?)
                .map_err(|error| error.to_string())?;
            (temporary.path().to_path_buf(), Some(temporary))
        }
    };
    let _scratch = Scratch(&directory);
    let burn_danmu = !job.danmus.is_empty();
    let fps = output_fps(&job.clips, burn_danmu);
    let clips: Vec<PlanClip> = job
        .clips
        .iter()
        .map(|clip| PlanClip {
            id: &clip.id,
            duration: clip.duration,
        })
        .collect();
    let segments = plan_segments(&clips, fps, limits);

    let mut pictures = Vec::with_capacity(job.overlays.len());
    let mut files = HashMap::new();
    for clip in &job.clips {
        if !files.contains_key(&clip.path) {
            files.insert(clip.path.clone(), file_identity(&clip.path).await);
        }
    }
    for (index, overlay) in job.overlays.iter().enumerate() {
        match &overlay.layer {
            OverlayLayer::Image { png, .. } => {
                tokio::fs::write(directory.join(overlay_image_name(index)), png)
                    .await
                    .map_err(|error| error.to_string())?;
                pictures.push(Some(digest(png)));
            }
            OverlayLayer::Video(layer) => {
                if !files.contains_key(&layer.path) {
                    files.insert(layer.path.clone(), file_identity(&layer.path).await);
                }
                pictures.push(None);
            }
        }
    }
    let subtitles = match &job.subtitle_ass {
        Some(ass) => {
            tokio::fs::write(directory.join("subtitle.ass"), ass)
                .await
                .map_err(|error| error.to_string())?;
            let mut options = "filename=subtitle.ass".to_string();
            if let Some(fonts) = job.fonts_directory {
                options.push_str(&format!(
                    ":fontsdir={}",
                    super::subtitle::subtitle_filter_path(fonts)?
                ));
            }
            Some(Subtitles {
                options,
                document: AssDocument::parse(ass),
            })
        }
        None => None,
    };
    let danmaku = if burn_danmu {
        let images = danmu::prepare_danmu_layers(
            Some(reporter),
            &job.danmus,
            &job.danmu_options,
            &directory,
        )
        .await?;
        let document = tokio::fs::read_to_string(directory.join("danmu.ass"))
            .await
            .map_err(|error| error.to_string())?;
        let mut pictures = HashMap::new();
        for image in &images {
            if !pictures.contains_key(&image.image.path) {
                let bytes = tokio::fs::read(&image.image.path)
                    .await
                    .map_err(|error| error.to_string())?;
                pictures.insert(image.image.path.clone(), digest(&bytes));
            }
        }
        Some(Danmaku {
            document: AssDocument::parse(&document),
            images,
            pictures,
        })
    } else {
        None
    };
    let render = Render {
        job,
        directory: &directory,
        fps,
        encoder,
        files,
        pictures,
        subtitles,
        danmaku,
    };

    let keys: Vec<String> = segments
        .iter()
        .map(|segment| {
            let overlays = segment_overlays(&job.overlays, segment.start(fps), segment.end(fps));
            render.key(segment, &overlays)
        })
        .collect();
    let total = segments.iter().map(|segment| segment.frames).sum::<u64>() as f64 / fps;
    let progress = SegmentProgress::new(reporter, segments.len(), total);
    // Each segment not rendered yet, once however often it is shown.
    let mut pending = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (index, (segment, key)) in segments.iter().zip(&keys).enumerate() {
        let rendered = tokio::fs::try_exists(directory.join(segment_video_name(key)))
            .await
            .unwrap_or(false)
            && tokio::fs::try_exists(directory.join(segment_sound_name(key)))
                .await
                .unwrap_or(false);
        if rendered || !seen.insert(key) {
            progress
                .segment(index, segment.frames as f64 / fps)
                .finished()
                .await;
        } else {
            pending.push((index, segment, key));
        }
    }
    let stats = RenderStats {
        segments: segments.len(),
        reused: segments.len() - pending.len(),
    };
    log::info!(
        "Render project in {} segments, {} of them rendered before",
        stats.segments,
        stats.reused,
    );
    if job.segment_cache.is_some() {
        // Segments of other renders go before this one writes its own, so
        // that the cache never needs room for both.
        remove_other_segments(&directory, &keys).await;
    }
    stream::iter(pending.into_iter().map(Ok::<_, String>))
        .try_for_each_concurrent(PARALLEL_SEGMENTS, |(index, segment, key)| {
            let (render, progress) = (
                &render,
                progress.segment(index, segment.frames as f64 / fps),
            );
            async move {
                render.segment(index, segment, key, &progress).await?;
                progress.finished().await;
                Ok(())
            }
        })
        .await?;

    let output_guard = TempArtifact::new(&output);
    join_segments(reporter, &directory, &segments, &keys, fps, &output).await?;
    output_guard.keep();
    if job.segment_cache.is_some() {
        // The cache keeps what this render is made of: its segments, and the
        // danmaku images it would need to render them again.
        let used: std::collections::HashSet<&str> = keys.iter().map(String::as_str).collect();
        let images: std::collections::HashSet<&std::ffi::OsStr> = render
            .danmaku
            .iter()
            .flat_map(|danmaku| &danmaku.images)
            .filter_map(|image| image.image.path.file_name())
            .collect();
        if let Ok(mut entries) = tokio::fs::read_dir(&directory).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name();
                let key = name.to_str().and_then(|name| name.split('.').next());
                let kept = if is_segment_file(&name) {
                    key.is_some_and(|key| used.contains(key))
                } else if is_image_file(&name) {
                    images.contains(name.as_os_str())
                } else {
                    continue;
                };
                if kept {
                    // Used now, though maybe rendered long ago: the cache
                    // limit goes by when segments were last used.
                    mark_used(&entry.path());
                } else {
                    let _ = tokio::fs::remove_file(entry.path()).await;
                }
            }
        }
    }
    Ok(stats)
}

/// Remove the segments in `directory` other than those of `keys`.
async fn remove_other_segments(directory: &Path, keys: &[String]) {
    let used: std::collections::HashSet<&str> = keys.iter().map(String::as_str).collect();
    let Ok(mut entries) = tokio::fs::read_dir(directory).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let key = name.to_str().and_then(|name| name.split('.').next());
        if is_segment_file(&name) && !key.is_some_and(|key| used.contains(key)) {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

/// Set the time `path` last changed to now.
fn mark_used(path: &Path) {
    let marked = std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(std::time::SystemTime::now()));
    if let Err(error) = marked {
        log::warn!("Cannot mark segment {} used: {error}", path.display());
    }
}

/// Remove the segment caches in `root` other than `current`, the least
/// recently used first, until all of them take at most `limit` bytes.
pub async fn limit_segment_caches(root: &Path, current: &Path, limit: u64) {
    let Ok(mut projects) = tokio::fs::read_dir(root).await else {
        return;
    };
    let mut caches = Vec::new();
    let mut total = 0;
    while let Ok(Some(project)) = projects.next_entry().await {
        let Ok(mut files) = tokio::fs::read_dir(project.path()).await else {
            continue;
        };
        let (mut size, mut latest) = (0, UNIX_EPOCH);
        while let Ok(Some(file)) = files.next_entry().await {
            if let Ok(metadata) = file.metadata().await {
                size += metadata.len();
                latest = latest.max(metadata.modified().unwrap_or(UNIX_EPOCH));
            }
        }
        total += size;
        if project.path() != current {
            caches.push((latest, size, project.path()));
        }
    }
    caches.sort();
    for (_, size, path) in caches {
        if total <= limit {
            break;
        }
        if tokio::fs::remove_dir_all(&path).await.is_ok() {
            total -= size;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::project_plan::{anchor_id, plain_ids};
    use super::*;

    fn clip(has_audio: bool, fps: f64) -> RenderClip {
        RenderClip {
            id: String::new(),
            path: PathBuf::from("a.mp4"),
            start: 1.0,
            duration: 2.5,
            has_audio,
            fps,
            file_duration: 0.0,
        }
    }

    fn one_segment(durations: &[f64], fps: f64) -> Segment {
        let ids = plain_ids(durations.len());
        let clips: Vec<_> = ids
            .iter()
            .zip(durations)
            .map(|(id, duration)| PlanClip {
                id,
                duration: *duration,
            })
            .collect();
        let mut segments = plan_segments(&clips, fps, SEGMENT_LIMITS);
        assert_eq!(segments.len(), 1);
        segments.remove(0)
    }

    #[test]
    fn clips_are_normalized_and_joined_in_order() {
        let segment = one_segment(&[2.5, 2.5], 60.0);
        let filter = concat_filter(
            &[clip(true, 30.0), clip(false, 60.0)],
            &segment,
            60.0,
            "outa",
        );
        // Read from a second before the in point, which is dropped. Both
        // streams of a clip last exactly its timeline duration.
        assert!(filter.contains(
            "[0:v]setpts=PTS+0.000000/TB,fps=60:start_time=0,\
             tpad=stop_mode=clone:stop_duration=3.500000,\
             trim=start_frame=60:end_frame=210,setpts=PTS-STARTPTS,scale=1920:1080"
        ));
        assert!(filter.contains("setsar=1,format=yuv420p[pv0]"));
        assert!(
            filter.contains("[0:a]aresample=48000:async=1:first_pts=0,atrim=start_sample=48000,")
        );
        assert!(filter.contains("apad,atrim=end_sample=120000[pa0]"));
        // A clip without audio gets silence of the same length.
        assert!(filter.contains("anullsrc=r=48000:cl=stereo,atrim=end_sample=120000"));
        assert!(!filter.contains("[1:a]"));
        assert!(filter.contains("[pv0][pv1]concat=n=2:v=1:a=0,setpts=(N+0)/60/TB[joined]"));
        assert!(filter.contains("[pa0][pa1]concat=n=2:v=0:a=1[outa]"));
    }

    #[test]
    fn a_part_cut_within_a_clip_goes_on_in_project_time() {
        let mut long = clip(true, 30.0);
        long.duration = 100.0;
        let clips = [PlanClip {
            id: "long",
            duration: 100.0,
        }];
        let segments = plan_segments(&clips, 30.0, SEGMENT_LIMITS);
        assert_eq!(segments.len(), 2);
        let second = &segments[1];
        assert_eq!(second.parts[0].skip, 50.0);
        // 51 s into the file, read from a second before.
        assert_eq!(part_input(&long, second.parts[0].skip), (50.0, 1.0));
        let filter = concat_filter(&[long], second, 30.0, "outa");
        assert!(filter.contains("[0:v]setpts=PTS+0.000000/TB,fps=30:start_time=0,"));
        assert!(filter.contains("trim=start_frame=30:end_frame=1530,"));
        assert!(filter.contains("concat=n=1:v=1:a=0,setpts=(N+1500)/30/TB[joined]"));
        // Near the start of a file, only what there is before the in point.
        let mut early = clip(true, 30.0);
        early.start = 0.4;
        assert_eq!(part_input(&early, 0.0), (0.0, 0.4));
        // Past the end of a file of 3 s, from its last two seconds.
        early.file_duration = 3.0;
        assert_eq!(part_input(&early, 4.6), (1.0, 4.0));
        let filter = concat_filter(
            &[early],
            &Segment {
                first_frame: 150,
                cut: 5.0,
                frames: 30,
                samples: 48000,
                parts: vec![crate::ffmpeg::project_plan::Part {
                    clip: 0,
                    skip: 4.6,
                    frames: 30,
                    samples: 48000,
                }],
            },
            30.0,
            "outa",
        );
        // Its last frame is padded on to the part.
        assert!(filter.contains(
            "[0:v]setpts=PTS+0.000000/TB,fps=30:start_time=0,\
             tpad=stop_mode=clone:stop_duration=5.000000,\
             trim=start_frame=120:end_frame=150,"
        ));
    }

    fn video_layer(has_audio: bool, rotation: f64) -> OverlayLayer {
        OverlayLayer::Video(VideoLayer {
            path: PathBuf::from("pip.mp4"),
            source_start: 3.0,
            duration: 0.0,
            has_audio,
            x: 1500.0,
            y: 800.0,
            width: 641.0,
            rotation,
            volume: 0.5,
        })
    }

    #[test]
    fn overlays_are_laid_over_the_joined_clips_in_order() {
        let overlays = [
            RenderOverlay {
                start: 1.0,
                end: 4.0,
                layer: video_layer(true, 0.0),
            },
            RenderOverlay {
                start: 2.0,
                end: 3.5,
                layer: OverlayLayer::Image {
                    png: Vec::new(),
                    x: -10.4,
                    y: 20.6,
                },
            },
            RenderOverlay {
                start: 0.0,
                end: 9.0,
                layer: video_layer(false, 90.0),
            },
        ];
        let shown = segment_overlays(&overlays, 0.0, 10.0);
        let graph = overlay_filter(&shown, 2, 30.0, (0.0, 480_000), "joined");
        let filter = &graph.filter;
        // The video is read from a second before its in point, scaled to an
        // even width and moved to its place in time.
        assert!(filter.contains(
            "[2:v]fps=30:start_time=0,tpad=stop_mode=clone:stop_duration=4.000000,\
             trim=start=1.000000:duration=3.000000,setpts=PTS-STARTPTS,scale=642:-2,setsar=1,format=yuva420p,\
             setpts=PTS+1.000000/TB[ov0]"
        ));
        assert!(filter.contains(
            "[joined][ov0]overlay=x=1500.000-overlay_w/2:y=800.000-overlay_h/2:eof_action=repeat:\
             enable='gte(t,1.000000)*lt(t,4.000000)'[layer0]"
        ));
        // Its sound starts with it and lasts the whole segment.
        assert!(
            filter.contains("[2:a]aresample=48000:async=1:first_pts=0,atrim=start_sample=48000,")
        );
        assert!(filter.contains(
            "atrim=duration=3.000000,volume=0.500,adelay=1000|1000,apad,atrim=end_sample=480000[oa0]"
        ));
        // Pictures are placed by their top left, each over the layer before.
        assert!(filter.contains("[3:v]format=rgba[ov1]"));
        assert!(filter.contains(
            "[layer0][ov1]overlay=x=-10:y=21:enable='gte(t,2.000000)*lt(t,3.500000)'[layer1]"
        ));
        assert!(filter.contains("rotate=1.570796:c=black@0:ow=rotw(1.570796):oh=roth(1.570796)"));
        assert!(!filter.contains("[4:a]"));
        assert_eq!(
            (graph.video.as_str(), graph.audio.as_slice()),
            ("layer2", &["oa0".to_string()][..])
        );
        assert_eq!(
            mix_filter("maina", &graph.audio),
            "[maina][oa0]amix=inputs=2:duration=first:dropout_transition=0,volume=2[outa];\n"
        );
        // Without overlays the joined clips are used as they are.
        let none = overlay_filter(&[], 2, 30.0, (0.0, 480_000), "joined");
        assert_eq!((none.filter.as_str(), none.video.as_str()), ("", "joined"));
    }

    #[test]
    fn overlays_go_on_in_the_segments_after_the_one_they_start_in() {
        let mut held = video_layer(true, 0.0);
        if let OverlayLayer::Video(layer) = &mut held {
            layer.duration = 5.0;
        }
        let overlays = [
            RenderOverlay {
                start: 0.0,
                end: 2.5,
                layer: video_layer(true, 0.0),
            },
            RenderOverlay {
                start: 1.0,
                end: 4.0,
                layer: video_layer(true, 0.0),
            },
            RenderOverlay {
                start: 1.0,
                end: 9.0,
                layer: held,
            },
        ];
        // The first ends where the segment starts.
        let shown = segment_overlays(&overlays, 2.5, 10.0);
        let spans: Vec<_> = shown
            .iter()
            .map(|item| (item.index, item.start, item.end))
            .collect();
        assert_eq!(spans, [(1, 2.5, 4.0), (2, 2.5, 9.0)]);
        let OverlayLayer::Video(layer) = &overlays[1].layer else {
            unreachable!()
        };
        // 1.5 s past its in point at 3 s, read from a second before.
        assert_eq!(shown[0].seek(layer), (3.5, 1.0));
        let graph = overlay_filter(&shown, 1, 30.0, (2.5, 360_000), "joined");
        assert!(graph.filter.contains(
            "[1:v]fps=30:start_time=0,tpad=stop_mode=clone:stop_duration=2.500000,trim=start=1.000000:duration=1.500000,"
        ));
        assert!(graph.filter.contains("setpts=PTS+2.500000/TB[ov0]"));
        assert!(graph
            .filter
            .contains("enable='gte(t,2.500000)*lt(t,4.000000)'[layer0]"));
        // Its sound goes on at the start of the segment.
        assert!(graph.filter.contains(
            "atrim=duration=1.500000,volume=0.500,adelay=0|0,apad,atrim=end_sample=360000[oa0]"
        ));
        // 4.5 s into a file of 5 s: read from its last two seconds, so that
        // its last frame is there to hold.
        let OverlayLayer::Video(layer) = &overlays[2].layer else {
            unreachable!()
        };
        let (seek, skip) = shown[1].seek(layer);
        assert_eq!(seek, 3.0);
        assert!((skip - 1.5).abs() < 1e-9);
        assert!(graph.filter.contains(
            "tpad=stop_mode=clone:stop_duration=8.000000,trim=start=1.500000:duration=6.500000,"
        ));
    }

    /// Keeps the progress it is given.
    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<StageProgress>>>);

    #[async_trait::async_trait]
    impl ProgressReporterTrait for Recorder {
        async fn update(&self, _content: &str) {}
        async fn progress(&self, progress: StageProgress) {
            self.0.lock().unwrap().push(progress);
        }
        async fn finish(&self, _success: bool, _message: &str) {}
    }

    #[tokio::test]
    async fn segments_rendering_at_once_report_one_progress() {
        let recorder = Recorder::default();
        let progress = SegmentProgress::new(&recorder, 2, 10.0);
        let (first, second) = (progress.segment(0, 4.0), progress.segment(1, 6.0));
        let at = |processed: f64, speed: f64| StageProgress {
            stage: "导出切片".to_string(),
            processed,
            total: Some(6.0),
            speed: Some(speed),
        };
        first.progress(at(2.0, 1.5)).await;
        second.progress(at(3.0, 2.0)).await;
        first.finished().await;
        // Past its end, as FFmpeg can report, it counts as done.
        second.progress(at(7.0, 2.5)).await;
        let reported: Vec<_> = recorder
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|progress| (progress.processed, progress.total, progress.speed))
            .collect();
        assert_eq!(
            reported,
            [
                (2.0, Some(10.0), Some(1.5)),
                (5.0, Some(10.0), Some(3.5)),
                // The first reports itself done, and no longer adds its speed.
                (7.0, Some(10.0), Some(2.0)),
                (10.0, Some(10.0), Some(2.5)),
            ]
        );
    }

    #[test]
    fn segments_take_the_image_danmaku_on_screen_during_them() {
        let image = |start: f64| ImageDanmu {
            image: crate::danmu2ass::DanmuImage {
                path: PathBuf::from("emoji.png"),
                width: 40.0,
                height: 40.0,
            },
            start,
            top: 0.0,
        };
        let images = [image(0.0), image(5.0), image(20.0)];
        let starts = |from: f64, to: f64| -> Vec<f64> {
            segment_danmaku(&images, from, to)
                .iter()
                .map(|image| image.start)
                .collect()
        };
        // Scrolling for R2L_TIME seconds from its start.
        assert_eq!(starts(10.0, 20.0), [5.0]);
        assert_eq!(starts(19.0, 21.0), [20.0]);
        assert_eq!(starts(0.0, 30.0), [0.0, 5.0, 20.0]);
    }

    const ASS: &str = "[Script Info]\nPlayResX: 1920\n\n[V4+ Styles]\nFormat: Name, Fontname\n\
        Style: Default,Arial\n\n[Events]\nFormat: Layer, Start, End, Style, Text\n\
        Dialogue: 0,0:00:01.00,0:00:02.50,Default,one, two\n\
        Comment: 0,0:00:00.00,0:00:09.00,Default,note\n\
        Dialogue: 0,0:01:00.00,0:01:01.00,Default,later\n";

    #[test]
    fn ass_events_are_read_with_their_times() {
        let document = AssDocument::parse(ASS);
        assert_eq!(document.events.len(), 2);
        assert_eq!(
            (document.events[0].start, document.events[0].end),
            (1.0, 2.5)
        );
        assert_eq!(document.events[0].content, "0,Default,one, two");
        assert_eq!(document.events[1].start, 60.0);
        // An excerpt keeps everything but the events not shown.
        let excerpt = document.excerpt(0.0, 10.0);
        assert!(excerpt.contains("Style: Default,Arial"));
        assert!(excerpt.contains("Format: Layer, Start, End, Style, Text"));
        assert!(excerpt.contains("Comment:"));
        assert!(excerpt.contains("one, two") && !excerpt.contains("later"));
        // Described by the frames it is on, from the start, and what it shows.
        let mut key = String::new();
        document.describe(&mut key, 0.5, 30.0, 60);
        assert!(key.ends_with("\n15 60 0,Default,one, two\n"), "{key}");
        // A segment with none of the events shown has nothing of them, not
        // even their styles, in its key.
        let mut none = String::new();
        document.describe(&mut none, 20.0, 30.0, 30);
        assert!(none.is_empty(), "{none}");
    }

    fn timed(id: &str, duration: f64) -> RenderClip {
        RenderClip {
            id: id.to_string(),
            path: PathBuf::from("a.mp4"),
            start: 0.0,
            duration,
            has_audio: true,
            fps: 30.0,
            file_duration: 0.0,
        }
    }

    /// A job of `clips`, with a picture from `picture` to a second later at
    /// `x`, without rendering it.
    fn job(clips: Vec<RenderClip>, picture: f64, x: f64) -> ProjectRender<'static> {
        ProjectRender {
            clips,
            overlays: vec![RenderOverlay {
                start: picture,
                end: picture + 1.0,
                layer: OverlayLayer::Image {
                    png: Vec::new(),
                    x,
                    y: 0.0,
                },
            }],
            subtitle_ass: None,
            fonts_directory: None,
            danmus: Vec::new(),
            danmu_options: Danmu2AssOptions::default(),
            output: Path::new("out.mp4"),
            segment_cache: None,
        }
    }

    fn keys(job: &ProjectRender, ass: &str) -> Vec<String> {
        keys_with_danmaku(job, ass, &[])
    }

    /// The segments' keys, with `danmus` in project time burned.
    fn keys_with_danmaku(job: &ProjectRender, ass: &str, danmus: &[DanmuEntry]) -> Vec<String> {
        let fps = output_fps(&job.clips, !danmus.is_empty());
        let danmaku = (!danmus.is_empty()).then(|| {
            let options = Danmu2AssOptions::default();
            let plan = crate::danmu2ass::danmu_render_plan(danmus, &options, |_| None);
            Danmaku {
                document: AssDocument::parse(&plan.ass),
                images: plan.images,
                pictures: HashMap::new(),
            }
        });
        let render = Render {
            job,
            directory: Path::new("."),
            fps,
            encoder: "libx264",
            files: HashMap::new(),
            pictures: vec![Some("picture".to_string()); job.overlays.len()],
            subtitles: Some(Subtitles {
                options: String::new(),
                document: AssDocument::parse(ass),
            }),
            danmaku,
        };
        let clips: Vec<_> = job
            .clips
            .iter()
            .map(|clip| PlanClip {
                id: &clip.id,
                duration: clip.duration,
            })
            .collect();
        plan_segments(&clips, fps, SEGMENT_LIMITS)
            .iter()
            .map(|segment| {
                let overlays =
                    segment_overlays(&job.overlays, segment.start(fps), segment.end(fps));
                render.key(segment, &overlays)
            })
            .collect()
    }

    /// Subtitles at 1 s and 6 s, `shift` later.
    fn subtitles(shift: f64, text: &str) -> String {
        let clock = |seconds: f64| {
            let centis = (seconds * 100.0).round() as u64;
            format!(
                "0:{:02}:{:02}.{:02}",
                centis / 6000,
                centis / 100 % 60,
                centis % 100
            )
        };
        format!(
            "[Script Info]\n\n[Events]\nFormat: Layer, Start, End, Style, Text\n\
             Dialogue: 0,{},{},Default,first\nDialogue: 0,{},{},Default,{text}\n",
            clock(1.0 + shift),
            clock(2.0 + shift),
            clock(6.0 + shift),
            clock(7.0 + shift),
        )
    }

    #[test]
    fn segments_moved_by_whole_frames_keep_their_keys() {
        let ids = [
            anchor_id(0),
            plain_ids(1)[0].clone(),
            anchor_id(100),
            plain_ids(2)[1].clone(),
        ];
        let clips = |before: Option<f64>| {
            let mut clips: Vec<_> = ids
                .iter()
                .zip([2.0, 3.0, 2.5, 1.5])
                .map(|(id, duration)| timed(id, duration))
                .collect();
            if let Some(duration) = before {
                clips.insert(0, timed(&plain_ids(3)[2], duration));
            }
            clips
        };
        // Two segments: the clips from 0 s and from 5 s.
        let first = keys(&job(clips(None), 5.5, 10.0), &subtitles(0.0, "third"));
        assert_eq!(first.len(), 2);
        // A clip of 30 frames before them: they are the same, a second later.
        let moved = keys(&job(clips(Some(1.0)), 6.5, 10.0), &subtitles(1.0, "third"));
        assert_eq!(&moved[1..], &first[..]);
        // Moved by part of a frame, they are not.
        let nudged = keys(
            &job(clips(Some(1.0137)), 6.5137, 10.0),
            &subtitles(1.0137, "third"),
        );
        assert_ne!(&nudged[1..], &first[..]);
        // What a segment shows is in its key, and only in its own.
        let retitled = keys(&job(clips(None), 5.5, 10.0), &subtitles(0.0, "changed"));
        assert_eq!(
            (retitled[0] == first[0], retitled[1] == first[1]),
            (true, false)
        );
        let moved_picture = keys(&job(clips(None), 5.5, 20.0), &subtitles(0.0, "third"));
        assert_eq!(
            (moved_picture[0] == first[0], moved_picture[1] == first[1]),
            (true, false)
        );
    }

    #[test]
    fn subtitles_and_danmaku_moved_by_whole_frames_keep_their_keys() {
        // Burning danmaku, at 60 FPS.
        let fps = 60.0;
        let ids = [anchor_id(0), plain_ids(1)[0].clone(), anchor_id(100)];
        let lead = plain_ids(3)[2].clone();
        // Material times of the subtitles, on the second clip, and the danmaku.
        let cues = [(0.4567, 1.2345), (2.0123, 2.7777)];
        let danmu_times = [123, 777, 1_450, 2_901];
        let render = |frames: u32, between_frames: bool| {
            let clips: Vec<_> = std::iter::once((lead.as_str(), 1.0 + f64::from(frames) / fps))
                .chain(ids.iter().map(String::as_str).zip([2.0, 3.0, 2.5]))
                .collect();
            let timeline = crate::project::Timeline {
                clips: clips
                    .iter()
                    .enumerate()
                    .map(|(index, (id, duration))| crate::project::TimelineClip {
                        id: id.to_string(),
                        material_id: index as i64,
                        start: 0.0,
                        end: *duration,
                    })
                    .collect(),
                ..crate::project::Timeline::default()
            };
            let material = |ts: &[i64]| -> Vec<DanmuEntry> {
                ts.iter()
                    .map(|ts| DanmuEntry {
                        ts: *ts,
                        event_type: "danmu".into(),
                        content: format!("弹幕{ts}"),
                        emoji_content: None,
                        user_name: None,
                        price: None,
                        sc_duration: None,
                    })
                    .collect()
            };
            let on_second = material(&danmu_times);
            let danmus = timeline.map_danmus(fps, |id| (id == 2).then_some(&on_second[..]));
            // As the editor writes them, between frames, or as they are.
            let offset = clips[0].1 + clips[1].1;
            let at = |time: f64| {
                let time = offset + time;
                let time = if between_frames {
                    ((time * fps - EPSILON).ceil() - 0.5) / fps
                } else {
                    time
                };
                let centis = (time * 100.0).round() as u64;
                format!(
                    "0:{:02}:{:02}.{:02}",
                    centis / 6000,
                    centis / 100 % 60,
                    centis % 100
                )
            };
            let mut ass =
                "[Script Info]\n\n[Events]\nFormat: Layer, Start, End, Style, Text\n".to_string();
            for (start, end) in cues {
                ass.push_str(&format!(
                    "Dialogue: 0,{},{},Default,cue\n",
                    at(start),
                    at(end)
                ));
            }
            let job = ProjectRender {
                clips: clips
                    .iter()
                    .map(|(id, duration)| timed(id, *duration))
                    .collect(),
                overlays: Vec::new(),
                subtitle_ass: None,
                fonts_directory: None,
                danmus: Vec::new(),
                danmu_options: Danmu2AssOptions::default(),
                output: Path::new("out.mp4"),
                segment_cache: None,
            };
            // The lead clip's segment changes; those after it are compared.
            keys_with_danmaku(&job, &ass, &danmus)[1..].to_vec()
        };
        // The lead clip's segment, one with the subtitles and danmaku, and one
        // after.
        let before = render(0, true);
        assert_eq!(before.len(), 2);
        for frames in 1..=12 {
            assert_eq!(render(frames, true), before, "moved by {frames} frames");
        }
        // Subtitles at their own times change their segment's key for some
        // of these moves, as they fall on other frames once rounded.
        let loose = render(0, false);
        assert!((1..=12).any(|frames| render(frames, false)[0] != loose[0]));
    }

    #[tokio::test]
    async fn only_the_segments_of_a_render_stay_before_it_writes_its_own() {
        let directory = tempfile::tempdir().unwrap();
        let (kept, other) = ("a".repeat(32), "b".repeat(32));
        for name in [
            format!("{kept}.mp4"),
            format!("{kept}.wav"),
            format!("{other}.mp4"),
            format!("{other}.wav"),
            format!("image-{}.png", "c".repeat(32)),
            "segment0.txt".to_string(),
        ] {
            std::fs::write(directory.path().join(name), b"x").unwrap();
        }
        remove_other_segments(directory.path(), std::slice::from_ref(&kept)).await;
        let mut left: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        // Danmaku images and the render's own files are left to the cleanup
        // after it.
        assert_eq!(
            left,
            [
                format!("{kept}.mp4"),
                format!("{kept}.wav"),
                format!("image-{}.png", "c".repeat(32)),
                "segment0.txt".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn segment_caches_beyond_the_limit_go_least_recently_used_first() {
        let root = tempfile::tempdir().unwrap();
        let now = std::time::SystemTime::now();
        for (name, age) in [("old", 300), ("newer", 100), ("current", 400)] {
            let directory = root.path().join(name);
            std::fs::create_dir(&directory).unwrap();
            let file = directory.join("segment.mp4");
            std::fs::write(&file, vec![0; 300]).unwrap();
            std::fs::File::options()
                .write(true)
                .open(&file)
                .unwrap()
                .set_modified(now - std::time::Duration::from_secs(age))
                .unwrap();
        }
        let current = root.path().join("current");
        let left = || {
            let mut names: Vec<_> = std::fs::read_dir(root.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        };
        // Rendered long ago, used now: it is the most recently used.
        let reused = root.path().join("old").join("segment.mp4");
        mark_used(&reused);
        let age = std::fs::metadata(&reused).unwrap().modified().unwrap();
        assert!(now.duration_since(age).unwrap_or_default().as_secs() < 5);
        limit_segment_caches(root.path(), &current, 700).await;
        assert_eq!(left(), ["current", "old"]);
        // The current project's stays, even over the limit.
        limit_segment_caches(root.path(), &current, 100).await;
        assert_eq!(left(), ["current"]);
    }

    #[test]
    fn frame_rate_follows_the_fastest_clip_within_limits() {
        assert_eq!(
            output_fps(&[clip(true, 25.0), clip(true, 29.97)], false),
            29.97
        );
        assert_eq!(output_fps(&[clip(true, 120.0)], false), 60.0);
        assert_eq!(output_fps(&[clip(true, 0.0)], false), DEFAULT_FPS);
        // Danmaku scrolls smoothly only at 60 FPS or more.
        assert_eq!(output_fps(&[clip(true, 30.0)], true), 60.0);
    }

    #[derive(Clone)]
    struct NoopReporter;

    #[async_trait::async_trait]
    impl ProgressReporterTrait for NoopReporter {
        async fn update(&self, _content: &str) {}
        async fn finish(&self, _success: bool, _message: &str) {}
    }

    async fn run_ffmpeg(args: &[&str], output: &Path) {
        let result = ffmpeg_command()
            .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
            .args(args)
            .arg("-y")
            .arg(output)
            .output()
            .await
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    /// Average RGB of the frame at `seconds`, scaled down to one pixel.
    async fn average_color(file: &Path, seconds: f64) -> [u8; 3] {
        let result = ffmpeg_command()
            .args(["-v", "error", "-ss", &seconds.to_string(), "-i"])
            .arg(file)
            .args([
                "-frames:v",
                "1",
                "-vf",
                "crop=iw/2:ih/2,scale=1:1",
                "-f",
                "rawvideo",
            ])
            .args(["-pix_fmt", "rgb24", "-"])
            .output()
            .await
            .unwrap();
        assert!(result.status.success());
        [result.stdout[0], result.stdout[1], result.stdout[2]]
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg with libx264"]
    async fn a_clip_longer_than_its_file_keeps_its_timeline_length() {
        let directory = tempfile::tempdir().unwrap();
        let red = directory.path().join("red.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=320x180:r=30:d=2",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-c:a",
                "aac",
            ],
            &red,
        )
        .await;
        let blue = directory.path().join("blue.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=320x180:r=30:d=2",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
            ],
            &blue,
        )
        .await;
        let output = directory.path().join("out.mp4");
        let clip = |path: &Path, duration: f64, has_audio: bool| RenderClip {
            id: String::new(),
            path: path.to_path_buf(),
            start: 0.0,
            duration,
            has_audio,
            fps: 30.0,
            file_duration: 2.0,
        };
        // The timeline allows a clip to end a little after its file.
        render_project(
            &NoopReporter,
            ProjectRender {
                clips: vec![clip(&red, 2.4, true), clip(&blue, 2.0, false)],
                overlays: Vec::new(),
                subtitle_ass: None,
                fonts_directory: None,
                danmus: Vec::new(),
                danmu_options: Danmu2AssOptions::default(),
                output: &output,
                segment_cache: None,
            },
        )
        .await
        .unwrap();
        let metadata = ffmpeg_utils::extract_video_metadata(&output).await.unwrap();
        assert!(
            (metadata.duration - 4.4).abs() < 0.1,
            "{}",
            metadata.duration
        );
        // The second clip starts where the timeline puts it, not early.
        let padded = average_color(&output, 2.2).await;
        assert!(padded[0] > 150 && padded[2] < 80, "{padded:?}");
        let second = average_color(&output, 2.6).await;
        assert!(second[2] > 150 && second[0] < 80, "{second:?}");

        // Far longer than its file, in segments of 1.5 s: those after the
        // file's end still show its last frame.
        render_in_segments(
            &NoopReporter,
            &ProjectRender {
                clips: vec![clip(&red, 6.0, true), clip(&blue, 2.0, false)],
                overlays: Vec::new(),
                subtitle_ass: None,
                fonts_directory: None,
                danmus: Vec::new(),
                danmu_options: Danmu2AssOptions::default(),
                output: &output,
                segment_cache: None,
            },
            SegmentLimits {
                seconds: 1.5,
                parts: 16,
            },
        )
        .await
        .unwrap();
        let metadata = ffmpeg_utils::extract_video_metadata(&output).await.unwrap();
        assert!(
            (metadata.duration - 8.0).abs() < 0.1,
            "{}",
            metadata.duration
        );
        for seconds in [3.2, 5.5] {
            let held = average_color(&output, seconds).await;
            assert!(held[0] > 150 && held[2] < 80, "{seconds} s: {held:?}");
        }
        let second = average_color(&output, 6.5).await;
        assert!(second[2] > 150 && second[0] < 80, "{second:?}");
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg with libx264"]
    async fn cuts_between_frames_keep_later_clips_on_the_timeline() {
        let directory = tempfile::tempdir().unwrap();
        let red = directory.path().join("red.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=320x180:r=30:d=2",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-c:a",
                "aac",
            ],
            &red,
        )
        .await;
        let blue = directory.path().join("blue.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=320x180:r=30:d=2",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
            ],
            &blue,
        )
        .await;
        let output = directory.path().join("out.mp4");
        // Twenty clips of 30.6 frames, red and blue in turn.
        let clips = (0..20)
            .map(|index| RenderClip {
                id: String::new(),
                path: if index % 2 == 0 { &red } else { &blue }.clone(),
                start: 0.0,
                duration: 1.02,
                has_audio: index % 2 == 0,
                fps: 30.0,
                file_duration: 2.0,
            })
            .collect();
        render_project(
            &NoopReporter,
            ProjectRender {
                clips,
                overlays: Vec::new(),
                subtitle_ass: None,
                fonts_directory: None,
                danmus: Vec::new(),
                danmu_options: Danmu2AssOptions::default(),
                output: &output,
                segment_cache: None,
            },
        )
        .await
        .unwrap();
        let metadata = ffmpeg_utils::extract_video_metadata(&output).await.unwrap();
        assert!(
            (metadata.duration - 20.4).abs() < 0.05,
            "{}",
            metadata.duration
        );
        // The last clip, blue, starts at 19.38 s as on the timeline.
        let before = average_color(&output, 19.33).await;
        assert!(before[0] > 150 && before[2] < 80, "{before:?}");
        let after = average_color(&output, 19.43).await;
        assert!(after[2] > 150 && after[0] < 80, "{after:?}");
    }

    fn text_danmu(ts: i64) -> DanmuEntry {
        DanmuEntry {
            ts,
            event_type: "danmu".into(),
            content: "弹幕".into(),
            emoji_content: None,
            user_name: None,
            price: None,
            sc_duration: None,
        }
    }

    /// Render 1-3 s of a red 30 FPS clip and 0.5-2.5 s of a larger blue
    /// 60 FPS clip without audio, then check the joined result.
    async fn render_two_clips(subtitles: bool, danmaku: bool, limits: SegmentLimits) {
        let directory = tempfile::Builder::new()
            .prefix("project [it's]")
            .tempdir()
            .unwrap();
        let red = directory.path().join("red clip.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=320x180:r=30:d=4",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=4",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-c:a",
                "aac",
            ],
            &red,
        )
        .await;
        let blue = directory.path().join("blue.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=640x360:r=60:d=3",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
            ],
            &blue,
        )
        .await;
        let output = directory.path().join("out.mp4");
        let ass = "[Script Info]\nScriptType: v4.00+\nPlayResX: 1920\nPlayResY: 1080\n\n\
            [V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour\n\
            Style: Default,Arial,48,&H00FFFFFF\n\n[Events]\n\
            Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
            Dialogue: 0,0:00:00.50,0:00:03.50,Default,,0,0,0,,hello\n";
        render_in_segments(
            &NoopReporter,
            &ProjectRender {
                clips: vec![
                    RenderClip {
                        id: String::new(),
                        path: red.clone(),
                        start: 1.0,
                        duration: 2.0,
                        has_audio: true,
                        fps: 30.0,
                        file_duration: 4.0,
                    },
                    RenderClip {
                        id: String::new(),
                        path: blue.clone(),
                        start: 0.5,
                        duration: 2.0,
                        has_audio: false,
                        fps: 60.0,
                        file_duration: 3.0,
                    },
                ],
                overlays: Vec::new(),
                subtitle_ass: subtitles.then(|| ass.to_string()),
                fonts_directory: None,
                danmus: if danmaku {
                    vec![text_danmu(500), text_danmu(2500)]
                } else {
                    Vec::new()
                },
                danmu_options: Danmu2AssOptions::default(),
                output: &output,
                segment_cache: None,
            },
            limits,
        )
        .await
        .unwrap();
        let metadata = ffmpeg_utils::extract_video_metadata(&output).await.unwrap();
        assert_eq!((metadata.width, metadata.height), (1920, 1080));
        assert_eq!(metadata.fps, 60.0);
        assert!(!metadata.audio_codec.is_empty());
        assert!(
            (metadata.duration - 4.0).abs() < 0.1,
            "{}",
            metadata.duration
        );
        let first = average_color(&output, 1.0).await;
        assert!(first[0] > 150 && first[2] < 80, "{first:?}");
        let second = average_color(&output, 3.0).await;
        assert!(second[2] > 150 && second[0] < 80, "{second:?}");
    }

    /// Average RGB of the `width`×`height` area at `x`, `y` in the frame at `seconds`.
    async fn area_color(file: &Path, seconds: f64, [x, y, width, height]: [u32; 4]) -> [u8; 3] {
        let result = ffmpeg_command()
            .args(["-v", "error", "-ss", &seconds.to_string(), "-i"])
            .arg(file)
            .args(["-frames:v", "1", "-vf"])
            .arg(format!("crop={width}:{height}:{x}:{y},scale=1:1"))
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .output()
            .await
            .unwrap();
        assert!(result.status.success());
        [result.stdout[0], result.stdout[1], result.stdout[2]]
    }

    /// Mean volume in dB of `duration` seconds of sound from `seconds` on.
    async fn mean_volume(file: &Path, seconds: f64, duration: f64) -> f64 {
        let result = ffmpeg_command()
            .args([
                "-hide_banner",
                "-nostdin",
                "-ss",
                &seconds.to_string(),
                "-t",
            ])
            .arg(duration.to_string())
            .arg("-i")
            .arg(file)
            .args(["-af", "volumedetect", "-f", "null", "-"])
            .output()
            .await
            .unwrap();
        let log = String::from_utf8_lossy(&result.stderr);
        log.lines()
            .find_map(|line| line.split("mean_volume: ").nth(1))
            .and_then(|value| value.trim_end_matches(" dB").trim().parse().ok())
            .unwrap_or_else(|| panic!("no volume in {log}"))
    }

    async fn lay_videos_and_pictures(limits: SegmentLimits) {
        let directory = tempfile::tempdir().unwrap();
        // A silent red clip under a blue video with a tone and a green picture.
        let red = directory.path().join("red.mp4");
        run_ffmpeg(
            &["-f", "lavfi", "-i", "color=c=red:s=320x180:r=30:d=4"]
                .into_iter()
                .chain(["-c:v", "libx264", "-preset", "ultrafast"])
                .collect::<Vec<_>>(),
            &red,
        )
        .await;
        let blue = directory.path().join("blue.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=320x180:r=30:d=6",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=6",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-c:a",
                "aac",
            ],
            &blue,
        )
        .await;
        let green = directory.path().join("green.png");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=lime:s=200x100",
                "-frames:v",
                "1",
            ],
            &green,
        )
        .await;
        let video = |start: f64, end: f64, source_start: f64, [x, y, width, rotation]: [f64; 4]| {
            RenderOverlay {
                start,
                end,
                layer: OverlayLayer::Video(VideoLayer {
                    path: blue.clone(),
                    source_start,
                    duration: 6.0,
                    has_audio: true,
                    x,
                    y,
                    width,
                    rotation,
                    volume: 1.0,
                }),
            }
        };
        let output = directory.path().join("out.mp4");
        render_in_segments(
            &NoopReporter,
            &ProjectRender {
                clips: vec![RenderClip {
                    id: String::new(),
                    path: red.clone(),
                    start: 0.0,
                    duration: 4.0,
                    has_audio: false,
                    fps: 30.0,
                    file_duration: 4.0,
                }],
                overlays: vec![
                    // The top left quarter from 1 s to 3 s.
                    video(1.0, 3.0, 2.0, [480.0, 270.0, 960.0, 0.0]),
                    RenderOverlay {
                        start: 2.0,
                        end: 3.5,
                        layer: OverlayLayer::Image {
                            png: std::fs::read(&green).unwrap(),
                            x: 1600.0,
                            y: 900.0,
                        },
                    },
                    // Turned on its side, still centered where it was placed.
                    video(0.0, 1.0, 2.0, [1440.0, 540.0, 400.0, 90.0]),
                    // The bottom left quarter from 2 s to 3.5 s, though the
                    // file has only 1 s left from its in point.
                    video(2.0, 3.5, 5.0, [480.0, 810.0, 960.0, 0.0]),
                ],
                subtitle_ass: None,
                fonts_directory: None,
                danmus: Vec::new(),
                danmu_options: Danmu2AssOptions::default(),
                output: &output,
                segment_cache: None,
            },
            limits,
        )
        .await
        .unwrap();

        // Overlays reaching past their span do not make the video longer.
        let metadata = ffmpeg_utils::extract_video_metadata(&output).await.unwrap();
        assert!(
            (metadata.duration - 4.0).abs() < 0.05,
            "{}",
            metadata.duration
        );
        let quarter = [100, 100, 760, 340];
        let is = |color: [u8; 3], [r, g, b]: [bool; 3]| {
            let high = |value: u8, on: bool| if on { value > 150 } else { value < 80 };
            high(color[0], r) && high(color[1], g) && high(color[2], b)
        };
        let (red, blue, green) = (
            [true, false, false],
            [false, false, true],
            [false, true, false],
        );
        for (seconds, area, expected) in [
            (0.5, quarter, red),
            (1.5, quarter, blue),
            (3.5, quarter, red),
            (1.5, [1620, 910, 160, 80], red),
            (2.5, [1620, 910, 160, 80], green),
            (3.8, [1620, 910, 160, 80], red),
            // 400 wide and 225 high on its side: blue 1440±100 across, red past it.
            (0.5, [1360, 330, 160, 420], blue),
            (0.5, [1560, 330, 60, 420], red),
            (1.5, [1360, 330, 160, 420], red),
            // Its last frame stays until the overlay ends.
            (2.5, [100, 640, 760, 340], blue),
            (3.3, [100, 640, 760, 340], blue),
            (3.8, [100, 640, 760, 340], red),
        ] {
            let color = area_color(&output, seconds, area).await;
            assert!(is(color, expected), "{seconds} s {area:?}: {color:?}");
        }
        // The video's tone plays while it is shown, over the silent clip.
        assert!(mean_volume(&output, 1.2, 0.6).await > -40.0);
        assert!(mean_volume(&output, 3.2, 0.6).await < -60.0);
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg with libx264"]
    async fn ffmpeg_lays_videos_and_pictures_over_the_clips() {
        lay_videos_and_pictures(SEGMENT_LIMITS).await;
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg with libx264"]
    async fn ffmpeg_lays_overlays_across_segments() {
        // Seams at 4/3 s and 8/3 s, within the overlays: the video from 1 s
        // to 3 s, the picture, and the video whose file ends at 3 s.
        lay_videos_and_pictures(SegmentLimits {
            seconds: 1.5,
            parts: 16,
        })
        .await;
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg with libx264"]
    async fn ffmpeg_renders_trimmed_clips_on_one_canvas() {
        render_two_clips(false, false, SEGMENT_LIMITS).await;
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg with libass and libx264"]
    async fn ffmpeg_renders_subtitles_and_danmaku_on_the_joined_timeline() {
        render_two_clips(true, true, SEGMENT_LIMITS).await;
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg with libass and libx264"]
    async fn ffmpeg_renders_subtitles_and_danmaku_across_segments() {
        // Each clip in three segments, under the subtitle and the danmaku.
        render_two_clips(
            true,
            true,
            SegmentLimits {
                seconds: 0.75,
                parts: 16,
            },
        )
        .await;
    }

    /// The grey of every frame of `file`, its luma averaged.
    async fn frame_greys(file: &Path) -> Vec<u8> {
        let result = ffmpeg_command()
            .args(["-v", "error", "-i"])
            .arg(file)
            .args(["-vf", "extractplanes=y,scale=1:1", "-f", "rawvideo", "-"])
            .output()
            .await
            .unwrap();
        assert!(result.status.success());
        result.stdout
    }

    /// The sound of `file`, mono.
    async fn mono_samples(file: &Path) -> Vec<u8> {
        let result = ffmpeg_command()
            .args(["-v", "error", "-i"])
            .arg(file)
            .args(["-map", "0:a", "-ac", "1", "-f", "s16le", "-"])
            .output()
            .await
            .unwrap();
        assert!(result.status.success());
        result.stdout
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg with libx264"]
    async fn segments_cut_within_a_clip_join_without_a_seam() {
        let directory = tempfile::tempdir().unwrap();
        // Every frame a grey of its own, and a steady tone.
        let source = directory.path().join("frames.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=320x180:r=25:d=6,geq=lum='16+mod(N*7,210)':cb=128:cr=128",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=6",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-qp",
                "0",
                "-c:a",
                "aac",
            ],
            &source,
        )
        .await;
        // With the encoder exports use here, and with the one they fall back to.
        for encoder in [hwaccel::get_x264_encoder().await, "libx264"] {
            assert_seamless(&directory.path().join(encoder), &source, encoder).await;
        }
    }

    async fn assert_seamless(directory: &Path, source: &Path, encoder: &'static str) {
        std::fs::create_dir(directory).unwrap();
        let whole = directory.join("whole.mp4");
        let cut = directory.join("cut.mp4");
        // 4 s from 0.51 s, between the file's frames: in one segment, and in
        // four cut at 1, 2 and 3 s.
        for (output, seconds) in [(&whole, 60.0), (&cut, 1.3)] {
            render_with_encoder(
                &NoopReporter,
                &ProjectRender {
                    clips: vec![RenderClip {
                        id: String::new(),
                        path: source.to_path_buf(),
                        start: 0.51,
                        duration: 4.0,
                        has_audio: true,
                        fps: 25.0,
                        file_duration: 6.0,
                    }],
                    overlays: Vec::new(),
                    subtitle_ass: None,
                    fonts_directory: None,
                    danmus: Vec::new(),
                    danmu_options: Danmu2AssOptions::default(),
                    output,
                    segment_cache: None,
                },
                SegmentLimits { seconds, parts: 16 },
                encoder,
            )
            .await
            .unwrap();
        }
        // Frame n shows the file's frame nearest 0.51 s + n / 25, its 13 + n,
        // with none repeated or skipped at the seams. Its grey is read in
        // full range.
        let greys = frame_greys(&cut).await;
        assert_eq!(greys.len(), 100);
        for (n, grey) in greys.into_iter().enumerate() {
            let expected = ((13 + n as i64) * 7 % 210) * 255 / 219;
            assert!(
                (i64::from(grey) - expected).abs() <= 3,
                "{encoder} frame {n}: {grey}, not {expected}"
            );
        }
        // The sound is the same as rendered in one piece, sample for sample.
        let sound = mono_samples(&cut).await;
        assert_eq!(sound.len(), 2 * 4 * 48000);
        assert!(sound == mono_samples(&whole).await);
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg with libass and libx264"]
    async fn a_render_uses_the_segments_it_finds_rendered_again() {
        let directory = tempfile::tempdir().unwrap();
        let red = directory.path().join("red.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=320x180:r=30:d=2",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-c:a",
                "aac",
            ],
            &red,
        )
        .await;
        let blue = directory.path().join("blue.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=320x180:r=30:d=2",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
            ],
            &blue,
        )
        .await;
        let cache = directory.path().join("cache");
        let plain = plain_ids(4);
        // Three segments of a red and a blue clip each.
        let ids = [
            anchor_id(0),
            plain[0].clone(),
            anchor_id(100),
            plain[1].clone(),
            anchor_id(200),
            plain[2].clone(),
        ];
        let clips = |before: bool| {
            let mut clips: Vec<_> = ids
                .iter()
                .enumerate()
                .map(|(index, id)| RenderClip {
                    id: id.clone(),
                    path: if index % 2 == 0 { &red } else { &blue }.clone(),
                    start: 0.0,
                    duration: 1.5,
                    has_audio: index % 2 == 0,
                    fps: 30.0,
                    file_duration: 2.0,
                })
                .collect();
            if before {
                // 30 frames of blue before them.
                clips.insert(
                    0,
                    RenderClip {
                        id: plain[3].clone(),
                        path: blue.clone(),
                        start: 0.0,
                        duration: 1.0,
                        has_audio: false,
                        fps: 30.0,
                        file_duration: 2.0,
                    },
                );
            }
            clips
        };
        let ass = |shift: f64, text: &str| {
            format!(
                "[Script Info]\nScriptType: v4.00+\nPlayResX: 1920\nPlayResY: 1080\n\n\
                 [V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour\n\
                 Style: Default,Arial,48,&H00FFFFFF\n\n[Events]\n\
                 Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
                 Dialogue: 0,0:00:0{:.2},0:00:0{:.2},Default,,0,0,0,,hello\n\
                 Dialogue: 0,0:00:0{:.2},0:00:0{:.2},Default,,0,0,0,,{text}\n",
                0.5 + shift,
                1.0 + shift,
                6.5 + shift,
                7.0 + shift,
            )
        };
        let render = |clips: Vec<RenderClip>, ass: String, output: PathBuf, cached: bool| {
            let cache = cache.clone();
            async move {
                render_in_segments(
                    &NoopReporter,
                    &ProjectRender {
                        clips,
                        overlays: Vec::new(),
                        subtitle_ass: Some(ass),
                        fonts_directory: None,
                        danmus: Vec::new(),
                        danmu_options: Danmu2AssOptions::default(),
                        output: &output,
                        segment_cache: cached.then_some(cache.as_path()),
                    },
                    SEGMENT_LIMITS,
                )
                .await
                .unwrap()
            }
        };
        let output = directory.path().join("out.mp4");
        let stats = |segments, reused| RenderStats { segments, reused };
        assert_eq!(
            render(clips(false), ass(0.0, "bye"), output.clone(), true).await,
            stats(3, 0)
        );
        // Nothing changed: nothing to render.
        assert_eq!(
            render(clips(false), ass(0.0, "bye"), output.clone(), true).await,
            stats(3, 3)
        );
        // A subtitle changed: only its segment.
        assert_eq!(
            render(clips(false), ass(0.0, "again"), output.clone(), true).await,
            stats(3, 2)
        );
        // A segment's sound left without its video, as by a render stopped
        // between renaming them: it is rendered again.
        let videos: Vec<_> = std::fs::read_dir(&cache)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "mp4"))
            .collect();
        std::fs::remove_file(&videos[0]).unwrap();
        assert_eq!(
            render(clips(false), ass(0.0, "again"), output.clone(), true).await,
            stats(3, 2)
        );
        // A second before them all: they are all found again.
        assert_eq!(
            render(clips(true), ass(1.0, "again"), output.clone(), true).await,
            stats(4, 3)
        );
        // The cache keeps the segments of the last render, and nothing else.
        let kept = std::fs::read_dir(&cache).unwrap().count();
        assert_eq!(kept, 8);

        // As rendered without a cache.
        let fresh = directory.path().join("fresh.mp4");
        assert_eq!(
            render(clips(true), ass(1.0, "again"), fresh.clone(), false).await,
            stats(4, 0)
        );
        assert_eq!(mono_samples(&output).await, mono_samples(&fresh).await);
        for seconds in [0.5, 1.75, 3.25, 4.75, 6.25, 7.75, 9.25] {
            let (cached, rendered) = (
                average_color(&output, seconds).await,
                average_color(&fresh, seconds).await,
            );
            let close = cached.iter().zip(rendered).all(|(a, b)| a.abs_diff(b) <= 8);
            assert!(
                close,
                "{seconds} s: {cached:?} cached, {rendered:?} rendered"
            );
        }
        let metadata = ffmpeg_utils::extract_video_metadata(&output).await.unwrap();
        assert!(
            (metadata.duration - 10.0).abs() < 0.05,
            "{}",
            metadata.duration
        );
    }
}
