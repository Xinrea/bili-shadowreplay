use super::*;
use native_overlay::vk;

#[repr(C)]
pub struct VulkanContext {
    instance: *mut c_void,
    physical_device: *mut c_void,
    device: *mut c_void,
    queue: *mut c_void,
    queue_family: u32,
    width: u32,
    height: u32,
    resolver: *mut c_void,
    get_proc:
        unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void, *mut c_void) -> *const c_void,
    instance_extensions: *const *const c_char,
    instance_extension_count: u32,
    device_extensions: *const *const c_char,
    device_extension_count: u32,
}

#[repr(C)]
pub struct VulkanImage {
    image: u64,
    memory: u64,
    offset: u64,
    size: u64,
    layout: u32,
    queue_family: u32,
}

impl VulkanImage {
    unsafe fn info(&self) -> vk::ImageInfo {
        let alloc = vk::Alloc::from_device_memory(
            self.memory as _,
            self.offset,
            self.size,
            vk::AllocFlag::empty(),
        );
        let mut info = vk::ImageInfo::new(
            self.image as _,
            alloc,
            vk::ImageTiling::OPTIMAL,
            if self.layout == 0 {
                vk::ImageLayout::UNDEFINED
            } else {
                vk::ImageLayout::GENERAL
            },
            vk::Format::R8G8B8A8_UNORM,
            1,
            Some(self.queue_family),
            None,
            None,
            Some(if self.queue_family == u32::MAX {
                vk::SharingMode::CONCURRENT
            } else {
                vk::SharingMode::EXCLUSIVE
            }),
        );
        info.image_usage_flags = 0x01 | 0x02 | 0x04 | 0x08 | 0x10; // transfer, sampled, storage, color attachment
        info
    }
}

unsafe fn renderer(
    config: *const c_char,
    context: &VulkanContext,
    image: &VulkanImage,
) -> Result<(Renderer, f64), String> {
    let scene = scene(config)?;
    let resolve = |proc| match proc {
        vk::GetProcOf::Instance(instance, name) => (context.get_proc)(
            context.resolver,
            name,
            instance.cast(),
            std::ptr::null_mut(),
        ),
        vk::GetProcOf::Device(device, name) => {
            (context.get_proc)(context.resolver, name, std::ptr::null_mut(), device.cast())
        }
    };
    let strings = |pointer: *const *const c_char, length: u32| {
        if length == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(pointer, length as usize)
                .iter()
                .map(|v| CStr::from_ptr(*v).to_str().unwrap())
                .collect::<Vec<_>>()
        }
    };
    let mut backend = vk::BackendContext::new_with_extensions(
        context.instance.cast(),
        context.physical_device.cast(),
        context.device.cast(),
        (context.queue.cast(), context.queue_family as usize),
        &resolve,
        &strings(
            context.instance_extensions,
            context.instance_extension_count,
        ),
        &strings(context.device_extensions, context.device_extension_count),
    );
    backend.set_max_api_version((1, 3, 0));
    Renderer::new_vulkan(
        context.width,
        context.height,
        scene.input,
        &backend,
        &image.info(),
    )
    .map(|renderer| (renderer, scene.start))
}

#[no_mangle]
pub unsafe extern "C" fn bsr_skia_vulkan_new(
    config: *const c_char,
    context: *const VulkanContext,
    image: *const VulkanImage,
) -> *mut c_void {
    new_renderer(renderer(config, &*context, &*image))
}

#[no_mangle]
pub unsafe extern "C" fn bsr_skia_vulkan_draw(
    renderer: *mut c_void,
    time: f64,
    image: *const VulkanImage,
) -> i32 {
    let renderer = &mut *renderer.cast::<GpuRenderer>();
    draw_result(
        renderer
            .renderer
            .render_vulkan(time + renderer.start, &(*image).info()),
    )
}
