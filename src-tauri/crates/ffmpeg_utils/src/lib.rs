//! Native media probing, image/audio profiles and structured media execution
//! shared by the application and recorder. CLI helpers remain for callers
//! that have not yet migrated and for optional test oracles.
//!
//! The recorder probes every downloaded HLS segment to notice a resolution
//! change; the application uses the same description for finished recordings.
//!
//! What does not belong here: anything that needs application state, such as
//! progress reporting, subtitle generation or application-specific filter graphs.

mod audio;
mod capabilities;
mod command;
mod hls;
mod image;
mod media;
mod metadata;
mod output;
mod video_frame;

pub use audio::{
    decode_audio, decode_audio_range, encode_flac_preview, encode_mp3, encode_opus_sample,
    encode_opus_upload, import_audio, AudioDecodeOptions, AudioDecodeResult, AudioRange, AudioSink,
    AudioTiming, ImportAudioMode,
};
pub use capabilities::{available_video_encoders, linked_ffmpeg_version};
pub use command::{ffmpeg_command, ffmpeg_path, ffprobe_command, ffprobe_path, path_str};
pub use hls::{record_to_hls, HlsRecordingOptions};
pub use image::normalize_image_to_png;
pub use media::{
    execute_media_plan, execute_with_sources, parse_frame_rate, probe_video_encoder,
    raw_video_channel, AudioEncodeOptions, AvOptions, ColorSource, FilterGraphPlan,
    FrameRatePolicy, GraphInput, GraphOutput, HardwareDevice, InputSeek, MediaBindings, MediaError,
    MediaInput, MediaJobResult, MediaKind, MediaOutput, MediaPlan, MediaProgress,
    MediaProgressCallback, MediaProgressTotal, MediaResourceLease, MediaSource, MediaStage,
    NegativeTs, RawVideoAlpha, RawVideoCancelHandle, RawVideoFrame, RawVideoInput, RawVideoSender,
    RawVideoSpec, RawVideoStats, SilenceSource, StreamMode, StreamRoute, StreamSelector,
    StreamSource, SubtitleEncodeOptions, VideoEncodeOptions,
};
pub use metadata::{
    extract_video_metadata, extract_video_metadata_with_init, probe_media, AudioStreamMetadata,
    MediaProbe, VideoMetadata,
};
pub use video_frame::{extract_jpeg, write_jpeg};
