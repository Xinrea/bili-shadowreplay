//! FFmpeg/ffprobe plumbing shared by the desktop application and the
//! `recorder` crate.
//!
//! Both layers shell out to the same ffmpeg binaries: the recorder probes every
//! downloaded HLS segment to notice a resolution change, while the application
//! probes finished recordings and drives the editing/encoding pipeline. Binary
//! lookup, the platform quirks (no console window on Windows) and the metadata
//! type live here so that a media file is described the same way no matter
//! which layer looks at it.
//!
//! What does not belong here: anything that needs application state, such as
//! progress reporting, subtitle generation or ffmpeg filter graphs.

mod command;
mod metadata;

pub use command::{ffmpeg_command, ffmpeg_path, ffprobe_command, ffprobe_path, path_str};
pub use metadata::{extract_video_metadata, extract_video_metadata_with_init, VideoMetadata};
