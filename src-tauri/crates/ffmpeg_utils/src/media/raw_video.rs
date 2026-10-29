//! Bounded premultiplied/straight BGRA input for native renderers. The plan
//! holds only RawVideoSpec; the non-clone channel binding is consumed per job.
//! Queue closure is independently callable, even while a media worker is queued.

use ffmpeg_next::{self as ffmpeg, ffi, frame, Rational};
use std::{
    collections::VecDeque,
    ffi::CString,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};

const POLL: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawVideoAlpha {
    Straight,
    Premultiplied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawVideoSpec {
    pub width: u32,
    pub height: u32,
    pub frame_rate_num: u32,
    pub frame_rate_den: u32,
    pub frame_count: u64,
    pub alpha: RawVideoAlpha,
}
impl RawVideoSpec {
    pub fn bgra(
        width: u32,
        height: u32,
        fps: f64,
        frame_count: u64,
        alpha: RawVideoAlpha,
    ) -> Result<Self, String> {
        if !fps.is_finite() || !(1.0..=240.0).contains(&fps) {
            return Err("invalid raw BGRA framerate".into());
        }
        let text = CString::new(fps.to_string()).map_err(|error| error.to_string())?;
        let mut rate = ffi::AVRational { num: 0, den: 1 };
        if unsafe { ffi::av_parse_video_rate(&mut rate, text.as_ptr()) } < 0
            || rate.num <= 0
            || rate.den <= 0
        {
            return Err("invalid raw BGRA framerate".into());
        }
        let spec = Self {
            width,
            height,
            frame_rate_num: rate.num as u32,
            frame_rate_den: rate.den as u32,
            frame_count,
            alpha,
        };
        spec.validate()?;
        Ok(spec)
    }
    pub(crate) fn validate(&self) -> Result<usize, String> {
        let pixels = u64::from(self.width) * u64::from(self.height);
        if self.width == 0
            || self.height == 0
            || self.width > 8192
            || self.height > 8192
            || pixels > 16_777_216
            || self.frame_rate_num == 0
            || self.frame_rate_den == 0
            || self.frame_rate_num > i32::MAX as u32
            || self.frame_rate_den > i32::MAX as u32
            || self.frame_rate_num < self.frame_rate_den
            || u64::from(self.frame_rate_num) > 240 * u64::from(self.frame_rate_den)
            || self.frame_count == 0
        {
            return Err("invalid raw BGRA dimensions, rate or frame count".into());
        }
        let max = (7 * 24 * 60 * 60 * u64::from(self.frame_rate_num))
            .div_ceil(u64::from(self.frame_rate_den));
        if self.frame_count > max + 1 {
            return Err("raw BGRA source exceeds seven days".into());
        }
        usize::try_from(pixels * 4).map_err(|_| "raw BGRA frame size exceeds address space".into())
    }
    pub(crate) fn rate(&self) -> Rational {
        Rational(self.frame_rate_num as i32, self.frame_rate_den as i32)
    }
    pub(crate) fn time_base(&self) -> Rational {
        self.rate().invert()
    }
    /// A graph can negotiate its sources before requesting rendered pixels.
    pub(crate) fn template(&self) -> Result<frame::Video, String> {
        self.validate()?;
        let mut output = frame::Video::empty();
        output.set_width(self.width);
        output.set_height(self.height);
        output.set_format(ffmpeg::format::Pixel::BGRA);
        unsafe {
            let raw = &mut *output.as_mut_ptr();
            raw.time_base = self.time_base().into();
            raw.duration = 1;
            raw.sample_aspect_ratio = ffi::AVRational { num: 0, den: 1 };
            raw.flags |= ffi::AV_FRAME_FLAG_KEY;
            raw.pict_type = ffi::AVPictureType::AV_PICTURE_TYPE_I;
        }
        #[cfg(bsr_ffmpeg_9_0)]
        unsafe {
            (*output.as_mut_ptr()).alpha_mode = match self.alpha {
                RawVideoAlpha::Straight => ffi::AVAlphaMode::AVALPHA_MODE_STRAIGHT,
                RawVideoAlpha::Premultiplied => ffi::AVAlphaMode::AVALPHA_MODE_PREMULTIPLIED,
            };
        }
        Ok(output)
    }
}

pub enum RawVideoFrame {
    Frame(Vec<u8>),
    Finished,
}

struct State {
    queue: VecDeque<Result<RawVideoFrame, String>>,
    closed: bool,
    producer_alive: bool,
    finished_sent: bool,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    capacity: usize,
}

/// Keep this outside spawn_blocking in a Drop guard. Closing it immediately
/// wakes both sides even if that worker has not started and cannot drop a receiver.
#[derive(Clone)]
pub struct RawVideoCancelHandle(Arc<Shared>);
impl RawVideoCancelHandle {
    pub fn close(&self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.closed = true;
        state.queue.clear();
        self.0.changed.notify_all();
    }
}

pub struct RawVideoSender {
    shared: Arc<Shared>,
}
impl RawVideoSender {
    pub fn is_closed(&self) -> bool {
        let state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.closed || state.finished_sent
    }
    pub fn blocking_send(&self, message: Result<RawVideoFrame, String>) -> Result<(), String> {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        while state.queue.len() >= self.shared.capacity && !state.closed && !state.finished_sent {
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
        if state.closed || state.finished_sent {
            return Err("raw BGRA consumer closed or stream already finished".into());
        }
        if matches!(message, Ok(RawVideoFrame::Finished)) {
            state.finished_sent = true;
        }
        state.queue.push_back(message);
        self.shared.changed.notify_all();
        Ok(())
    }
}
impl Drop for RawVideoSender {
    fn drop(&mut self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.producer_alive = false;
        self.shared.changed.notify_all();
    }
}

#[derive(Clone, Debug, Default)]
pub struct RawVideoStats {
    pub frames: u64,
    pub bytes: u64,
    pub queue_wait: Duration,
    pub copy: Duration,
    pub finished_at: Option<Instant>,
}

pub struct RawVideoInput {
    reader: Option<RawVideoReader>,
}
impl RawVideoInput {
    pub fn spec(&self) -> RawVideoSpec {
        self.reader
            .as_ref()
            .expect("raw input is consumed by value")
            .spec
    }
    pub fn cancel_handle(&self) -> RawVideoCancelHandle {
        RawVideoCancelHandle(
            self.reader
                .as_ref()
                .expect("raw input is consumed by value")
                .shared
                .clone(),
        )
    }
    pub fn with_recycle(mut self, recycle: SyncSender<Vec<u8>>) -> Self {
        self.reader
            .as_mut()
            .expect("raw input is consumed by value")
            .recycle = Some(recycle);
        self
    }
    pub fn with_stats(mut self, stats: Arc<Mutex<RawVideoStats>>) -> Self {
        self.reader
            .as_mut()
            .expect("raw input is consumed by value")
            .destination = Some(stats);
        self
    }
    pub(crate) fn into_reader(mut self) -> RawVideoReader {
        self.reader.take().expect("raw input is consumed by value")
    }
}

pub fn raw_video_channel(
    spec: RawVideoSpec,
    capacity: usize,
) -> Result<(RawVideoSender, RawVideoInput), String> {
    let bytes = spec.validate()?;
    if !(1..=3).contains(&capacity) {
        return Err("raw BGRA queue capacity must be one to three frames".into());
    }
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            queue: VecDeque::new(),
            closed: false,
            producer_alive: true,
            finished_sent: false,
        }),
        changed: Condvar::new(),
        capacity,
    });
    Ok((
        RawVideoSender {
            shared: shared.clone(),
        },
        RawVideoInput {
            reader: Some(RawVideoReader {
                spec,
                bytes,
                shared,
                recycle: None,
                destination: None,
                stats: RawVideoStats::default(),
                received: 0,
                finished: false,
            }),
        },
    ))
}

pub(crate) struct RawVideoReader {
    spec: RawVideoSpec,
    bytes: usize,
    shared: Arc<Shared>,
    recycle: Option<SyncSender<Vec<u8>>>,
    destination: Option<Arc<Mutex<RawVideoStats>>>,
    stats: RawVideoStats,
    received: u64,
    finished: bool,
}
impl RawVideoReader {
    pub(crate) fn next(&mut self, cancelled: &AtomicBool) -> Result<Option<frame::Video>, String> {
        if cancelled.load(Ordering::Acquire) {
            self.close();
            return Err("raw BGRA input cancelled".into());
        }
        if self.finished {
            return Ok(None);
        }
        let started = Instant::now();
        let message = self.receive(cancelled);
        self.stats.queue_wait += started.elapsed();
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                self.close();
                return Err(error);
            }
        };
        match message {
            Ok(RawVideoFrame::Frame(bytes)) => {
                if bytes.len() != self.bytes || self.received >= self.spec.frame_count {
                    self.close();
                    return Err("invalid raw BGRA frame size or count".into());
                }
                let started = Instant::now();
                let mut output = self.spec.template()?;
                let allocated = unsafe { ffi::av_frame_get_buffer(output.as_mut_ptr(), 32) };
                if allocated < 0 {
                    self.close();
                    return Err(format!(
                        "allocate raw BGRA frame: {}",
                        ffmpeg::Error::from(allocated)
                    ));
                }
                let width = self.spec.width as usize * 4;
                let stride = output.stride(0);
                if stride < width {
                    self.close();
                    return Err("invalid native BGRA frame stride".into());
                }
                for (target, source) in output
                    .data_mut(0)
                    .chunks_exact_mut(stride)
                    .take(self.spec.height as usize)
                    .zip(bytes.chunks_exact(width))
                {
                    target[..width].copy_from_slice(source);
                    target[width..].fill(0);
                }
                output.set_pts(Some(self.received as i64));
                unsafe {
                    (*output.as_mut_ptr()).best_effort_timestamp = self.received as i64;
                }
                self.received += 1;
                self.stats.frames += 1;
                self.stats.bytes += bytes.len() as u64;
                self.stats.copy += started.elapsed();
                // The native frame owns another allocation. Filters may keep
                // arbitrary references after the renderer reuses this Vec.
                if let Some(recycle) = &self.recycle {
                    let _ = recycle.try_send(bytes);
                }
                if cancelled.load(Ordering::Acquire) {
                    self.close();
                    return Err("raw BGRA input cancelled".into());
                }
                Ok(Some(output))
            }
            Ok(RawVideoFrame::Finished) => {
                if self.received != self.spec.frame_count {
                    let error = format!(
                        "raw BGRA stream ended after {} of {} frames",
                        self.received, self.spec.frame_count
                    );
                    self.close();
                    return Err(error);
                }
                self.stats.finished_at = Some(Instant::now());
                self.close();
                Ok(None)
            }
            Err(error) => {
                self.close();
                Err(format!("raw BGRA producer failed: {error}"))
            }
        }
    }
    fn receive(&self, cancelled: &AtomicBool) -> Result<Result<RawVideoFrame, String>, String> {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err("raw BGRA input cancelled".into());
            }
            if state.closed {
                return Err("raw BGRA input closed".into());
            }
            if let Some(message) = state.queue.pop_front() {
                self.shared.changed.notify_all();
                return Ok(message);
            }
            if !state.producer_alive {
                return Err("raw BGRA producer closed without Finished".into());
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, POLL)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }
    pub(crate) fn close(&mut self) {
        self.finished = true;
        RawVideoCancelHandle(self.shared.clone()).close();
        self.recycle.take();
        if let Some(destination) = &self.destination {
            *destination
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = self.stats.clone();
        }
    }
}
impl Drop for RawVideoReader {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests;
