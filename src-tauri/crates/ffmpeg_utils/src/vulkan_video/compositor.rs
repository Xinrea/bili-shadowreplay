//! A frame processor for the native media graph. It owns no encoder or output:
//! CPU-decoded NV12 is uploaded once, composed with Skia's premultiplied RGBA
//! image on the shared Vulkan device, and returned as hardware NV12 AVFrames.

use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct VulkanCompositorStats {
    pub submitted_frames: u64,
    pub output_frames: u64,
    pub uploaded_frames: u64,
    pub unique_rgb_images: usize,
    pub gpu_wait: Duration,
    pub upload: Duration,
    pub composition: Duration,
}

/// A synchronous first implementation. The media worker creates this together
/// with its borrowed Skia context, and retains both on that same worker thread.
pub struct VulkanCompositor {
    // The graph must release its frame references before the pools/device.
    graph: filter::Graph,
    output_pool: BufferRef,
    pool: Rc<FramePool>,
    base_pool: Option<Rc<FramePool>>,
    device: VulkanDevice,
    options: VulkanVideoOptions,
    cancelled: Arc<AtomicBool>,
    stats: VulkanCompositorStats,
    images: BTreeSet<u64>,
    last_pts: Option<i64>,
    timings: BTreeMap<i64, i64>,
    flushed: bool,
}

impl Drop for VulkanCompositor {
    fn drop(&mut self) {
        // A failed renderer can leave a live lease outside this object. Its
        // pool keeps the device alive, and observes the same queue-lock flag.
        self.pool.wait_and_unlock_queue();
    }
}

impl VulkanCompositor {
    pub fn new(
        options: VulkanVideoOptions,
        composite_video: bool,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        options.validate()?;
        crate::metadata::initialize()?;
        if cancelled.load(Ordering::Acquire) {
            return Err("Vulkan composition cancelled".into());
        }
        let device = VulkanDevice::new(&options.device_selector)?;
        let pool = FramePool::new(device.clone(), &options)?;
        let base_pool = if composite_video {
            let pool = FramePool::new_format(
                device.clone(),
                &options,
                ffi::AVPixelFormat::AV_PIX_FMT_NV12,
                0x4 | 0x8,
            )?;
            // No pool image aliases memory. Set this before the first upload,
            // just as for the encoder's NV12 output pool.
            unsafe {
                (*(*pool.context()).hwctx.cast::<ffi::AVVulkanFramesContext>()).img_flags &= !0x400;
            }
            Some(pool)
        } else {
            None
        };
        let mut graph = build_graph(&device, &pool, base_pool.as_deref(), &options)?;
        prepare_nv12_encode_pool(&mut graph)?;
        let output_pool = unsafe {
            let mut sink = graph.get("gpu_out").ok_or("Vulkan sink missing")?;
            let frames = ffi::av_buffersink_get_hw_frames_ctx(sink.as_mut_ptr());
            if frames.is_null() {
                return Err("Vulkan compositor has no output hardware pool".into());
            }
            let retained = ffi::av_buffer_ref(frames);
            if retained.is_null() {
                return Err("Retain Vulkan compositor output pool".into());
            }
            BufferRef(retained)
        };
        let compositor = Self {
            graph,
            output_pool,
            pool,
            base_pool,
            device,
            options,
            cancelled,
            stats: VulkanCompositorStats::default(),
            images: BTreeSet::new(),
            last_pts: None,
            timings: BTreeMap::new(),
            flushed: false,
        };
        validate_base(
            &compositor.output_template()?,
            &compositor.options,
            &compositor.device,
        )?;
        Ok(compositor)
    }

    pub fn device(&self) -> &VulkanDevice {
        &self.device
    }

    pub fn stats(&self) -> &VulkanCompositorStats {
        &self.stats
    }

    pub fn output_sink(&mut self) -> Result<filter::Context, String> {
        self.graph
            .get("gpu_out")
            .ok_or("Vulkan sink missing".into())
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
            raw.hw_frames_ctx = ffi::av_buffer_ref(self.output_pool.0);
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
    /// The renderer must have finished writing this lease and report its real
    /// image state. It must not retain the borrowed image. With a video base,
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
        if let Some(base) = base {
            validate_base(base, &self.options, &self.device)?;
            unsafe { (*lease.frame.as_mut_ptr()).duration = (*base.as_ptr()).duration };
        }
        let duration = unsafe { (*lease.frame.as_ptr()).duration };
        let started = Instant::now();
        // Complete/unlock Skia's graphics submission before asking FFmpeg to
        // upload on the shared device; transfer_data takes queue locks itself.
        let (overlay, wait) = complete_pool_frame(&self.pool, lease, state)?;
        self.stats.gpu_wait += wait;
        if let Some(base) = base {
            let upload = Instant::now();
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
            }
            self.stats.upload += upload.elapsed();
            self.feed("video_in", &video)?;
        }
        self.feed("gpu_in", &overlay)?;
        self.last_pts = Some(pts);
        self.timings.insert(pts, duration);
        self.stats.submitted_frames += 1;
        let frames = self.drain()?;
        self.stats.composition += started.elapsed();
        self.check_cancelled()?;
        Ok(frames)
    }

    pub fn flush(&mut self) -> Result<Vec<frame::Video>, String> {
        self.check_cancelled()?;
        if self.pool.active_lease.get() {
            return Err("Cannot flush while a Vulkan frame lease is active".into());
        }
        if !self.flushed {
            if self.base_pool.is_some() {
                self.graph
                    .get("video_in")
                    .ok_or("Vulkan video source missing")?
                    .source()
                    .flush()
                    .map_err(|e| e.to_string())?;
            }
            self.graph
                .get("gpu_in")
                .ok_or("Vulkan overlay source missing")?
                .source()
                .flush()
                .map_err(|e| e.to_string())?;
            self.flushed = true;
        }
        let frames = self.drain()?;
        if !self.timings.is_empty() {
            return Err("Vulkan compositor did not drain every submitted video frame".into());
        }
        Ok(frames)
    }

    fn check_cancelled(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::Acquire) {
            Err("Vulkan composition cancelled".into())
        } else {
            Ok(())
        }
    }

    fn feed(&mut self, name: &str, frame: &frame::Video) -> Result<(), String> {
        let mut source = self.graph.get(name).ok_or("Vulkan source missing")?;
        let result = unsafe {
            ffi::av_buffersrc_add_frame_flags(
                source.as_mut_ptr(),
                frame.as_ptr() as *mut _,
                ffi::AV_BUFFERSRC_FLAG_KEEP_REF as i32,
            )
        };
        if result < 0 {
            let raw = unsafe { &*frame.as_ptr() };
            Err(format!(
                "Feed Vulkan compositor {name} frame: {} (FFmpeg code {result}, buf0={}, hw_pool={})",
                ffmpeg::Error::from(result),
                !raw.buf[0].is_null(),
                !raw.hw_frames_ctx.is_null(),
            ))
        } else {
            Ok(())
        }
    }

    fn drain(&mut self) -> Result<Vec<frame::Video>, String> {
        let mut frames = Vec::new();
        loop {
            let mut frame = frame::Video::empty();
            match self.output_sink()?.sink().frame(&mut frame) {
                Ok(()) => {
                    if frame.format() != ffmpeg::format::Pixel::VULKAN {
                        return Err("Vulkan compositor unexpectedly downloaded a CPU frame".into());
                    }
                    validate_base(&frame, &self.options, &self.device)?;
                    let base = self.output_sink()?.sink().time_base();
                    let pts = frame.pts().ok_or("Vulkan compositor output PTS missing")?;
                    let pts = unsafe {
                        ffi::av_rescale_q(pts, base.into(), self.options.rate().invert().into())
                    };
                    let duration = self
                        .timings
                        .remove(&pts)
                        .ok_or("Vulkan compositor changed or duplicated a video timestamp")?;
                    unsafe {
                        set_output_properties(
                            &mut *frame.as_mut_ptr(),
                            self.options.rate(),
                            pts,
                            duration,
                        );
                    }
                    self.stats.output_frames += 1;
                    frames.push(frame);
                }
                Err(ffmpeg::Error::Eof) => return Ok(frames),
                Err(ffmpeg::Error::Other { errno }) if errno == ffi::EAGAIN => return Ok(frames),
                Err(error) => return Err(format!("Read Vulkan compositor output: {error}")),
            }
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
    options: &VulkanVideoOptions,
    device: &VulkanDevice,
) -> Result<(), String> {
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
    ) || !matches!(
        raw.color_primaries,
        ffi::AVColorPrimaries::AVCOL_PRI_UNSPECIFIED | ffi::AVColorPrimaries::AVCOL_PRI_BT709
    ) || !matches!(
        raw.color_trc,
        ffi::AVColorTransferCharacteristic::AVCOL_TRC_UNSPECIFIED
            | ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709
    ) {
        return Err("Vulkan video base must be SDR BT.709 limited-range NV12".into());
    }
    match frame.format() {
        ffmpeg::format::Pixel::NV12 => Ok(()),
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
            Ok(())
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
