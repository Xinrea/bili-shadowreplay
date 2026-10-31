//! Shared Vulkan rendering/composition with platform-specific encoder frames.
//!
//! A borrowed renderer writes the RGB frame pool; a compute shader composites on the
//! same device into NV12. Vulkan Video consumes that pool directly; macOS
//! exports MoltenVK textures for a GPU copy into VideoToolbox IOSurfaces.
//! The compositor can upload a CPU-decoded NV12 base once. Rendered/composited
//! pixels remain GPU images and are never read back by this bridge.

mod compositor;
mod compute;
#[cfg(target_os = "macos")]
mod macos_runtime;
mod sync;
#[cfg(target_os = "macos")]
mod videotoolbox;
#[cfg(target_os = "macos")]
pub use videotoolbox::VideoToolboxFrameBridge;

pub use compositor::{VulkanCompositor, VulkanCompositorStats};

/// Independent of Skia/Vulkan submission mode. Blocking is retained for
/// controlled comparisons; production overlaps the encoder handoff by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EncoderFrameHandoff {
    Blocking,
    #[default]
    Asynchronous,
}

/// Platform-specific frame handoff, independent of scene drawing and MediaPlan
/// scheduling. Unsupported combinations fail before the first output frame.
pub enum HardwareEncoderFrames {
    Vulkan(frame::Video),
    #[cfg(target_os = "macos")]
    VideoToolbox(VideoToolboxFrameBridge),
}

impl HardwareEncoderFrames {
    pub fn new(compositor: &mut VulkanCompositor, encoder: &str) -> Result<Self, String> {
        Self::new_with_handoff(compositor, encoder, EncoderFrameHandoff::default())
    }

    pub fn new_with_handoff(
        compositor: &mut VulkanCompositor,
        encoder: &str,
        handoff: EncoderFrameHandoff,
    ) -> Result<Self, String> {
        // Vulkan Video consumes frames and their timeline dependencies directly.
        let _ = handoff;
        match encoder {
            "h264_vulkan" | "hevc_vulkan" => {
                crate::profiling::annotate("encoder_handoff", || "VulkanTimeline".into());
                Ok(Self::Vulkan(compositor.output_template()?))
            }
            #[cfg(target_os = "macos")]
            "h264_videotoolbox" | "hevc_videotoolbox" => {
                VideoToolboxFrameBridge::new_with_handoff(compositor, handoff)
                    .map(Self::VideoToolbox)
            }
            _ => Err(format!("No GPU frame adapter for {encoder}")),
        }
    }

    pub fn output_template(&self) -> Result<frame::Video, String> {
        match self {
            Self::Vulkan(template) => {
                let mut result = frame::Video::empty();
                unsafe {
                    check(
                        ffi::av_frame_copy_props(result.as_mut_ptr(), template.as_ptr()),
                        "Copy Vulkan template properties",
                    )?;
                    let raw = &mut *result.as_mut_ptr();
                    raw.format = ffi::AVPixelFormat::AV_PIX_FMT_VULKAN as i32;
                    raw.width = template.width() as i32;
                    raw.height = template.height() as i32;
                    raw.hw_frames_ctx = ffi::av_buffer_ref((*template.as_ptr()).hw_frames_ctx);
                    if raw.hw_frames_ctx.is_null() {
                        return Err("Retain Vulkan template pool".into());
                    }
                }
                Ok(result)
            }
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(bridge) => bridge.output_template(),
        }
    }

    /// Submit hardware frames and return only completed encoder inputs, in order.
    /// VideoToolbox retains at most three frames; Vulkan passes through directly.
    pub fn process(&mut self, frames: Vec<frame::Video>) -> Result<Vec<frame::Video>, String> {
        match self {
            Self::Vulkan(_) => Ok(frames),
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(bridge) => bridge.process(frames),
        }
    }

    /// Drain every pending handoff before sending EOF to the encoder.
    pub fn flush(&mut self) -> Result<Vec<frame::Video>, String> {
        match self {
            Self::Vulkan(_) => Ok(Vec::new()),
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(bridge) => bridge.flush(),
        }
    }
}

/// CPU completion waits are retained as a validation baseline. Asynchronous
/// mode permits up to three complete GPU frame transactions in flight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VulkanSubmissionMode {
    #[default]
    Synchronous,
    Asynchronous,
}
impl VulkanSubmissionMode {
    pub fn max_in_flight(self) -> usize {
        match self {
            Self::Synchronous => 1,
            Self::Asynchronous => 3,
        }
    }
    fn command_slots(self) -> usize {
        // One acquire and one publish barrier per in-flight rendered frame.
        match self {
            Self::Synchronous => 1,
            Self::Asynchronous => 6,
        }
    }
}

use std::{
    cell::Cell,
    collections::BTreeSet,
    ffi::{c_void, CStr, CString},
    ptr,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use ffmpeg_next::{self as ffmpeg, ffi, frame, Rational};

#[derive(Clone, Debug)]
pub struct VulkanRenderOptions {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub device_selector: String,
}

impl Default for VulkanRenderOptions {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps_num: 60,
            fps_den: 1,
            device_selector: "0".into(),
        }
    }
}

impl VulkanRenderOptions {
    pub fn validate(&self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.width > 8192
            || self.height > 8192
            || !self.width.is_multiple_of(2)
            || !self.height.is_multiple_of(2)
            || self.fps_num == 0
            || self.fps_den == 0
            || self.fps_num > i32::MAX as u32
            || self.fps_den > i32::MAX as u32
            || u64::from(self.fps_num) > 240 * u64::from(self.fps_den)
        {
            return Err("Invalid Vulkan render size or frame rate".into());
        }
        CString::new(self.device_selector.as_str()).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn rate(&self) -> Rational {
        Rational(self.fps_num as i32, self.fps_den as i32)
    }
}

/// Borrowed native handles. Keep the originating VulkanDevice clone alive for
/// the entire lifetime of a renderer constructed from these values.
#[derive(Clone, Debug)]
pub struct VulkanDeviceInfo {
    pub instance: usize,
    pub physical_device: usize,
    pub device: usize,
    pub queue: usize,
    pub queue_family: u32,
    pub instance_extensions: Vec<String>,
    pub device_extensions: Vec<String>,
    pub max_api_version: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct VulkanImageInfo {
    pub image: u64,
    pub memory: u64,
    pub memory_offset: u64,
    pub memory_size: u64,
    pub layout: u32,
    pub format: u32,
    pub usage: u32,
    pub queue_family: u32,
    pub sharing_mode: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct VulkanImageState {
    pub layout: u32,
    pub queue_family: u32,
}

struct BufferRef(*mut ffi::AVBufferRef);
impl Drop for BufferRef {
    fn drop(&mut self) {
        unsafe { ffi::av_buffer_unref(&mut self.0) }
    }
}

struct DeviceInner {
    sync: sync::VulkanSync,
    info: VulkanDeviceInfo,
    mode: VulkanSubmissionMode,
    raw: BufferRef,
}
impl DeviceInner {
    fn hw(&self) -> &ffi::AVVulkanDeviceContext {
        unsafe { &*(*self.context()).hwctx.cast::<ffi::AVVulkanDeviceContext>() }
    }
    fn context(&self) -> *mut ffi::AVHWDeviceContext {
        unsafe { (*self.raw.0).data.cast() }
    }
}
impl Drop for DeviceInner {
    fn drop(&mut self) {
        // No host mappings exist; let in-flight device work release references
        // before FFmpeg destroys the device and unloads its Vulkan library.
        let _ = self.sync.wait_idle();
    }
}

/// Intentionally !Send/!Sync: the renderer, graph and encoder run on one worker.
#[derive(Clone)]
pub struct VulkanDevice(Rc<DeviceInner>);
impl VulkanDevice {
    fn new_with_mode(selector: &str, mode: VulkanSubmissionMode) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        macos_runtime::prepare()?;
        let selector = CString::new(selector).map_err(|e| e.to_string())?;
        let mut raw = ptr::null_mut();
        let mut opts = ptr::null_mut();
        unsafe {
            ffi::av_dict_set(&mut opts, c"limit_queues".as_ptr(), c"1".as_ptr(), 0);
            // MoltenVK cannot portably render/storage-bind multi-planar YUV
            // images. Separate R8/RG8 VkImages still represent GPU NV12.
            #[cfg(target_os = "macos")]
            ffi::av_dict_set(&mut opts, c"disable_multiplane".as_ptr(), c"1".as_ptr(), 0);
            #[cfg(target_os = "macos")]
            ffi::av_dict_set(
                &mut opts,
                c"device_extensions".as_ptr(),
                c"VK_EXT_metal_objects".as_ptr(),
                0,
            );
            // GPU libraries and diagnostic capture layers can use debug-label
            // entry points on the shared device. Request the instance
            // extension so those markers are legal through every layer;
            // this does not enable validation. FFmpeg excludes unsupported
            // optional user extensions during device creation.
            ffi::av_dict_set(
                &mut opts,
                c"instance_extensions".as_ptr(),
                c"VK_EXT_debug_utils".as_ptr(),
                0,
            );
            let result = ffi::av_hwdevice_ctx_create(
                &mut raw,
                ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VULKAN,
                selector.as_ptr(),
                opts,
                0,
            );
            ffi::av_dict_free(&mut opts);
            check(result, "Create Vulkan device")?;
        }
        let raw = BufferRef(raw);
        let hw = unsafe {
            &*(*(*raw.0).data.cast::<ffi::AVHWDeviceContext>())
                .hwctx
                .cast::<ffi::AVVulkanDeviceContext>()
        };
        let family = hw.qf[..hw.nb_qf as usize]
            .iter()
            .find(|q| q.flags & 1 != 0 && q.num > 0)
            .ok_or("FFmpeg Vulkan device did not enable a graphics queue")?;
        let sync = unsafe { sync::VulkanSync::new(hw, family.idx as u32, mode.command_slots())? };
        unsafe fn extensions(pointer: *const *const std::ffi::c_char, count: i32) -> Vec<String> {
            (0..count)
                .map(|i| {
                    unsafe { CStr::from_ptr(*pointer.add(i as usize)) }
                        .to_string_lossy()
                        .into_owned()
                })
                .collect()
        }
        let info = VulkanDeviceInfo {
            instance: hw.inst as usize,
            physical_device: hw.phys_dev as usize,
            device: hw.act_dev as usize,
            queue: sync.queue(),
            queue_family: family.idx as u32,
            instance_extensions: unsafe {
                extensions(hw.enabled_inst_extensions, hw.nb_enabled_inst_extensions)
            },
            device_extensions: unsafe {
                extensions(hw.enabled_dev_extensions, hw.nb_enabled_dev_extensions)
            },
            max_api_version: (1 << 22) | (3 << 12),
        };
        Ok(Self(Rc::new(DeviceInner {
            raw,
            info,
            sync,
            mode,
        })))
    }
    pub fn info(&self) -> VulkanDeviceInfo {
        self.0.info.clone()
    }
    /// Resolve against this exact device, not a second loader/device instance.
    pub fn get_proc_addr(&self, device_scope: bool, name: &CStr) -> *const c_void {
        self.0.sync.get_proc_addr(device_scope, name)
    }
    /// Preserve the instance/device handle requested by a borrowed renderer.
    /// Global Vulkan entry points must be queried with a null instance; some
    /// dispatch layers do not expose them through a non-null instance.
    /// Foreign handles are rejected before calling the native resolver.
    pub fn get_proc_addr_for(
        &self,
        device_scope: bool,
        handle: usize,
        name: &CStr,
    ) -> *const c_void {
        let expected = if device_scope {
            self.0.info.device
        } else {
            self.0.info.instance
        };
        if handle != expected && (device_scope || handle != 0) {
            return ptr::null();
        }
        self.0.sync.get_proc_addr_for(device_scope, handle, name)
    }
}

struct FramePool {
    raw: BufferRef,
    device: VulkanDevice,
    usage: u32,
    format: u32,
    active_lease: Cell<bool>,
    queue_locked: Cell<bool>,
}
impl FramePool {
    fn new(device: VulkanDevice, options: &VulkanRenderOptions) -> Result<Rc<Self>, String> {
        Self::new_format(
            device,
            options,
            ffi::AVPixelFormat::AV_PIX_FMT_RGBA,
            0x4 | 0x8 | 0x10,
        )
    }
    fn new_format(
        device: VulkanDevice,
        options: &VulkanRenderOptions,
        format: ffi::AVPixelFormat,
        usage: u32,
    ) -> Result<Rc<Self>, String> {
        let raw = unsafe { ffi::av_hwframe_ctx_alloc(device.0.raw.0) };
        if raw.is_null() {
            return Err("Allocate Vulkan RGB frame pool".into());
        }
        let raw = BufferRef(raw);
        let ctx = unsafe { &mut *(*raw.0).data.cast::<ffi::AVHWFramesContext>() };
        ctx.format = ffi::AVPixelFormat::AV_PIX_FMT_VULKAN;
        ctx.sw_format = format;
        ctx.width = options.width as i32;
        ctx.height = options.height as i32;
        // NV12 flags are finalized after pool initialization and before its
        // first upload. Keep that pool dynamic so init cannot preallocate an
        // image with the driver's unsupported alias/profile combination.
        ctx.initial_pool_size = if format == ffi::AVPixelFormat::AV_PIX_FMT_NV12 {
            0
        } else {
            3
        };
        let vk = unsafe { &mut *ctx.hwctx.cast::<ffi::AVVulkanFramesContext>() };
        // SAMPLED | STORAGE | COLOR_ATTACHMENT. FFmpeg also supplies transfer
        // usages for its own layout transitions; this code never copies pixels.
        vk.usage = usage as _;
        unsafe {
            check(
                ffi::av_hwframe_ctx_init(raw.0),
                "Initialize Vulkan RGB frame pool",
            )?;
        }
        let vk = unsafe { &*ctx.hwctx.cast::<ffi::AVVulkanFramesContext>() };
        let usage = vk.usage as u32;
        // Software pixel-format names do not imply a Vulkan format: FFmpeg
        // can map BGRA and RGBA to the same image with different component
        // interpretation. Export the initialized pool's actual VkFormat.
        let format = vk.format[0] as u32;
        if format == 0 {
            return Err("Vulkan RGB frame pool has no image format".into());
        }
        Ok(Rc::new(Self {
            raw,
            device,
            usage,
            format,
            active_lease: Cell::new(false),
            queue_locked: Cell::new(false),
        }))
    }
    fn new_nv12(
        device: VulkanDevice,
        options: &VulkanRenderOptions,
        encoder_output: bool,
    ) -> Result<Rc<Self>, String> {
        let mut usage = 0x4 | 0x8; // SAMPLED | STORAGE (+ transfers from FFmpeg)
        if encoder_output && !cfg!(target_os = "macos") {
            usage |= 0x4000; // VIDEO_ENCODE_SRC_KHR, used by the native encoder.
        }
        let pool = Self::new_format(device, options, ffi::AVPixelFormat::AV_PIX_FMT_NV12, usage)?;
        unsafe {
            let vk = &mut *(*pool.context()).hwctx.cast::<ffi::AVVulkanFramesContext>();
            // Dynamic independent images: no aliasing. Keep FFmpeg's mutable
            // plane-view and video-profile flags; the Mac adapter may still
            // append Metal export declarations before the first allocation.
            vk.img_flags &= !0x400;
        }
        Ok(pool)
    }
    fn context(&self) -> *mut ffi::AVHWFramesContext {
        unsafe { (*self.raw.0).data.cast() }
    }
    fn lock_queue(&self) {
        let inner = &self.device.0;
        if let Some(lock) = inner.hw().lock_queue {
            unsafe { lock(inner.context(), inner.info.queue_family, 0) };
        }
        self.queue_locked.set(true);
    }
    fn unlock_queue(&self) {
        if self.queue_locked.replace(false) {
            let inner = &self.device.0;
            if let Some(unlock) = inner.hw().unlock_queue {
                unsafe { unlock(inner.context(), inner.info.queue_family, 0) };
            }
        }
    }
    fn wait_and_unlock_queue(&self) {
        if self.queue_locked.get() {
            let _ = self.device.0.sync.wait_idle();
            self.unlock_queue();
        }
    }
}

pub struct VulkanFrameLease {
    frame: frame::Video,
    pool: Rc<FramePool>,
    image: VulkanImageInfo,
    wait: Duration,
}
impl VulkanFrameLease {
    pub fn image(&self) -> VulkanImageInfo {
        self.image
    }
    fn vk_frame(&mut self) -> &mut ffi::AVVkFrame {
        unsafe { &mut *(*self.frame.as_mut_ptr()).data[0].cast::<ffi::AVVkFrame>() }
    }
    fn unlock_queue(&mut self) {
        self.pool.unlock_queue();
    }
}
impl Drop for VulkanFrameLease {
    fn drop(&mut self) {
        // A renderer error can occur after a submission. Do not recycle the
        // referenced image while its device may still be writing it. The
        // shared flag also handles a compositor dropped before this lease.
        self.pool.wait_and_unlock_queue();
        self.pool.active_lease.set(false);
    }
}

fn acquire_pool_frame(
    pool: &Rc<FramePool>,
    options: &VulkanRenderOptions,
    pts: i64,
) -> Result<VulkanFrameLease, String> {
    if pool.active_lease.get() {
        return Err("Submit or drop the active Vulkan frame lease before acquiring another".into());
    }
    pool.device.0.sync.collect_completed()?;
    let mut frame = frame::Video::empty();
    unsafe {
        check(
            ffi::av_hwframe_get_buffer(pool.raw.0, frame.as_mut_ptr(), 0),
            "Acquire Vulkan RGB frame",
        )?;
    }
    let vk = unsafe { &*(*frame.as_ptr()).data[0].cast::<ffi::AVVkFrame>() };
    if vk.img[0] == 0 || vk.img[1] != 0 {
        return Err("Expected one RGBA Vulkan image".into());
    }
    let image = VulkanImageInfo {
        image: vk.img[0],
        memory: vk.mem[0],
        memory_offset: vk.offset[0] as u64,
        memory_size: vk.size[0] as u64,
        layout: vk.layout[0] as u32,
        format: pool.format,
        usage: pool.usage,
        queue_family: vk.queue_family[0],
        sharing_mode: u32::from(vk.queue_family[0] == u32::MAX),
    };
    pool.lock_queue();
    unsafe {
        let raw = &mut *frame.as_mut_ptr();
        raw.pts = pts;
        raw.duration = 1;
        raw.time_base = options.rate().invert().into();
        raw.sample_aspect_ratio = ffi::AVRational { num: 1, den: 1 };
        raw.colorspace = ffi::AVColorSpace::AVCOL_SPC_RGB;
        raw.color_range = ffi::AVColorRange::AVCOL_RANGE_JPEG;
        raw.color_primaries = ffi::AVColorPrimaries::AVCOL_PRI_BT709;
        raw.color_trc = ffi::AVColorTransferCharacteristic::AVCOL_TRC_IEC61966_2_1;
        // The renderer composes onto an opaque background before encoding.
        raw.alpha_mode = ffi::AVAlphaMode::AVALPHA_MODE_PREMULTIPLIED;
    }
    let mut lease = VulkanFrameLease {
        frame,
        pool: pool.clone(),
        image,
        wait: Duration::ZERO,
    };
    pool.active_lease.set(true);
    let started = Instant::now();
    // A device-side semaphore wait plus a real image barrier establishes
    // visibility across FFmpeg's compute/transfer and Skia's graphics queue.
    // Asynchronous draws use this exact graphics queue, so queue order
    // carries the dependency into Skia without a CPU completion wait.
    let keep_alive = reference_video_frame(&lease.frame)?;
    let vk = lease.vk_frame();
    let next = pool.device.0.sync.transition(sync::ImageTransition {
        image: vk.img[0],
        semaphore: vk.sem[0],
        value: vk.sem_value[0],
        old_layout: vk.layout[0] as u32,
        new_layout: 2,
        src_access: vk.access[0],
        keep_alive,
    })?;
    // Skia m133 infers HOST_WRITE access from GENERAL images. Supply a
    // render-attachment layout to keep its inferred source access valid
    // for graphics stages without introducing a host access or copy.
    vk.layout[0] = 2; // VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL
    vk.access[0] = 0x80 | 0x100; // COLOR_ATTACHMENT_READ | WRITE
    vk.sem_value[0] = next;
    if pool.device.0.mode == VulkanSubmissionMode::Synchronous {
        pool.device.0.sync.wait(vk.sem[0], next)?;
    }
    lease.image.layout = 2;
    lease.wait = started.elapsed();
    Ok(lease)
}

fn complete_pool_frame(
    pool: &Rc<FramePool>,
    mut lease: VulkanFrameLease,
    state: VulkanImageState,
) -> Result<(frame::Video, Duration), String> {
    if !Rc::ptr_eq(&lease.pool, pool) {
        return Err("Foreign Vulkan frame lease".into());
    }
    if state.queue_family != lease.image.queue_family {
        return Err("Renderer changed Vulkan image ownership unexpectedly".into());
    }
    let mut gpu_wait = lease.wait;
    let fc = pool.context();
    let hw = unsafe { &*(*fc).hwctx.cast::<ffi::AVVulkanFramesContext>() };
    let keep_alive = reference_video_frame(&lease.frame)?;
    let vk = lease.vk_frame();
    if let Some(lock) = hw.lock_frame {
        unsafe { lock(fc, vk) };
    }
    vk.layout[0] = state.layout as i32;
    vk.access[0] = 0x100 | 0x10000; // COLOR_ATTACHMENT_WRITE | MEMORY_WRITE
    let signalled = pool.device.0.sync.transition(sync::ImageTransition {
        image: vk.img[0],
        semaphore: vk.sem[0],
        value: vk.sem_value[0],
        old_layout: state.layout,
        new_layout: 1,
        src_access: vk.access[0],
        keep_alive,
    });
    if let Ok(next) = signalled {
        vk.sem_value[0] = next;
        vk.layout[0] = 1;
        vk.access[0] = 0x8000 | 0x10000;
    }
    let semaphore = vk.sem[0];
    if let Some(unlock) = hw.unlock_frame {
        unsafe { unlock(fc, vk) };
    }
    let next = signalled?;
    let wait = Instant::now();
    if pool.device.0.mode == VulkanSubmissionMode::Synchronous {
        pool.device.0.sync.wait(semaphore, next)?;
        gpu_wait += wait.elapsed();
    }
    lease.unlock_queue();
    Ok((reference_video_frame(&lease.frame)?, gpu_wait))
}

/// `Video::clone` allocates a software pixel buffer and does not retain Vulkan
/// frames. Use checked AVFrame references when sharing a hardware image.
fn reference_video_frame(source: &frame::Video) -> Result<frame::Video, String> {
    let mut frame = frame::Video::empty();
    unsafe {
        check(
            ffi::av_frame_ref(frame.as_mut_ptr(), source.as_ptr()),
            "Retain Vulkan hardware frame",
        )?;
    }
    Ok(frame)
}

fn check(result: i32, context: &str) -> Result<(), String> {
    if result < 0 {
        Err(format!(
            "{context}: {} (FFmpeg code {result})",
            ffmpeg::Error::from(result)
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_hardware_frames_retain_buffers_without_allocating_software_pixels() {
        // Ref-only opaque buffers exercise AVFrame ownership without creating
        // a GPU or passing these sentinel contents to a Vulkan API.
        let mut source = frame::Video::empty();
        unsafe {
            let raw = &mut *source.as_mut_ptr();
            raw.format = ffi::AVPixelFormat::AV_PIX_FMT_VULKAN as i32;
            raw.width = 320;
            raw.height = 240;
            raw.buf[0] = ffi::av_buffer_alloc(std::mem::size_of::<ffi::AVVkFrame>());
            raw.hw_frames_ctx = ffi::av_buffer_alloc(std::mem::size_of::<ffi::AVHWFramesContext>());
            assert!(!raw.buf[0].is_null() && !raw.hw_frames_ctx.is_null());
            raw.data[0] = (*raw.buf[0]).data;
        }
        let retained = reference_video_frame(&source).unwrap();
        unsafe {
            let actual = &*retained.as_ptr();
            let original = &*source.as_ptr();
            assert_eq!(actual.data[0], original.data[0]);
            assert_eq!((*actual.hw_frames_ctx).data, (*original.hw_frames_ctx).data);
            assert_eq!(ffi::av_buffer_get_ref_count(original.buf[0]), 2);
            assert_eq!(ffi::av_buffer_get_ref_count(original.hw_frames_ctx), 2);
        }
        drop(source);
        unsafe {
            assert_eq!(ffi::av_buffer_get_ref_count((*retained.as_ptr()).buf[0]), 1);
        }
    }
    #[test]
    fn validates_contract_before_touching_a_device() {
        let good = VulkanRenderOptions::default();
        good.validate().unwrap();
        for invalid in [
            VulkanRenderOptions {
                width: 0,
                ..good.clone()
            },
            VulkanRenderOptions {
                width: 319,
                ..good.clone()
            },
            VulkanRenderOptions {
                height: 8194,
                ..good.clone()
            },
            VulkanRenderOptions {
                fps_den: 0,
                ..good.clone()
            },
            VulkanRenderOptions {
                fps_num: 241,
                ..good.clone()
            },
            VulkanRenderOptions {
                device_selector: "0\0".into(),
                ..good.clone()
            },
        ] {
            assert!(invalid.validate().is_err());
        }
    }

    #[test]
    #[ignore = "requires a Vulkan GPU and FFmpeg 9"]
    fn vulkan_gpu_frame_lease_lifecycle_releases_queue_on_abort() {
        let options = VulkanRenderOptions {
            width: 320,
            height: 240,
            fps_num: 30,
            fps_den: 1,
            ..VulkanRenderOptions::default()
        };
        let new_compositor = || {
            let cancelled = Arc::new(AtomicBool::new(false));
            let compositor = VulkanCompositor::new_with_mode(
                options.clone(),
                false,
                cancelled.clone(),
                VulkanSubmissionMode::Asynchronous,
            )
            .unwrap();
            (compositor, cancelled)
        };

        // A second safe acquire must reject immediately, before attempting to
        // lock the graphics queue already held by the outstanding lease.
        let (mut compositor, _) = new_compositor();
        let lease = compositor.acquire_frame(0).unwrap();
        let pool = lease.pool.clone();
        let error = compositor.acquire_frame(0).err().unwrap();
        assert!(error.contains("active Vulkan frame lease"), "{error}");
        drop(lease);
        assert!(!pool.active_lease.get());
        assert!(!pool.queue_locked.get());
        drop(compositor);

        // A failed flush can leave the caller owning a lease. Dropping the
        // compositor must unlock before compute teardown,
        // and the later lease Drop must not unlock the queue a second time.
        let (mut compositor, _) = new_compositor();
        let lease = compositor.acquire_frame(0).unwrap();
        let pool = lease.pool.clone();
        let error = compositor.flush().err().unwrap();
        assert!(error.contains("frame lease is active"), "{error}");
        drop(compositor);
        assert!(!pool.queue_locked.get());
        drop(lease);
        assert!(!pool.active_lease.get());

        // The same ownership order is legal when the compositor is dropped
        // directly. The surviving lease keeps its pool and device alive.
        let (mut compositor, _) = new_compositor();
        let lease = compositor.acquire_frame(0).unwrap();
        let pool = lease.pool.clone();
        drop(compositor);
        assert!(!pool.queue_locked.get());
        drop(lease);
        assert!(!pool.active_lease.get());

        let (mut compositor, cancelled) = new_compositor();
        cancelled.store(true, Ordering::Release);
        let error = compositor.acquire_frame(0).err().unwrap();
        assert!(error.contains("cancelled"), "{error}");
        drop(compositor);

        // No unrendered image may enter the compositor: cancellation is checked at
        // the very start of submit, which consumes and drops the locked lease.
        let (mut compositor, cancelled) = new_compositor();
        let lease = compositor.acquire_frame(0).unwrap();
        let pool = lease.pool.clone();
        let image = lease.image();
        cancelled.store(true, Ordering::Release);
        let error = unsafe {
            compositor.submit_frame(
                lease,
                VulkanImageState {
                    layout: image.layout,
                    queue_family: image.queue_family,
                },
                None,
            )
        }
        .err()
        .unwrap();
        assert!(error.contains("cancelled"), "{error}");
        assert_eq!(compositor.stats().submitted_frames, 0);
        assert!(!pool.active_lease.get());
        assert!(!pool.queue_locked.get());
    }
}
