//! Native media probing, PCM decoding and FFmpeg CLI plumbing shared by the
//! application and recorder. ffmpeg-next owns demuxing/decoding/resampling;
//! encoding uses the command factory and its process cancellation defaults.
//!
//! The recorder probes every downloaded HLS segment to notice a resolution
//! change; the application uses the same description for finished recordings.
//!
//! What does not belong here: anything that needs application state, such as
//! progress reporting, subtitle generation or ffmpeg filter graphs.

mod audio;
mod command;
mod metadata;

pub use audio::{decode_audio, AudioDecodeOptions, AudioDecodeResult, AudioSink};
pub use command::{ffmpeg_command, ffmpeg_path, ffprobe_command, ffprobe_path, path_str};
pub use metadata::{
    extract_video_metadata, extract_video_metadata_with_init, probe_media, AudioStreamMetadata,
    MediaProbe, VideoMetadata,
};
