//! MoltenVK GPU NV12 -> IOSurface -> FFmpeg VideoToolbox hardware frame.
//!
//! The Vulkan image and destination CVPixelBuffer stay owned throughout a GPU
//! blit. Completion waits do not map/download pixels. Encoding and muxing stay
//! in MediaPlan; this adapter owns neither a codec nor an output file.

use crate::profiling as perf;
use super::*;
use std::collections::VecDeque;

// VK_EXT_metal_objects ABI, deliberately independent of a Vulkan SDK header
// installation. Metal object pointers are borrowed from the live VkImage.
#[repr(C)]
struct ExportCreateInfo {
    s_type: i32,
    next: *const c_void,
    object_type: u32,
}
#[repr(C)]
struct ExportObjectsInfo {
    s_type: i32,
    next: *const c_void,
}
#[repr(C)]
struct ExportTextureInfo {
    s_type: i32,
    next: *const c_void,
    image: u64,
    image_view: u64,
    buffer_view: u64,
    plane: u32,
    texture: *mut c_void,
}
type ExportObjects = unsafe extern "system" fn(ffi::VkDevice, *mut ExportObjectsInfo);

unsafe extern "C" {
    fn bsr_metal_transfer_create(
        source: *mut c_void,
        error: *mut std::ffi::c_char,
        capacity: usize,
    ) -> *mut c_void;
    fn bsr_metal_transfer_destroy(context: *mut c_void);
    fn bsr_metal_transfer_submit(
        context: *mut c_void, luma: *mut c_void, chroma: *mut c_void,
        pixel_buffer: *mut c_void, error: *mut std::ffi::c_char,
        capacity: usize, profile: i32,
    ) -> *mut c_void;
    fn bsr_metal_copy_finish(
        ticket: *mut c_void, wait: i32, error: *mut std::ffi::c_char,
        capacity: usize, timings: *mut f64,
    ) -> i32;
    fn bsr_metal_copy_destroy(ticket: *mut c_void);
}

// A ticket is dropped before either AVFrame: its destructor waits for the
// GPU copy even on cancellation/error. Source references prevent the Vulkan
// output pool from recycling an image while Metal still reads it.
struct MetalCopy {
    ticket: *mut c_void,
    measuring: bool,
    completed: bool,
}
impl MetalCopy {
    fn finish(&mut self, wait: bool) -> Result<bool, String> {
        if self.completed { return Ok(true); }
        let mut error = [0 as std::ffi::c_char; 1024];
        let mut timings = [0.0; 6];
        let status = unsafe { bsr_metal_copy_finish(
            self.ticket, i32::from(wait), error.as_mut_ptr(), error.len(),
            if self.measuring { timings.as_mut_ptr() } else { ptr::null_mut() },
        ) };
        if status == 1 { return Ok(false); }
        if self.measuring {
            for (name, seconds) in ["metal_prepare_cpu", "metal_texture_views_cpu",
                "metal_commit_cpu", "metal_wait_cpu", "metal_cleanup_cpu", "metal_blit_gpu"]
                .into_iter().zip(timings) {
                if seconds.is_finite() && seconds > 0.0 {
                    perf::observe(name, Duration::from_secs_f64(seconds));
                }
            }
        }
        if status != 0 {
            return Err(unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy().into_owned());
        }
        self.completed = true;
        Ok(true)
    }
}
impl Drop for MetalCopy {
    fn drop(&mut self) { unsafe { bsr_metal_copy_destroy(self.ticket) } }
}

struct PendingFrame {
    copy: Option<MetalCopy>,
    output: Option<frame::Video>,
    source: frame::Video,
    device: VulkanDevice,
}
impl Drop for PendingFrame {
    fn drop(&mut self) {
        // Without a Metal ticket, Vulkan can still be producing this output.
        // Cleanup cannot obey cancellation until its borrowed resources are safe.
        if self.copy.is_none() {
            if let Err(error) = source_ready(&self.device, &self.source, true) {
                eprintln!("Vulkan handoff cleanup failed: {error}");
            }
        }
    }
}

fn source_ready(device: &VulkanDevice, source: &frame::Video, wait: bool) -> Result<bool, String> {
    let _span = perf::span(if wait { "vt_vulkan_wait" } else { "vt_vulkan_poll" });
    let vk = unsafe { &*(*source.as_ptr()).data[0].cast::<ffi::AVVkFrame>() };
    for plane in 0..2 {
        if wait {
            device.0.sync.wait(vk.sem[plane], vk.sem_value[plane])?;
        } else if !device.0.sync.is_ready(vk.sem[plane], vk.sem_value[plane])? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Created before the compositor allocates its first output frame. The
/// adapter and compositor must stay on their originating media-worker thread.
pub struct VideoToolboxFrameBridge {
    source: frame::Video,
    pool: BufferRef,
    device: VulkanDevice,
    export: ExportObjects,
    declaration: Box<ExportCreateInfo>,
    transfer: *mut c_void,
    pending: VecDeque<PendingFrame>,
    handoff: EncoderFrameHandoff,
    cancelled: Arc<AtomicBool>,
    flushed: bool,
    submitted: u64,
    completed: u64,
    pending_peak: usize,
}

impl VideoToolboxFrameBridge {
    pub fn new(compositor: &mut VulkanCompositor) -> Result<Self, String> {
        Self::new_with_handoff(compositor, EncoderFrameHandoff::default())
    }

    pub fn new_with_handoff(compositor: &mut VulkanCompositor, handoff: EncoderFrameHandoff) -> Result<Self, String> {
        if compositor.stats().submitted_frames != 0 || compositor.encoder_bridge_created {
            return Err("Configure VideoToolbox before submitting Vulkan frames".into());
        }
        let device = compositor.device().clone();
        if !device
            .0
            .info
            .device_extensions
            .iter()
            .any(|name| name == "VK_EXT_metal_objects")
        {
            return Err(
                "MoltenVK VK_EXT_metal_objects is required for VideoToolbox GPU export".into(),
            );
        }
        let address = device.get_proc_addr(true, c"vkExportMetalObjectsEXT");
        if address.is_null() {
            return Err("MoltenVK does not expose vkExportMetalObjectsEXT".into());
        }
        let source = compositor.output_template()?;
        unsafe {
            let mut vt = ptr::null_mut();
            check(
                ffi::av_hwdevice_ctx_create(
                    &mut vt,
                    ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
                    ptr::null(),
                    ptr::null_mut(),
                    0,
                ),
                "Create VideoToolbox device",
            )?;
            let vt = BufferRef(vt);
            let pool = ffi::av_hwframe_ctx_alloc(vt.0);
            if pool.is_null() {
                return Err("Allocate VideoToolbox frame pool".into());
            }
            let pool = BufferRef(pool);
            let context = &mut *(*pool.0).data.cast::<ffi::AVHWFramesContext>();
            context.format = ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX;
            context.sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
            context.width = source.width() as i32;
            context.height = source.height() as i32;
            check(
                ffi::av_hwframe_ctx_init(pool.0),
                "Initialize VideoToolbox frame pool",
            )?;
            let source_pool = &mut *(*(*source.as_ptr()).hw_frames_ctx)
                .data
                .cast::<ffi::AVHWFramesContext>();
            let vk = &mut *source_pool.hwctx.cast::<ffi::AVVulkanFramesContext>();
            let declaration = Box::new(ExportCreateInfo {
                s_type: 1_000_311_000,
                next: vk.create_pnext,
                object_type: 8, // VK_EXPORT_METAL_OBJECT_TYPE_METAL_TEXTURE_BIT_EXT
            });
            vk.create_pnext = (&*declaration as *const ExportCreateInfo).cast_mut().cast();
            compositor.encoder_bridge_created = true;
            Ok(Self {
                source,
                pool,
                device,
                export: std::mem::transmute::<*const c_void, ExportObjects>(address),
                declaration,
                transfer: ptr::null_mut(),
                pending: VecDeque::new(), handoff, cancelled: compositor.cancellation(),
                flushed: false, submitted: 0, completed: 0, pending_peak: 0,
            })
        }
    }

    pub fn output_template(&self) -> Result<frame::Video, String> {
        let mut result = frame::Video::empty();
        unsafe {
            check(
                ffi::av_frame_copy_props(result.as_mut_ptr(), self.source.as_ptr()),
                "Copy GPU frame properties",
            )?;
            let raw = &mut *result.as_mut_ptr();
            raw.format = ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX as i32;
            raw.width = self.source.width() as i32;
            raw.height = self.source.height() as i32;
            raw.hw_frames_ctx = ffi::av_buffer_ref(self.pool.0);
            if raw.hw_frames_ctx.is_null() {
                return Err("Retain VideoToolbox frame pool".into());
            }
        }
        Ok(result)
    }

    fn validate_source(&self, source: &frame::Video) -> Result<(), String> {
        unsafe {
            let raw = &*source.as_ptr();
            if raw.format != ffi::AVPixelFormat::AV_PIX_FMT_VULKAN as i32
                || raw.hw_frames_ctx.is_null()
                || (*raw.hw_frames_ctx).data != (*(*self.source.as_ptr()).hw_frames_ctx).data
                || raw.data[0].is_null()
            {
                return Err("VideoToolbox bridge received a foreign GPU frame".into());
            }
            let vk = &*raw.data[0].cast::<ffi::AVVkFrame>();
            if (0..2).any(|plane| vk.img[plane] == 0 || vk.sem[plane] == 0) {
                return Err("MoltenVK NV12 requires separate R8/RG8 images and timelines".into());
            }
        }
        Ok(())
    }

    /// At most three source/destination pairs remain pending after submission.
    /// The renderer and hardware encoder keep their original owning thread.
    pub fn process(&mut self, frames: Vec<frame::Video>) -> Result<Vec<frame::Video>, String> {
        self.check_cancelled()?;
        if self.flushed { return Err("VideoToolbox handoff already flushed".into()); }
        let mut ready = self.advance(false)?;
        for source in frames {
            self.validate_source(&source)?;
            if self.pending.len() == 3 {
                ready.extend(self.advance(true)?);
            }
            self.pending.push_back(PendingFrame {
                source, output: None, copy: None, device: self.device.clone(),
            });
            self.submitted += 1;
            self.pending_peak = self.pending_peak.max(self.pending.len());
            ready.extend(self.advance(self.handoff == EncoderFrameHandoff::Blocking)?);
        }
        Ok(ready)
    }

    pub fn flush(&mut self) -> Result<Vec<frame::Video>, String> {
        self.check_cancelled()?;
        self.flushed = true;
        let mut ready = Vec::new();
        while !self.pending.is_empty() {
            ready.extend(self.advance(true)?);
        }
        Ok(ready)
    }

    fn check_cancelled(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::Acquire) { Err("VideoToolbox handoff cancelled".into()) }
        else { Ok(()) }
    }

    fn advance(&mut self, wait_front: bool) -> Result<Vec<frame::Video>, String> {
        let mut ready = Vec::new();
        let mut blocked = false;
        for index in 0..self.pending.len() {
            self.check_cancelled()?;
            let mut pending = self.pending.pop_front().unwrap();
            let complete = self.progress(&mut pending, wait_front && index == 0)?;
            // Later copies may be submitted while the head is pending, but
            // completed outputs are always delivered in submission order.
            if complete && !blocked {
                ready.push(pending.output.take().ok_or("Completed handoff has no output")?);
                self.completed += 1;
            } else {
                blocked = true;
                self.pending.push_back(pending);
            }
        }
        Ok(ready)
    }

    fn progress(&mut self, pending: &mut PendingFrame, wait: bool) -> Result<bool, String> {
        if pending.copy.is_none() {
            if !source_ready(&self.device, &pending.source, wait)? { return Ok(false); }
            self.start_copy(pending)?;
        }
        perf::measure(if wait { "vt_metal_wait" } else { "vt_metal_poll" }, || {
            pending.copy.as_mut().unwrap().finish(wait)
        })
    }

    fn start_copy(&mut self, pending: &mut PendingFrame) -> Result<(), String> {
        unsafe {
            let source = &pending.source;
            let vk = &*(*source.as_ptr()).data[0].cast::<ffi::AVVkFrame>();
            let export = perf::span("vt_export_textures");
            let mut chroma = ExportTextureInfo {
                s_type: 1_000_311_006,
                next: ptr::null(),
                image: vk.img[1],
                image_view: 0,
                buffer_view: 0,
                // VK_EXT_metal_objects requires PLANE_0 even for an R8/RG8
                // single-plane image; COLOR is not a valid export selector.
                plane: 0x10,
                texture: ptr::null_mut(),
            };
            let mut luma = ExportTextureInfo {
                image: vk.img[0],
                next: (&mut chroma as *mut ExportTextureInfo).cast(),
                ..chroma
            };
            let mut objects = ExportObjectsInfo {
                s_type: 1_000_311_001,
                next: (&mut luma as *mut ExportTextureInfo).cast(),
            };
            (self.export)(self.device.0.hw().act_dev, &mut objects);
            if luma.texture.is_null() || chroma.texture.is_null() {
                return Err("MoltenVK could not export both NV12 Metal planes".into());
            }
            drop(export);
            let allocate = perf::span("vt_allocate_iosurface");
            let mut error = [0 as std::ffi::c_char; 1024];
            if self.transfer.is_null() {
                self.transfer =
                    bsr_metal_transfer_create(luma.texture, error.as_mut_ptr(), error.len());
                if self.transfer.is_null() {
                    return Err(CStr::from_ptr(error.as_ptr())
                        .to_string_lossy()
                        .into_owned());
                }
            }
            let mut result = frame::Video::empty();
            check(
                ffi::av_hwframe_get_buffer(self.pool.0, result.as_mut_ptr(), 0),
                "Allocate IOSurface frame",
            )?;
            check(
                ffi::av_frame_copy_props(result.as_mut_ptr(), source.as_ptr()),
                "Copy GPU frame properties",
            )?;
            let pixel_buffer: *mut c_void = (*result.as_ptr()).data[3].cast();
            if pixel_buffer.is_null() {
                return Err("VideoToolbox frame has no CVPixelBuffer".into());
            }
            drop(allocate);
            let _copy = perf::span("vt_metal_submit");
            let ticket = bsr_metal_transfer_submit(
                self.transfer, luma.texture, chroma.texture, pixel_buffer,
                error.as_mut_ptr(), error.len(), i32::from(perf::enabled()),
            );
            if ticket.is_null() {
                return Err(CStr::from_ptr(error.as_ptr()).to_string_lossy().into_owned());
            }
            pending.output = Some(result);
            pending.copy = Some(MetalCopy { ticket, measuring: perf::enabled(), completed: false });
            Ok(())
        }
    }
}

impl Drop for VideoToolboxFrameBridge {
    fn drop(&mut self) {
        perf::annotate("encoder_handoff", || format!("{:?}", self.handoff));
        perf::annotate("vt_bridge_counters", || format!(
            "submitted={}, completed={}, pending_peak={}, pending_at_drop={}",
            self.submitted, self.completed, self.pending_peak, self.pending.len(),
        ));
        // Tickets must finish before their source/destination pools and the
        // image-export declaration can be released, including failed jobs.
        self.pending.clear();
        unsafe {
            bsr_metal_transfer_destroy(self.transfer);
            // Keep the pool alive while removing our borrowed pNext. Other
            // AVFrames may retain the pool beyond the adapter's lifetime.
            let pool = &mut *(*(*self.source.as_ptr()).hw_frames_ctx)
                .data
                .cast::<ffi::AVHWFramesContext>();
            let vk = &mut *pool.hwctx.cast::<ffi::AVVulkanFramesContext>();
            if vk.create_pnext
                == (&*self.declaration as *const ExportCreateInfo)
                    .cast_mut()
                    .cast()
            {
                vk.create_pnext = self.declaration.next.cast_mut();
            }
        }
    }
}
