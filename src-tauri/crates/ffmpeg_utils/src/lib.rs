//! Native media probing, audio decoding/encoding and FFmpeg CLI plumbing
//! shared by the application and recorder.
//!
//! The recorder probes every downloaded HLS segment to notice a resolution
//! change; the application uses the same description for finished recordings.
//!
//! What does not belong here: anything that needs application state, such as
//! progress reporting, subtitle generation or application-specific filter graphs.

mod audio;
mod capabilities;
mod command;
mod metadata;
mod video_frame;

pub use audio::{
    decode_audio, decode_audio_range, encode_flac_preview, encode_mp3, encode_opus_sample,
    encode_opus_upload, import_audio, AudioDecodeOptions, AudioDecodeResult, AudioRange, AudioSink,
    AudioTiming, ImportAudioMode,
};
pub use capabilities::{available_video_encoders, linked_ffmpeg_version};
pub use command::{ffmpeg_command, ffmpeg_path, ffprobe_command, ffprobe_path, path_str};
pub use metadata::{
    extract_video_metadata, extract_video_metadata_with_init, probe_media, AudioStreamMetadata,
    MediaProbe, VideoMetadata,
};
pub use video_frame::{extract_jpeg, write_jpeg};
