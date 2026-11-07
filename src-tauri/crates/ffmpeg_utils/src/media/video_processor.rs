//! Worker-local video processing between frame scheduling and the encoder.
//! A processor may retain a bounded number of frames, including GPU resources.

use std::sync::{atomic::AtomicBool, Arc};

use super::{VideoFrame, VideoRational};

#[derive(Debug, Clone, Copy)]
pub struct VideoFrameInfo {
    pub width: u32,
    pub height: u32,
    pub frame_rate: VideoRational,
    /// PTS and duration passed to the processor use this encoder time base.
    pub time_base: VideoRational,
}

/// Created and used on the media worker. Implementations need not be Send:
/// Skia/Vulkan contexts must stay on their owning thread.
pub trait VideoFrameProcessor {
    /// An owning frame description, including a reference to the hardware pool
    /// when applicable. No initialized pixel data is needed in this template.
    fn output_template(&self) -> Result<VideoFrame, String>;
    /// Return ready frames in submission order, retaining their PTS/duration.
    /// At most eight submitted frames may remain pending after this call.
    fn process(&mut self, frame: VideoFrame) -> Result<Vec<VideoFrame>, String>;
    /// Return all remaining frames before the encoder is drained.
    fn flush(&mut self) -> Result<Vec<VideoFrame>, String>;
}

pub trait VideoFrameProcessorFactory: Send {
    fn create(
        self: Box<Self>,
        info: VideoFrameInfo,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Box<dyn VideoFrameProcessor>, String>;
}

impl<F> VideoFrameProcessorFactory for F
where
    F: FnOnce(VideoFrameInfo, Arc<AtomicBool>) -> Result<Box<dyn VideoFrameProcessor>, String>
        + Send,
{
    fn create(
        self: Box<Self>,
        info: VideoFrameInfo,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Box<dyn VideoFrameProcessor>, String> {
        (*self)(info, cancelled)
    }
}

#[cfg(test)]
mod tests;
