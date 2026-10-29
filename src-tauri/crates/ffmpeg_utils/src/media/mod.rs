//! Structured native media jobs. Plans describe stream routes and options;
//! they never contain FFmpeg command-line arguments.

use std::{collections::BTreeMap, fmt, path::PathBuf, sync::Arc};

mod audio;
mod execute;
mod input;
mod packet_clock;
mod subtitle;
mod util;
mod video;
mod video_clock;

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
}

#[derive(Debug, Clone)]
pub struct MediaInput {
    pub path: PathBuf,
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
            path: path.into(),
            format: None,
            options: AvOptions::new(),
            seek: None,
            duration: None,
            copy_timestamps: false,
            _lease: None,
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
    pub input: usize,
    pub output: usize,
    pub selector: StreamSelector,
    pub required: bool,
    pub mode: StreamMode,
}

#[derive(Debug, Clone, Copy)]
pub enum StreamSelector {
    AutoVideo,
    AutoAudio,
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
    execute::run(plan, progress).await.map_err(MediaError::from)
}

/// Open the real encoder with the render options, encode three synthetic
/// 320x240/30 fps frames, and drain it. Registry membership alone is not a probe.
pub async fn probe_video_encoder(options: VideoEncodeOptions) -> Result<(), MediaError> {
    execute::probe(options).await.map_err(MediaError::from)
}
