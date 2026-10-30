//! Vulkan export targets borrowed from a media frame pool.
//!
//! The scene compositor and its cached sprites draw into GPU surfaces. This
//! module has no pixel-readback API: it returns Vulkan image state to the
//! caller, which owns the image, synchronization and encoding frame lease.

use crate::{renderer::Renderer, RenderInput, RenderStats};
use skia_safe::{
    gpu::{self, vk},
    Color, ColorType,
};
use std::{ffi::c_void, ffi::CStr, path::PathBuf, time::Duration};

/// Raw handles borrowed from one externally owned Vulkan device. Queue zero
/// of `queue_family` must support graphics; all handles must name that device.
#[derive(Clone, Debug)]
pub struct NativeVulkanDevice {
    pub instance: usize,
    pub physical_device: usize,
    pub device: usize,
    pub queue: usize,
    pub queue_family: u32,
    pub instance_extensions: Vec<String>,
    pub device_extensions: Vec<String>,
    pub max_api_version: u32,
}

/// A Vulkan function requested while constructing the Skia context.
#[derive(Clone, Copy, Debug)]
pub enum VulkanProcAddress<'a> {
    Instance { handle: usize, name: &'a CStr },
    Device { handle: usize, name: &'a CStr },
}

/// One BGRA8 image owned by the caller's media frame pool. Its dimensions are
/// those given to [`VulkanRenderer::new`]. The optional memory description is
/// diagnostic/borrowing information, never an ownership transfer to Skia.
#[derive(Clone, Copy, Debug)]
pub struct ExternalVulkanImage {
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

/// Layout and queue-family state after all Skia writes have completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VulkanImageState {
    pub layout: u32,
    pub queue_family: u32,
}

/// CPU submission wall times; `submit_and_wait` includes an explicit GPU
/// completion wait. The PoC avoids frame readback, not CPU synchronization.
#[derive(Clone, Debug, Default)]
pub struct VulkanRenderStats {
    pub frames: u64,
    pub texture_wraps: u64,
    pub composition: Duration,
    pub submit_and_wait: Duration,
    pub scene: RenderStats,
}

/// GPU renderer bound to a borrowed device. Fields are dropped in declaration
/// order: scene images/surfaces are released before the direct context.
pub struct VulkanRenderer {
    renderer: Renderer,
    width: u32,
    height: u32,
    stats: VulkanRenderStats,
    context: gpu::DirectContext,
}

impl VulkanRenderer {
    /// Create a scene using the same layout, font and emoji logic as [`Renderer`].
    ///
    /// # Safety
    /// The device/instance/physical-device and graphics queue handles must be
    /// valid and remain alive until this renderer is dropped. `get_proc` must
    /// return the correct Vulkan ABI functions for those same handles. External
    /// submissions to this queue must be synchronized with rendering calls.
    pub unsafe fn new(
        width: u32,
        height: u32,
        input: RenderInput,
        font_paths: Vec<PathBuf>,
        device: &NativeVulkanDevice,
        get_proc: impl Fn(VulkanProcAddress<'_>) -> *const c_void,
    ) -> Result<Self, String> {
        validate_device(device)?;
        validate_extension_procs(device.instance, &get_proc)?;
        // Parse/validate scenes before touching an external device, retaining
        // only a four-byte CPU placeholder between borrowed target surfaces.
        let renderer = Renderer::new_gpu_scene(width, height, input, font_paths)?;
        let resolver = |requested: vk::GetProcOf| {
            let (handle, name, is_device) = match requested {
                vk::GetProcOf::Instance(handle, name) => (handle as usize, name, false),
                vk::GetProcOf::Device(handle, name) => (handle as usize, name, true),
            };
            if name.is_null() {
                return std::ptr::null();
            }
            // Skia supplies a terminated Vulkan entry point name for this call.
            let name = unsafe { CStr::from_ptr(name) };
            get_proc(if is_device {
                VulkanProcAddress::Device { handle, name }
            } else {
                VulkanProcAddress::Instance { handle, name }
            })
        };
        let instance_extensions: Vec<_> = device
            .instance_extensions
            .iter()
            .map(String::as_str)
            .collect();
        let device_extensions: Vec<_> = device
            .device_extensions
            .iter()
            .map(String::as_str)
            .collect();
        let mut backend = unsafe {
            vk::BackendContext::new_with_extensions(
                device.instance as vk::Instance,
                device.physical_device as vk::PhysicalDevice,
                device.device as vk::Device,
                (device.queue as vk::Queue, device.queue_family as usize),
                &resolver,
                &instance_extensions,
                &device_extensions,
            )
        };
        if device.max_api_version != 0 {
            backend.set_max_api_version(device.max_api_version);
        }
        let mut context = gpu::direct_contexts::make_vulkan(&backend, None)
            .ok_or("Cannot create Skia Vulkan direct context for the media device")?;
        context.set_resource_cache_limit(64 * 1024 * 1024);
        Ok(Self {
            renderer,
            width,
            height,
            stats: VulkanRenderStats::default(),
            context,
        })
    }

    pub fn set_profiling(&mut self, enabled: bool) {
        self.renderer.set_profiling(enabled);
    }

    pub fn stats(&self) -> VulkanRenderStats {
        let mut stats = self.stats.clone();
        stats.scene = self.renderer.stats();
        stats
    }

    pub fn missing_glyph_count(&self) -> usize {
        self.renderer.missing_glyph_count()
    }

    /// Draw into an externally owned BGRA8 image, wait for GPU completion and
    /// return its new layout. `background_rgba` supplies the video base colour;
    /// use alpha 255 for a directly encodable opaque test scene.
    ///
    /// # Safety
    /// The image/memory must belong to the constructor's device, have the given
    /// dimensions and usage, and remain alive through this call. The caller
    /// must finish/wait previous consumers before reuse and prevent concurrent
    /// image access. On error abort this frame/job rather than reusing an image
    /// with its old state: some GPU drawing may already have been submitted.
    pub unsafe fn render_into(
        &mut self,
        time: f64,
        image: &ExternalVulkanImage,
        background_rgba: [u8; 4],
    ) -> Result<VulkanImageState, String> {
        if !time.is_finite() || time < 0.0 {
            return Err("Vulkan render time must be finite and non-negative".into());
        }
        validate_image(image)?;
        let layout = image_layout(image.layout)?;
        let sharing = match image.sharing_mode {
            0 => vk::SharingMode::EXCLUSIVE,
            1 => vk::SharingMode::CONCURRENT,
            _ => return Err("Invalid Vulkan image sharing mode".into()),
        };
        let allocation = if image.memory == 0 {
            vk::Alloc::default()
        } else {
            unsafe {
                vk::Alloc::from_device_memory(
                    image.memory as vk::DeviceMemory,
                    image.memory_offset,
                    image.memory_size,
                    vk::AllocFlag::empty(),
                )
            }
        };
        let mut image_info = unsafe {
            vk::ImageInfo::new(
                image.image as vk::Image,
                allocation,
                vk::ImageTiling::OPTIMAL,
                layout,
                vk::Format::B8G8R8A8_UNORM,
                1,
                Some(image.queue_family),
                None,
                None,
                Some(sharing),
            )
        };
        image_info.image_usage_flags = image.usage;
        let backend = unsafe {
            gpu::backend_textures::make_vk(
                (self.width as i32, self.height as i32),
                &image_info,
                "shared-media-bgra",
            )
        };
        let target = gpu::surfaces::wrap_backend_texture(
            &mut self.context,
            &backend,
            gpu::SurfaceOrigin::TopLeft,
            Some(0),
            ColorType::BGRA8888,
            None,
            None,
        )
        .ok_or("Cannot wrap media VkImage as a Skia render target")?;
        self.stats.texture_wraps += 1;
        let [r, g, b, a] = background_rgba;
        let (target, composition, submitted) = self.renderer.compose_gpu(
            time,
            Color::from_argb(a, r, g, b),
            &mut self.context,
            target,
        )?;
        // BackendTexture copies share Vulkan's mutable texture state. Read
        // that state directly after the completion wait; a Surface FlushRead
        // request here could otherwise enqueue new work after the wait.
        let current = gpu::backend_textures::get_vk_image_info(&backend)
            .ok_or("Skia Vulkan render target lost its image state")?;
        let state = VulkanImageState {
            layout: current.layout as u32,
            queue_family: current.current_queue_family,
        };
        // Do not retain a borrowed frame in the scene between render calls.
        drop(target);
        self.stats.frames += 1;
        self.stats.composition += composition;
        self.stats.submit_and_wait += submitted;
        Ok(state)
    }
}

impl Drop for VulkanRenderer {
    fn drop(&mut self) {
        self.context.flush_submit_and_sync_cpu();
    }
}

fn validate_device(device: &NativeVulkanDevice) -> Result<(), String> {
    if device.instance == 0
        || device.physical_device == 0
        || device.device == 0
        || device.queue == 0
    {
        return Err("Skia Vulkan requires live instance/device/graphics queue handles".into());
    }
    if device.queue_family == u32::MAX {
        return Err("Skia Vulkan requires an explicit graphics queue family".into());
    }
    if device
        .instance_extensions
        .iter()
        .chain(&device.device_extensions)
        .any(|name| name.is_empty() || name.contains('\0'))
    {
        return Err("Invalid Vulkan extension name".into());
    }
    Ok(())
}

fn validate_extension_procs(
    instance: usize,
    get_proc: &impl Fn(VulkanProcAddress<'_>) -> *const c_void,
) -> Result<(), String> {
    // Skia's extension discovery only asserts these pointers before calling
    // them. In release builds an incomplete resolver would call null instead
    // of returning a failed context. Global queries must preserve instance 0.
    for (handle, name) in [
        (0, c"vkEnumerateInstanceExtensionProperties"),
        (instance, c"vkEnumerateDeviceExtensionProperties"),
    ] {
        if get_proc(VulkanProcAddress::Instance { handle, name }).is_null() {
            return Err(format!(
                "Skia Vulkan function {} is unavailable for instance {handle:#x}",
                name.to_string_lossy()
            ));
        }
    }
    Ok(())
}

fn validate_image(image: &ExternalVulkanImage) -> Result<(), String> {
    // VK_FORMAT_B8G8R8A8_UNORM; CPU readback formats cannot enter this API.
    if image.image == 0 || image.format != 44 {
        return Err("Skia Vulkan target must be a live BGRA8_UNORM VkImage".into());
    }
    // Wrapping as a renderable texture requires both COLOR_ATTACHMENT and SAMPLED.
    if image.usage & (0x10 | 0x04) != (0x10 | 0x04) {
        return Err("Skia Vulkan image needs COLOR_ATTACHMENT and SAMPLED usage".into());
    }
    image_layout(image.layout)?;
    Ok(())
}

fn image_layout(layout: u32) -> Result<vk::ImageLayout, String> {
    // Only layouts meaningful for the shared BGRA colour image are admitted;
    // avoid transmuting arbitrary extension values into a Rust Vulkan enum.
    match layout {
        0 => Ok(vk::ImageLayout::UNDEFINED),
        1 => Ok(vk::ImageLayout::GENERAL),
        2 => Ok(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
        5 => Ok(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL),
        6 => Ok(vk::ImageLayout::TRANSFER_SRC_OPTIMAL),
        7 => Ok(vk::ImageLayout::TRANSFER_DST_OPTIMAL),
        _ => Err("Unsupported shared BGRA Vulkan image layout".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn extension_lookup_preserves_global_and_instance_scopes() {
        let requests = RefCell::new(Vec::new());
        validate_extension_procs(0x1234, &|request| {
            match request {
                VulkanProcAddress::Instance { handle, name } => requests
                    .borrow_mut()
                    .push((handle, name.to_string_lossy().into_owned())),
                VulkanProcAddress::Device { .. } => panic!("Expected instance lookup"),
            }
            std::ptr::dangling::<c_void>()
        })
        .unwrap();
        assert_eq!(
            requests.into_inner(),
            [
                (0, "vkEnumerateInstanceExtensionProperties".into()),
                (0x1234, "vkEnumerateDeviceExtensionProperties".into()),
            ]
        );
    }

    #[test]
    fn missing_extension_lookup_returns_error_before_skia_can_call_null() {
        let device = NativeVulkanDevice {
            instance: 0x1234,
            physical_device: 2,
            device: 3,
            queue: 4,
            queue_family: 0,
            instance_extensions: Vec::new(),
            device_extensions: Vec::new(),
            max_api_version: 0,
        };
        for (missing_handle, missing_name) in [
            (0, "vkEnumerateInstanceExtensionProperties"),
            (0x1234, "vkEnumerateDeviceExtensionProperties"),
        ] {
            // The fake handles/pointers must never reach native Skia. This
            // constructor path returns before creating any GPU objects.
            let error = unsafe {
                VulkanRenderer::new(
                    16,
                    16,
                    RenderInput::default(),
                    Vec::new(),
                    &device,
                    |request| match request {
                        VulkanProcAddress::Instance { handle, .. } if handle == missing_handle => {
                            std::ptr::null()
                        }
                        _ => std::ptr::dangling::<c_void>(),
                    },
                )
            }
            .err()
            .expect("Missing extension resolver must fail before calling Skia");
            assert!(error.contains(missing_name), "{error}");
            assert!(
                error.contains(&format!("instance {missing_handle:#x}")),
                "{error}"
            );
        }
    }

    #[test]
    fn rejects_invalid_external_targets_before_gpu_access() {
        let mut target = ExternalVulkanImage {
            image: 1,
            memory: 0,
            memory_offset: 0,
            memory_size: 0,
            layout: 1,
            format: 44,
            usage: 0x10 | 0x04,
            queue_family: 0,
            sharing_mode: 0,
        };
        assert!(validate_image(&target).is_ok());
        target.format = 37;
        assert!(validate_image(&target).is_err());
        target.format = 44;
        target.usage = 0x04;
        assert!(validate_image(&target).is_err());
        target.usage = 0x10 | 0x04;
        target.layout = 999;
        assert!(validate_image(&target).is_err());
        target.layout = 1;
        target.image = 0;
        assert!(validate_image(&target).is_err());
    }
}
