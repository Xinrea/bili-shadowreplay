//! MoltenVK GPU NV12 -> IOSurface -> FFmpeg VideoToolbox hardware frame.
//!
//! The Vulkan image and destination CVPixelBuffer stay owned throughout a GPU
//! blit. Completion waits do not map/download pixels. Encoding and muxing stay
//! in MediaPlan; this adapter owns neither a codec nor an output file.

use super::*;

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
    fn bsr_metal_transfer_copy(
        context: *mut c_void,
        luma: *mut c_void,
        chroma: *mut c_void,
        pixel_buffer: *mut c_void,
        error: *mut std::ffi::c_char,
        capacity: usize,
    ) -> i32;
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
}

impl VideoToolboxFrameBridge {
    pub fn new(compositor: &mut VulkanCompositor) -> Result<Self, String> {
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

    pub fn convert(&mut self, source: frame::Video) -> Result<frame::Video, String> {
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
            for plane in 0..2 {
                if vk.img[plane] == 0 {
                    return Err("MoltenVK NV12 requires separate R8/RG8 images".into());
                }
                self.device
                    .0
                    .sync
                    .wait(vk.sem[plane], vk.sem_value[plane])?;
            }
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
            if bsr_metal_transfer_copy(
                self.transfer,
                luma.texture,
                chroma.texture,
                pixel_buffer,
                error.as_mut_ptr(),
                error.len(),
            ) != 0
            {
                return Err(CStr::from_ptr(error.as_ptr())
                    .to_string_lossy()
                    .into_owned());
            }
            Ok(result)
        }
    }
}

impl Drop for VideoToolboxFrameBridge {
    fn drop(&mut self) {
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
