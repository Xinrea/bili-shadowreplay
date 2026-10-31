//! Structured native media jobs. Plans describe stream routes and options;
//! they never contain FFmpeg command-line arguments.

use std::{collections::BTreeMap, fmt, path::PathBuf, sync::Arc};

mod audio;
mod audio_encoder;
mod decode;
mod execute;
mod graph;
mod input;
mod packet_clock;
mod raw_video;
mod subtitle;
mod util;
mod video;
mod video_clock;
mod video_encoder;
mod video_processor;

#[cfg(test)]
mod single_input_tests;

pub type AvOptions = BTreeMap<String, String>;
pub(super) type EncodedPacketSink<'a> =
    dyn FnMut(ffmpeg_next::Packet, ffmpeg_next::Rational) -> Result<(), String> + 'a;

#[derive(Debug, Clone, Default)]
pub struct MediaPlan {
    pub inputs: Vec<MediaInput>,
    pub outputs: Vec<MediaOutput>,
    pub routes: Vec<StreamRoute>,
    pub filter_graph: Option<FilterGraphPlan>,
    pub resources: Vec<MediaResourceLease>,
    pub progress_total: MediaProgressTotal,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum MediaProgressTotal {
    #[default]
    FirstInput,
    AllInputs,
    Known(f64),
}

#[derive(Debug, Clone)]
pub struct MediaInput {
    pub source: MediaSource,
    /// Positive explicit limit; None selects automatic video decoding threads
    /// and leaves audio at its codec default. Codec AVOptions still apply last.
    pub decoder_threads: Option<usize>,
    pub decoder_options: AvOptions,
    pub format: Option<String>,
    pub options: AvOptions,
    pub seek: Option<InputSeek>,
    pub duration: Option<f64>,
    pub copy_timestamps: bool,
    // Dropped with the worker's plan, after its input context has closed.
    _lease: Option<Arc<tempfile::TempPath>>,
}

impl MediaInput {
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            source: MediaSource::File(path.into()),
            decoder_threads: None,
            decoder_options: AvOptions::new(),
            format: None,
            options: AvOptions::new(),
            seek: None,
            duration: None,
            copy_timestamps: false,
            _lease: None,
        }
    }

    pub fn generated_video(source: ColorSource) -> Self {
        let mut input = Self::file(PathBuf::new());
        input.source = MediaSource::GeneratedVideo(source);
        input
    }
    pub fn generated_audio(source: SilenceSource) -> Self {
        let mut input = Self::file(PathBuf::new());
        input.source = MediaSource::GeneratedAudio(source);
        input
    }
    pub fn raw_video(spec: RawVideoSpec) -> Self {
        let mut input = Self::file(PathBuf::new());
        input.source = MediaSource::RawVideo(spec);
        input
    }
    pub(super) fn file_path(&self) -> Result<&std::path::Path, String> {
        match &self.source {
            MediaSource::File(path) => Ok(path),
            _ => Err("该媒体输入不是文件源".into()),
        }
    }

    /// Keep a temporary input alive until the native worker has closed it,
    /// including when its asynchronous caller has already been cancelled.
    pub fn owned_file(path: tempfile::TempPath) -> Self {
        let mut input = Self::file(path.to_path_buf());
        input._lease = Some(Arc::new(path));
        input
    }
}

/// File paths and generated/live sources are explicit; no pseudo pipe names.
#[derive(Debug, Clone)]
pub enum MediaSource {
    File(PathBuf),
    GeneratedVideo(ColorSource),
    GeneratedAudio(SilenceSource),
    RawVideo(RawVideoSpec),
}
#[derive(Debug, Clone)]
pub struct ColorSource {
    pub color: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate_num: u32,
    pub frame_rate_den: u32,
}
#[derive(Debug, Clone)]
pub struct SilenceSource {
    pub sample_rate: u32,
    pub channel_layout: String,
}
#[derive(Debug, Clone)]
pub enum StreamSource {
    Input {
        input: usize,
        selector: StreamSelector,
    },
    Filter {
        label: String,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Audio,
}
#[derive(Debug, Clone)]
pub struct GraphInput {
    pub input: usize,
    pub selector: StreamSelector,
    pub label: String,
    pub required: bool,
}
#[derive(Debug, Clone)]
pub struct GraphOutput {
    pub label: String,
    pub kind: MediaKind,
}
#[derive(Debug, Clone)]
pub struct FilterGraphPlan {
    pub spec: String,
    pub inputs: Vec<GraphInput>,
    pub outputs: Vec<GraphOutput>,
}
/// Shared ownership keeps all scratch assets alive through worker and renderer
/// shutdown, including when the awaiting future is dropped.
#[derive(Clone)]
pub struct MediaResourceLease {
    _resource: Arc<dyn Send + Sync>,
}
impl MediaResourceLease {
    pub fn new<T: Send + Sync + 'static>(resource: T) -> Self {
        Self {
            _resource: Arc::new(resource),
        }
    }
}
impl fmt::Debug for MediaResourceLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MediaResourceLease(..)")
    }
}
#[derive(Default)]
pub struct MediaBindings {
    /// Optional diagnostic collector for the complex graph worker.
    pub profiler: Option<crate::profiling::MediaProfiler>,
    pub raw_video: Vec<(usize, RawVideoInput)>,
    /// Original plan route index. Factories cross into the media worker; their
    /// thread-affine processors and GPU handles are created and dropped there.
    pub video_processors: Vec<(usize, Box<dyn VideoFrameProcessorFactory>)>,
}
pub use ffmpeg_next::{frame::Video as VideoFrame, Rational as VideoRational};
pub use raw_video::{
    raw_video_channel, RawVideoAlpha, RawVideoCancelHandle, RawVideoFrame, RawVideoInput,
    RawVideoSender, RawVideoSpec, RawVideoStats,
};
pub use video_processor::{VideoFrameInfo, VideoFrameProcessor, VideoFrameProcessorFactory};

/// Parse a decimal frame rate using the same FFmpeg parser as input `-r`.
pub fn parse_frame_rate(value: f64) -> Result<(u32, u32), MediaError> {
    if !value.is_finite() || value <= 0.0 {
        return Err("视频帧率无效".to_owned().into());
    }
    let value =
        std::ffi::CString::new(value.to_string()).map_err(|e| MediaError::from(e.to_string()))?;
    let mut rate = ffmpeg_next::ffi::AVRational { num: 0, den: 1 };
    if unsafe { ffmpeg_next::ffi::av_parse_video_rate(&mut rate, value.as_ptr()) } < 0
        || rate.num <= 0
        || rate.den <= 0
    {
        return Err("视频帧率无效".to_owned().into());
    }
    Ok((rate.num as u32, rate.den as u32))
}

#[derive(Debug, Clone, Copy)]
pub struct InputSeek {
    pub seconds: f64,
    pub accurate: bool,
}

#[derive(Debug, Clone)]
pub struct MediaOutput {
    pub path: PathBuf,
    pub format: Option<String>,
    pub seek: Option<f64>,
    pub duration: Option<f64>,
    pub negative_ts: NegativeTs,
    pub faststart: bool,
    pub options: AvOptions,
}

impl MediaOutput {
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            format: None,
            seek: None,
            duration: None,
            negative_ts: NegativeTs::Auto,
            faststart: false,
            options: AvOptions::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub enum NegativeTs {
    #[default]
    Auto,
    Disabled,
    MakeZero,
}

#[derive(Debug, Clone)]
pub struct StreamRoute {
    pub source: StreamSource,
    pub output: usize,
    pub required: bool,
    pub mode: StreamMode,
}

#[derive(Debug, Clone, Copy)]
pub enum StreamSelector {
    AutoVideo,
    AutoAudio,
    /// Map every audio stream, as an explicit `-map i:a?` does.
    AllAudio,
    AutoSubtitle,
    FirstVideo,
    FirstAudio,
    Index(usize),
}

#[derive(Debug, Clone)]
pub enum StreamMode {
    Copy,
    Video(VideoEncodeOptions),
    Audio(AudioEncodeOptions),
    Subtitle(SubtitleEncodeOptions),
}

/// `None` uses the output container's default subtitle encoder, as automatic
/// stream selection does. Explicit names select the first subtitle stream.
#[derive(Debug, Clone, Default)]
pub struct SubtitleEncodeOptions {
    pub encoder: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VideoEncodeOptions {
    pub encoder: String,
    pub filter: Option<String>,
    pub autorotate: bool,
    pub frame_rate: FrameRatePolicy,
    pub bit_rate: Option<u64>,
    pub max_rate: Option<u64>,
    pub buffer_size: Option<u64>,
    pub gop: Option<u32>,
    pub tag: Option<String>,
    pub threads: Option<usize>,
    pub options: AvOptions,
    pub hardware_device: Option<HardwareDevice>,
    /// Feed normalized NV12 frames through a bound worker-local processor
    /// before encoding. The processor supplies the encoder's actual frame pool.
    pub frame_processor: bool,
}

impl Default for VideoEncodeOptions {
    fn default() -> Self {
        Self {
            encoder: "libx264".into(),
            filter: None,
            autorotate: true,
            frame_rate: FrameRatePolicy::Auto,
            bit_rate: None,
            max_rate: None,
            buffer_size: None,
            gop: None,
            tag: None,
            threads: None,
            options: AvOptions::new(),
            hardware_device: None,
            frame_processor: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub enum FrameRatePolicy {
    #[default]
    Auto,
    Passthrough,
    Variable,
    Constant {
        num: u32,
        den: u32,
    },
}

#[derive(Debug, Clone)]
pub struct HardwareDevice {
    pub kind: String,
    pub device: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AudioEncodeOptions {
    pub encoder: String,
    pub bit_rate: Option<u64>,
    pub filter: Option<String>,
    pub sample_rate: Option<u32>,
    pub channel_layout: Option<String>,
    pub options: AvOptions,
}

impl Default for AudioEncodeOptions {
    fn default() -> Self {
        Self {
            encoder: "aac".into(),
            bit_rate: None,
            filter: None,
            sample_rate: None,
            channel_layout: None,
            options: AvOptions::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaStage {
    Processing,
    Finalizing,
    Completed,
}

#[derive(Debug, Clone)]
pub struct MediaProgress {
    pub processed: f64,
    pub total: Option<f64>,
    pub speed: Option<f64>,
    pub stage: MediaStage,
}

pub type MediaProgressCallback = Box<dyn FnMut(MediaProgress) + Send + 'static>;

#[derive(Debug, Clone)]
pub struct MediaJobResult {
    pub outputs: Vec<PathBuf>,
    pub video_frames: u64,
    pub audio_samples: u64,
    pub processed: f64,
}

#[derive(Debug, Clone)]
pub struct MediaError {
    pub message: String,
}

impl fmt::Display for MediaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for MediaError {}
impl From<String> for MediaError {
    fn from(message: String) -> Self {
        Self { message }
    }
}

/// Execute a validated native plan. Progress callbacks run on the media worker
/// and must be nonblocking (a watch-channel send is suitable).
pub async fn execute_media_plan(
    plan: MediaPlan,
    progress: Option<MediaProgressCallback>,
) -> Result<MediaJobResult, MediaError> {
    execute_with_sources(plan, MediaBindings::default(), progress).await
}

/// Open the real encoder with the render options, encode three synthetic
/// 320x240/30 fps frames, and drain it. Registry membership alone is not a probe.
pub async fn probe_video_encoder(options: VideoEncodeOptions) -> Result<(), MediaError> {
    execute::probe(options).await.map_err(MediaError::from)
}

/// Bind exclusive live sources to a cloneable declarative media plan.
pub async fn execute_with_sources(
    plan: MediaPlan,
    sources: MediaBindings,
    progress: Option<MediaProgressCallback>,
) -> Result<MediaJobResult, MediaError> {
    if plan.filter_graph.is_some()
        || plan.inputs.len() != 1
        || plan.outputs.len() != 1
        || !sources.raw_video.is_empty()
        || !sources.video_processors.is_empty()
        || plan.routes.iter().any(|route| {
            matches!(&route.mode,
            StreamMode::Video(options) if options.frame_processor)
        })
    {
        graph::run(plan, sources, progress)
            .await
            .map_err(Into::into)
    } else {
        execute::run(plan, progress).await.map_err(Into::into)
    }
}
