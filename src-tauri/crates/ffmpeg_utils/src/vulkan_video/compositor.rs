//! A frame processor for the native media graph. It owns no encoder or output:
//! CPU-decoded NV12 is uploaded once, composed with Skia's premultiplied RGBA
//! image on the shared Vulkan device, and returned as hardware NV12 AVFrames.

use super::compute::ComputeCompositor;
use super::*;
use crate::capabilities::{gpu_sdr_transfer, SdrTransfer};
use crate::profiling as perf;
use std::collections::VecDeque;

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct VulkanCompositorStats {
    pub submitted_frames: u64,
    pub output_frames: u64,
    pub uploaded_frames: u64,
    pub srgb_frames: u64,
    pub unique_rgb_images: usize,
    pub gpu_wait: Duration,
    pub upload: Duration,
    pub composition: Duration,
    pub in_flight_peak: usize,
    pub frame_slot_reuses: u64,
    pub frame_reuse_wait: Duration,
    pub frame_drain_wait: Duration,
}

struct InFlightFrame {
    overlay: frame::Video,
    base: Option<frame::Video>,
    // The same AVVkFrame state is shared with the native encoder. Its most
    // recent timeline includes the encoder's queued read before slot reuse.
    output: frame::Video,
}

/// The media worker creates this together with its borrowed Skia context,
/// and retains both on that same worker thread. Asynchronous mode bounds the
/// retained frame transactions to three without exposing GPU pixels to the CPU.
pub struct VulkanCompositor {
    #[cfg(target_os = "macos")]
    pub(super) encoder_bridge_created: bool,
    // Release compute commands/views before the pools and shared device.
    compute: ComputeCompositor,
    output_pool: Rc<FramePool>,
    pool: Rc<FramePool>,
    base_pool: Option<Rc<FramePool>>,
    device: VulkanDevice,
    options: VulkanRenderOptions,
    cancelled: Arc<AtomicBool>,
    stats: VulkanCompositorStats,
    images: BTreeSet<u64>,
    last_pts: Option<i64>,
    flushed: bool,
    mode: VulkanSubmissionMode,
    in_flight: VecDeque<InFlightFrame>,
}

impl Drop for VulkanCompositor {
    fn drop(&mut self) {
        // A failed renderer can leave a live lease outside this object. Its
        // pool keeps the device alive, and observes the same queue-lock flag.
        self.pool.wait_and_unlock_queue();
        // This must precede dropping the compute commands and pool references, also
        // after cancellation or a renderer error following an async submit.
        let _ = self.synchronize();
        self.in_flight.clear();
    }
}

impl VulkanCompositor {
    pub fn new(
        options: VulkanRenderOptions,
        composite_video: bool,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        Self::new_with_mode(
            options,
            composite_video,
            cancelled,
            VulkanSubmissionMode::Synchronous,
        )
    }

    pub fn new_with_mode(
        options: VulkanRenderOptions,
        composite_video: bool,
        cancelled: Arc<AtomicBool>,
        mode: VulkanSubmissionMode,
    ) -> Result<Self, String> {
        options.validate()?;
        crate::metadata::initialize()?;
        if cancelled.load(Ordering::Acquire) {
            return Err("Vulkan composition cancelled".into());
        }
        let device = VulkanDevice::new_with_mode(&options.device_selector, mode)?;
        let pool = FramePool::new(device.clone(), &options)?;
        let base_pool = if composite_video {
            Some(FramePool::new_nv12(device.clone(), &options, false)?)
        } else {
            None
        };
        let output_pool = FramePool::new_nv12(device.clone(), &options, true)?;
        let compute = ComputeCompositor::new(device.clone(), mode.max_in_flight())?;
        perf::annotate("gpu_compositor", || "vulkan-compute-nv12-v2-srgb".into());
        let compositor = Self {
            #[cfg(target_os = "macos")]
            encoder_bridge_created: false,
            compute,
            output_pool,
            pool,
            base_pool,
            device,
            options,
            cancelled,
            stats: VulkanCompositorStats::default(),
            images: BTreeSet::new(),
            last_pts: None,
            flushed: false,
            mode,
            in_flight: VecDeque::new(),
        };
        validate_base(
            &compositor.output_template()?,
            &compositor.options,
            &compositor.device,
        )?;
        Ok(compositor)
    }

    pub(super) fn overlay_pool(&self) -> Rc<FramePool> {
        self.pool.clone()
    }

    pub fn submission_mode(&self) -> VulkanSubmissionMode {
        self.mode
    }

    #[cfg(target_os = "macos")]
    pub(super) fn cancellation(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }

    /// Wait for GPU work before destroying a renderer borrowing this device.
    /// This also works after cancellation and does not flush/end the stream.
    /// The owner must call it before dropping Skia's textures and GPU context;
    /// the compositor alone cannot control an external renderer's drop order.
    pub fn synchronize(&mut self) -> Result<(), String> {
        // Preserve a live lease's graphics-queue lock. It can still be
        // submitted after this wait; only terminal Drop releases that lock.
        self.device.0.sync.wait_idle()?;
        self.compute.drain()?;
        self.device.0.sync.drain()
    }

    pub fn device(&self) -> &VulkanDevice {
        &self.device
    }

    pub fn stats(&self) -> &VulkanCompositorStats {
        &self.stats
    }

    /// An owning description for opening the existing MediaPlan encoder. No
    /// pixels are allocated or filled; the public hardware-pool reference is
    /// retained until this AVFrame is dropped.
    pub fn output_template(&self) -> Result<frame::Video, String> {
        let mut frame = frame::Video::empty();
        unsafe {
            let raw = &mut *frame.as_mut_ptr();
            raw.format = ffi::AVPixelFormat::AV_PIX_FMT_VULKAN as i32;
            raw.width = self.options.width as i32;
            raw.height = self.options.height as i32;
            raw.hw_frames_ctx = ffi::av_buffer_ref(self.output_pool.raw.0);
            if raw.hw_frames_ctx.is_null() {
                return Err("Retain Vulkan compositor output pool".into());
            }
            set_output_properties(raw, self.options.rate(), 0, 1);
        }
        Ok(frame)
    }

    /// PTS uses the encoder's inverse-frame-rate time base. Gaps and a delayed
    /// first frame are preserved; the standalone PoC retains its stricter
    /// consecutive-frame contract in VulkanVideoSession.
    pub fn acquire_frame(&mut self, pts: i64) -> Result<VulkanFrameLease, String> {
        self.check_cancelled()?;
        validate_pts(self.last_pts, pts, self.flushed)?;
        if self.pool.active_lease.get() {
            return Err(
                "Submit or drop the active Vulkan frame lease before acquiring another".into(),
            );
        }
        if self.mode == VulkanSubmissionMode::Asynchronous
            && self.in_flight.len() == self.mode.max_in_flight()
        {
            self.retire_oldest(true)?;
        }
        self.compute.collect_completed()?;
        let mut lease = acquire_pool_frame(&self.pool, &self.options, pts)?;
        if self.base_pool.is_some() {
            // Skia's unmanaged premultiplied RGB values use the same SDR
            // nonlinear blend convention as the production CPU overlay.
            unsafe {
                (*lease.frame.as_mut_ptr()).color_trc =
                    ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
            }
        }
        self.images.insert(lease.image.image);
        self.stats.unique_rgb_images = self.images.len();
        Ok(lease)
    }

    /// # Safety
    /// The renderer must have submitted writes to this lease on the shared
    /// graphics queue, report the submitted final image state, and retain no
    /// borrowed image. Synchronous mode also accepts a completed submission.
    /// With a video base,
    /// render onto transparent premultiplied RGBA, not an opaque background.
    pub unsafe fn submit_frame(
        &mut self,
        mut lease: VulkanFrameLease,
        state: VulkanImageState,
        base: Option<&frame::Video>,
    ) -> Result<Vec<frame::Video>, String> {
        self.check_cancelled()?;
        if self.base_pool.is_some() != base.is_some() {
            return Err("Vulkan composition base does not match its input mode".into());
        }
        let pts = lease.frame.pts().ok_or("Vulkan overlay PTS missing")?;
        validate_pts(self.last_pts, pts, self.flushed)?;
        let transfer = if let Some(base) = base {
            let transfer = validate_base(base, &self.options, &self.device)?;
            unsafe { (*lease.frame.as_mut_ptr()).duration = (*base.as_ptr()).duration };
            transfer
        } else {
            SdrTransfer::Bt709
        };
        let duration = unsafe { (*lease.frame.as_ptr()).duration };
        let started = Instant::now();
        // Complete/unlock Skia's graphics submission before asking FFmpeg to
        // upload on the shared device; transfer_data takes queue locks itself.
        let (overlay, wait) = perf::measure("vulkan_publish", || {
            complete_pool_frame(&self.pool, lease, state)
        })?;
        self.stats.gpu_wait += wait;
        let mut retained_base = None;
        if let Some(base) = base {
            let upload = Instant::now();
            let upload_span = perf::span("nv12_upload");
            let mut video = if base.format() == ffmpeg::format::Pixel::VULKAN {
                reference_video_frame(base)?
            } else {
                let mut video = frame::Video::empty();
                unsafe {
                    check(
                        ffi::av_hwframe_get_buffer(
                            self.base_pool.as_ref().unwrap().raw.0,
                            video.as_mut_ptr(),
                            0,
                        ),
                        "Acquire Vulkan video-base frame",
                    )?;
                    check(
                        ffi::av_hwframe_transfer_data(video.as_mut_ptr(), base.as_ptr(), 0),
                        "Upload NV12 video base to Vulkan",
                    )?;
                }
                self.stats.uploaded_frames += 1;
                video
            };
            unsafe {
                set_output_properties(&mut *video.as_mut_ptr(), self.options.rate(), pts, duration);
                // Upload does not copy AVFrame properties. Keep the real input
                // transfer; only the compute output is normalized to BT.709.
                (*video.as_mut_ptr()).color_trc = (*base.as_ptr()).color_trc;
            }
            self.stats.upload += upload.elapsed();
            drop(upload_span);
            retained_base = Some(video);
        }
        let mut output = frame::Video::empty();
        unsafe {
            check(
                ffi::av_hwframe_get_buffer(self.output_pool.raw.0, output.as_mut_ptr(), 0),
                "Acquire compute NV12 output",
            )?;
            set_output_properties(
                &mut *output.as_mut_ptr(),
                self.options.rate(),
                pts,
                duration,
            );
        }
        self.compute
            .submit(&overlay, retained_base.as_ref(), &output, transfer)?;
        self.stats.srgb_frames += u64::from(transfer == SdrTransfer::Srgb);
        if self.mode == VulkanSubmissionMode::Asynchronous {
            self.in_flight.push_back(InFlightFrame {
                overlay,
                base: retained_base,
                output: reference_video_frame(&output)?,
            });
            self.stats.in_flight_peak = self.stats.in_flight_peak.max(self.in_flight.len());
        } else {
            self.wait_frame_uses(&output)?;
        }
        self.last_pts = Some(pts);
        self.stats.submitted_frames += 1;
        self.stats.output_frames += 1;
        self.stats.composition += started.elapsed();
        self.check_cancelled()?;
        Ok(vec![output])
    }

    pub fn flush(&mut self) -> Result<Vec<frame::Video>, String> {
        self.check_cancelled()?;
        if self.pool.active_lease.get() {
            return Err("Cannot flush while a Vulkan frame lease is active".into());
        }
        self.flushed = true;
        while !self.in_flight.is_empty() {
            self.retire_oldest(false)?;
        }
        self.compute.drain()?;
        self.device.0.sync.drain()?;
        Ok(Vec::new())
    }

    fn retire_oldest(&mut self, reusing_slot: bool) -> Result<(), String> {
        let _span = perf::span("frame_slot_wait");
        let pending = self
            .in_flight
            .front()
            .ok_or("No Vulkan frame slot to retire")?;
        let output = &pending.output;
        let started = Instant::now();
        perf::measure("slot_overlay_wait", || {
            self.wait_frame_uses(&pending.overlay)
        })?;
        if let Some(base) = &pending.base {
            perf::measure("slot_base_wait", || self.wait_frame_uses(base))?;
        }
        perf::measure("slot_output_wait", || self.wait_frame_uses(output))?;
        self.device.0.sync.collect_completed()?;
        self.in_flight.pop_front();
        if reusing_slot {
            self.stats.frame_slot_reuses += 1;
            self.stats.frame_reuse_wait += started.elapsed();
        } else {
            self.stats.frame_drain_wait += started.elapsed();
        }
        Ok(())
    }

    fn wait_frame_uses(&self, frame: &frame::Video) -> Result<(), String> {
        let raw = unsafe { &*frame.as_ptr() };
        if raw.hw_frames_ctx.is_null() {
            return Err("In-flight Vulkan frame lost its hardware pool".into());
        }
        let pool = unsafe { (*raw.hw_frames_ctx).data.cast::<ffi::AVHWFramesContext>() };
        let hw = unsafe { &*(*pool).hwctx.cast::<ffi::AVVulkanFramesContext>() };
        let vk = raw.data[0].cast::<ffi::AVVkFrame>();
        if vk.is_null() {
            return Err("In-flight Vulkan frame lost its image state".into());
        }
        if let Some(lock) = hw.lock_frame {
            unsafe { lock(pool, vk) };
        }
        // Capture the latest value, including compute/encoder consumers,
        // without retaining the frame lock across a host wait.
        let signals: Vec<_> = unsafe {
            (*vk)
                .img
                .iter()
                .enumerate()
                .filter(|(_, image)| **image != 0)
                .map(|(index, _)| ((*vk).sem[index], (*vk).sem_value[index]))
                .collect()
        };
        if let Some(unlock) = hw.unlock_frame {
            unsafe { unlock(pool, vk) };
        }
        for (semaphore, value) in signals {
            if semaphore == 0 {
                return Err("In-flight Vulkan frame has no timeline semaphore".into());
            }
            self.device.0.sync.wait(semaphore, value)?;
        }
        Ok(())
    }

    fn check_cancelled(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::Acquire) {
            Err("Vulkan composition cancelled".into())
        } else {
            Ok(())
        }
    }
}

fn validate_pts(previous: Option<i64>, pts: i64, flushed: bool) -> Result<(), String> {
    if flushed {
        return Err("Vulkan compositor has already been flushed".into());
    }
    if pts == ffi::AV_NOPTS_VALUE || previous.is_some_and(|previous| pts <= previous) {
        return Err("Vulkan composition requires increasing frame PTS".into());
    }
    Ok(())
}

fn validate_base(
    frame: &frame::Video,
    options: &VulkanRenderOptions,
    device: &VulkanDevice,
) -> Result<SdrTransfer, String> {
    if frame.width() != options.width || frame.height() != options.height {
        return Err("Vulkan video base must match the compositor dimensions".into());
    }
    let raw = unsafe { &*frame.as_ptr() };
    if !matches!(
        raw.colorspace,
        ffi::AVColorSpace::AVCOL_SPC_UNSPECIFIED | ffi::AVColorSpace::AVCOL_SPC_BT709
    ) || !matches!(
        raw.color_range,
        ffi::AVColorRange::AVCOL_RANGE_UNSPECIFIED | ffi::AVColorRange::AVCOL_RANGE_MPEG
    ) {
        return Err("Vulkan video base must use the BT.709 matrix and limited-range NV12".into());
    }
    let transfer = gpu_sdr_transfer(raw.color_primaries, raw.color_trc)
        .ok_or("Vulkan video base requires BT.709 primaries and BT.709 or sRGB transfer")?;
    match frame.format() {
        ffmpeg::format::Pixel::NV12 => Ok(transfer),
        ffmpeg::format::Pixel::VULKAN => {
            if raw.hw_frames_ctx.is_null() {
                return Err("Vulkan video base has no hardware pool".into());
            }
            let pool = unsafe { &*(*raw.hw_frames_ctx).data.cast::<ffi::AVHWFramesContext>() };
            if pool.sw_format != ffi::AVPixelFormat::AV_PIX_FMT_NV12 || pool.device_ctx.is_null() {
                return Err("Vulkan video base must use an NV12 hardware pool".into());
            }
            let hw = unsafe {
                &*(*pool.device_ctx)
                    .hwctx
                    .cast::<ffi::AVVulkanDeviceContext>()
            };
            if hw.act_dev != device.0.hw().act_dev {
                return Err("Vulkan video base belongs to another device".into());
            }
            Ok(transfer)
        }
        _ => Err("Vulkan video base must be NV12 or shared-device Vulkan NV12".into()),
    }
}

unsafe fn set_output_properties(raw: &mut ffi::AVFrame, rate: Rational, pts: i64, duration: i64) {
    raw.pts = pts;
    raw.duration = duration;
    raw.time_base = rate.invert().into();
    raw.sample_aspect_ratio = ffi::AVRational { num: 1, den: 1 };
    raw.colorspace = ffi::AVColorSpace::AVCOL_SPC_BT709;
    raw.color_range = ffi::AVColorRange::AVCOL_RANGE_MPEG;
    raw.color_primaries = ffi::AVColorPrimaries::AVCOL_PRI_BT709;
    raw.color_trc = ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
    raw.alpha_mode = ffi::AVAlphaMode::AVALPHA_MODE_UNSPECIFIED;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_preserves_delayed_and_variable_frame_pts() {
        validate_pts(None, 120, false).unwrap();
        validate_pts(Some(120), 124, false).unwrap();
        assert!(validate_pts(Some(120), 120, false).is_err());
        assert!(validate_pts(Some(120), 119, false).is_err());
        assert!(validate_pts(None, ffi::AV_NOPTS_VALUE, false).is_err());
        assert!(validate_pts(None, 0, true).is_err());
    }
}
