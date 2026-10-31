//! Shared NV12 + premultiplied RGBA compute dispatch. Uses FFmpeg's existing
//! device, queue locks, frame locks and timelines; ash only supplies ABI types
//! and dispatch tables (no loader, device creation or ownership of VkDevice).
use super::*;
use crate::capabilities::SdrTransfer;
use ash::vk::{self, Handle};
use std::collections::{BTreeMap, HashSet};

const TIMEOUT: u64 = 30_000_000_000;

#[derive(Default)]
struct Slot {
    pool: vk::CommandPool,
    command: vk::CommandBuffer,
    fence: vk::Fence,
    descriptors: vk::DescriptorSet,
    submitted: bool,
    views: Vec<vk::ImageView>,
    frames: Vec<frame::Video>,
}

pub(super) struct ComputeCompositor {
    api: ash::Device,
    device: VulkanDevice,
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    descriptors: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    sampler: vk::Sampler,
    slots: Vec<Slot>,
    next: usize,
}

fn vk_result<T>(value: Result<T, vk::Result>, operation: &str) -> Result<T, String> {
    value.map_err(|e| format!("{operation}: VkResult {}", e.as_raw()))
}

impl ComputeCompositor {
    pub(super) fn new(device: VulkanDevice, count: usize) -> Result<Self, String> {
        if !(1..=3).contains(&count) {
            return Err("Compute compositor requires one to three command slots".into());
        }
        // ffmpeg-sys represents VkPhysicalDeviceFeatures as its 55 VkBool32s.
        // The checked transmute preserves that stable Vulkan C ABI.
        let features: vk::PhysicalDeviceFeatures =
            unsafe { std::mem::transmute(device.0.hw().device_features.features) };
        if features.shader_storage_image_write_without_format == 0 {
            return Err("Compute NV12 requires shaderStorageImageWriteWithoutFormat".into());
        }
        // FFmpeg's qf flags describe assigned roles and can list the same
        // physical family separately for graphics/compute. Query real support.
        let instance = unsafe {
            ash::Instance::load_with(
                |name| device.get_proc_addr(false, name),
                vk::Instance::from_raw(device.0.info.instance as u64),
            )
        };
        let families = unsafe {
            instance.get_physical_device_queue_family_properties(vk::PhysicalDevice::from_raw(
                device.0.info.physical_device as u64,
            ))
        };
        if !families
            .get(device.0.info.queue_family as usize)
            .is_some_and(|family| family.queue_flags.contains(vk::QueueFlags::COMPUTE))
        {
            return Err("Shared graphics queue does not support compute".into());
        }
        let api = unsafe {
            ash::Device::load_with(
                |name| device.get_proc_addr(true, name),
                vk::Device::from_raw(device.0.info.device as u64),
            )
        };
        let mut result = Self {
            api,
            device,
            pipeline: vk::Pipeline::null(),
            layout: vk::PipelineLayout::null(),
            descriptors: vk::DescriptorSetLayout::null(),
            descriptor_pool: vk::DescriptorPool::null(),
            sampler: vk::Sampler::null(),
            slots: Vec::new(),
            next: 0,
        };
        unsafe {
            result.initialize(count)?;
        }
        Ok(result)
    }

    unsafe fn initialize(&mut self, count: usize) -> Result<(), String> {
        let bindings: Vec<_> = (0..5)
            .map(|binding| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(binding)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::COMPUTE)
                    .descriptor_type(if binding < 3 {
                        vk::DescriptorType::COMBINED_IMAGE_SAMPLER
                    } else {
                        vk::DescriptorType::STORAGE_IMAGE
                    })
            })
            .collect();
        self.descriptors = vk_result(
            self.api.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            ),
            "Create compute bindings",
        )?;
        let sets = [self.descriptors];
        let constants = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .size(16)];
        self.layout = vk_result(
            self.api.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&sets)
                    .push_constant_ranges(&constants),
                None,
            ),
            "Create compute layout",
        )?;
        let words: Vec<u32> = include_bytes!("shaders/composite.spv")
            .chunks_exact(4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
            .collect();
        let shader = vk_result(
            self.api
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None),
            "Create embedded NV12 shader",
        )?;
        let stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(shader)
            .name(c"main");
        let pipeline = self.api.create_compute_pipelines(
            vk::PipelineCache::null(),
            &[vk::ComputePipelineCreateInfo::default()
                .stage(stage)
                .layout(self.layout)],
            None,
        );
        self.api.destroy_shader_module(shader, None);
        self.pipeline = match pipeline {
            Ok(pipelines) => pipelines[0],
            Err((pipelines, error)) => {
                for pipeline in pipelines {
                    self.api.destroy_pipeline(pipeline, None);
                }
                return Err(format!("Create NV12 compute pipeline: {}", error.as_raw()));
            }
        };
        self.sampler = vk_result(
            self.api.create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(vk::Filter::NEAREST)
                    .min_filter(vk::Filter::NEAREST)
                    .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                None,
            ),
            "Create compute sampler",
        )?;
        let sizes = [
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(3 * count as u32),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::STORAGE_IMAGE)
                .descriptor_count(2 * count as u32),
        ];
        self.descriptor_pool = vk_result(
            self.api.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(count as u32)
                    .pool_sizes(&sizes),
                None,
            ),
            "Create compute descriptor pool",
        )?;
        for _ in 0..count {
            // Push the partially initialized slot first so every error path
            // destroys handles already created, in the same owning scope.
            self.slots.push(Slot::default());
            let slot = self.slots.last_mut().unwrap();
            slot.pool = vk_result(
                self.api.create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(self.device.0.info.queue_family),
                    None,
                ),
                "Create compute command pool",
            )?;
            slot.command = vk_result(
                self.api.allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(slot.pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                ),
                "Allocate compute command",
            )?[0];
            slot.fence = vk_result(
                self.api.create_fence(&vk::FenceCreateInfo::default(), None),
                "Create compute fence",
            )?;
            slot.descriptors = vk_result(
                self.api.allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(self.descriptor_pool)
                        .set_layouts(&sets),
                ),
                "Allocate compute bindings",
            )?[0];
        }
        Ok(())
    }

    fn retire(api: &ash::Device, slot: &mut Slot, wait: bool) -> Result<bool, String> {
        if slot.submitted {
            let complete = if wait {
                vk_result(
                    unsafe { api.wait_for_fences(&[slot.fence], true, TIMEOUT) },
                    "Wait for compute slot",
                )?;
                true
            } else {
                vk_result(
                    unsafe { api.get_fence_status(slot.fence) },
                    "Poll compute slot",
                )?
            };
            if !complete {
                return Ok(false);
            }
            slot.submitted = false;
        }
        // Views are destroyed before their owning frame references. The
        // encoder/Metal bridge may independently retain these same AVFrames.
        for view in slot.views.drain(..) {
            unsafe { api.destroy_image_view(view, None) };
        }
        slot.frames.clear();
        Ok(true)
    }

    pub(super) fn collect_completed(&mut self) -> Result<(), String> {
        for slot in &mut self.slots {
            Self::retire(&self.api, slot, false)?;
        }
        Ok(())
    }

    pub(super) fn drain(&mut self) -> Result<(), String> {
        for slot in &mut self.slots {
            Self::retire(&self.api, slot, true)?;
        }
        Ok(())
    }

    pub(super) fn submit(
        &mut self,
        overlay: &frame::Video,
        base: Option<&frame::Video>,
        output: &frame::Video,
        transfer: SdrTransfer,
    ) -> Result<(), String> {
        let _span = crate::profiling::span("compute_submit");
        let slot = &mut self.slots[self.next];
        crate::profiling::measure("compute_slot_wait", || Self::retire(&self.api, slot, true))?;
        slot.frames.push(reference_video_frame(overlay)?);
        if let Some(base) = base {
            slot.frames.push(reference_video_frame(base)?);
        }
        slot.frames.push(reference_video_frame(output)?);
        // Stable pointer ordering makes multi-frame locking independent of
        // frame role, including when a caller reuses an earlier output as base.
        let mut pointers: Vec<_> = slot
            .frames
            .iter()
            .map(|frame| unsafe { (*frame.as_ptr()).data[0] as usize })
            .collect();
        pointers.sort_unstable();
        if pointers.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err("Compute input/output frames must not alias".into());
        }
        let mut locked = Vec::new();
        for pointer in pointers {
            let frame = slot
                .frames
                .iter()
                .find(|frame| unsafe { (*frame.as_ptr()).data[0] as usize == pointer })
                .unwrap();
            locked.push(LockedFrame::new(frame, &self.device)?);
        }
        let overlay_index = locked
            .iter()
            .position(|f| f.raw == unsafe { (*overlay.as_ptr()).data[0].cast() })
            .unwrap();
        let output_index = locked
            .iter()
            .position(|f| f.raw == unsafe { (*output.as_ptr()).data[0].cast() })
            .unwrap();
        let base_index = base.map(|base| {
            locked
                .iter()
                .position(|f| f.raw == unsafe { (*base.as_ptr()).data[0].cast() })
                .unwrap()
        });
        let mut images = HashSet::new();
        let mut timelines = BTreeMap::<u64, u64>::new();
        let mut barriers = Vec::new();
        for (index, frame) in locked.iter().enumerate() {
            let raw = unsafe { &*frame.raw };
            for plane in 0..frame.images {
                if !images.insert(raw.img[plane]) {
                    return Err("Compute images must not alias".into());
                }
                let value = timelines.entry(raw.sem[plane]).or_default();
                *value = (*value).max(raw.sem_value[plane]);
                let old_layout = vk::ImageLayout::from_raw(raw.layout[plane]);
                let access = if index == output_index {
                    vk::AccessFlags2::SHADER_WRITE
                } else {
                    vk::AccessFlags2::SHADER_READ
                };
                barriers.push(
                    vk::ImageMemoryBarrier2::default()
                        .src_stage_mask(if old_layout == vk::ImageLayout::UNDEFINED {
                            vk::PipelineStageFlags2::NONE
                        } else {
                            vk::PipelineStageFlags2::ALL_COMMANDS
                        })
                        .src_access_mask(if old_layout == vk::ImageLayout::UNDEFINED {
                            vk::AccessFlags2::NONE
                        } else {
                            vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE
                        })
                        .dst_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
                        .dst_access_mask(access)
                        .old_layout(old_layout)
                        .new_layout(vk::ImageLayout::GENERAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(vk::Image::from_raw(raw.img[plane]))
                        .subresource_range(full_range(vk::ImageAspectFlags::COLOR)),
                );
            }
        }
        let semaphores: Vec<_> = timelines
            .keys()
            .copied()
            .map(vk::Semaphore::from_raw)
            .collect();
        let waits: Vec<_> = timelines.values().copied().collect();
        let signals: Vec<_> = waits
            .iter()
            .map(|value| {
                value
                    .checked_add(1)
                    .ok_or("Compute frame timeline exhausted")
            })
            .collect::<Result<_, _>>()?;
        let next_values: BTreeMap<_, _> = timelines
            .keys()
            .copied()
            .zip(signals.iter().copied())
            .collect();
        let overlay_view = locked[overlay_index].view(&self.api, 0, false)?;
        slot.views.push(overlay_view);
        let mut views = [
            overlay_view,
            overlay_view,
            overlay_view,
            vk::ImageView::null(),
            vk::ImageView::null(),
        ];
        if let Some(index) = base_index {
            for (plane, view) in views[..2].iter_mut().enumerate() {
                *view = locked[index].view(&self.api, plane, false)?;
                slot.views.push(*view);
            }
        }
        for (plane, view) in views[3..].iter_mut().enumerate() {
            *view = locked[output_index].view(&self.api, plane, true)?;
            slot.views.push(*view);
        }
        let infos: Vec<_> = views
            .iter()
            .enumerate()
            .map(|(index, view)| {
                vk::DescriptorImageInfo::default()
                    .sampler(if index < 3 {
                        self.sampler
                    } else {
                        vk::Sampler::null()
                    })
                    .image_view(*view)
                    .image_layout(vk::ImageLayout::GENERAL)
            })
            .collect();
        let writes: Vec<_> = (0..5)
            .map(|index| {
                vk::WriteDescriptorSet::default()
                    .dst_set(slot.descriptors)
                    .dst_binding(index as u32)
                    .descriptor_type(if index < 3 {
                        vk::DescriptorType::COMBINED_IMAGE_SAMPLER
                    } else {
                        vk::DescriptorType::STORAGE_IMAGE
                    })
                    .image_info(&infos[index..index + 1])
            })
            .collect();
        unsafe {
            self.api.update_descriptor_sets(&writes, &[]);
            vk_result(self.api.reset_fences(&[slot.fence]), "Reset compute fence")?;
            vk_result(
                self.api
                    .reset_command_pool(slot.pool, vk::CommandPoolResetFlags::empty()),
                "Reset compute command pool",
            )?;
            vk_result(
                self.api.begin_command_buffer(
                    slot.command,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                ),
                "Begin compute command",
            )?;
            self.api.cmd_pipeline_barrier2(
                slot.command,
                &vk::DependencyInfo::default().image_memory_barriers(&barriers),
            );
            self.api
                .cmd_bind_pipeline(slot.command, vk::PipelineBindPoint::COMPUTE, self.pipeline);
            self.api.cmd_bind_descriptor_sets(
                slot.command,
                vk::PipelineBindPoint::COMPUTE,
                self.layout,
                0,
                &[slot.descriptors],
                &[],
            );
            let words = [
                output.width(),
                output.height(),
                u32::from(base.is_some()),
                u32::from(base.is_some() && transfer == SdrTransfer::Srgb),
            ];
            let constants: Vec<u8> = words.iter().flat_map(|v| v.to_ne_bytes()).collect();
            self.api.cmd_push_constants(
                slot.command,
                self.layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                &constants,
            );
            self.api.cmd_dispatch(
                slot.command,
                (output.width() / 2).div_ceil(8),
                (output.height() / 2).div_ceil(8),
                1,
            );
            vk_result(
                self.api.end_command_buffer(slot.command),
                "End compute command",
            )?;
            let stages = vec![vk::PipelineStageFlags::ALL_COMMANDS; semaphores.len()];
            let commands = [slot.command];
            let mut timeline = vk::TimelineSemaphoreSubmitInfo::default()
                .wait_semaphore_values(&waits)
                .signal_semaphore_values(&signals);
            let submission = vk::SubmitInfo::default()
                .push_next(&mut timeline)
                .wait_semaphores(&semaphores)
                .wait_dst_stage_mask(&stages)
                .command_buffers(&commands)
                .signal_semaphores(&semaphores);
            let _queue = QueueLock::new(&self.device);
            vk_result(
                self.api.queue_submit(
                    vk::Queue::from_raw(self.device.0.info.queue as u64),
                    &[submission],
                    slot.fence,
                ),
                "Submit NV12 compute",
            )?;
        }
        slot.submitted = true;
        // Publish both read and write dependencies only after a successful
        // submission, while every referenced AVVkFrame is still locked.
        for (index, frame) in locked.iter_mut().enumerate() {
            let raw = unsafe { &mut *frame.raw };
            for plane in 0..frame.images {
                raw.layout[plane] = vk::ImageLayout::GENERAL.as_raw();
                raw.access[plane] = if index == output_index {
                    vk::AccessFlags2::SHADER_WRITE
                } else {
                    vk::AccessFlags2::SHADER_READ
                }
                .as_raw();
                raw.sem_value[plane] = next_values[&raw.sem[plane]];
            }
        }
        self.next = (self.next + 1) % self.slots.len();
        Ok(())
    }
}

impl Drop for ComputeCompositor {
    fn drop(&mut self) {
        if let Err(error) = self.drain() {
            eprintln!("Compute compositor cleanup: {error}");
            let _ = self.device.0.sync.wait_idle();
        }
        unsafe {
            for slot in &mut self.slots {
                for view in slot.views.drain(..) {
                    self.api.destroy_image_view(view, None);
                }
                self.api.destroy_command_pool(slot.pool, None);
                self.api.destroy_fence(slot.fence, None);
                slot.frames.clear();
            }
            self.api.destroy_descriptor_pool(self.descriptor_pool, None);
            self.api.destroy_pipeline(self.pipeline, None);
            self.api.destroy_pipeline_layout(self.layout, None);
            self.api
                .destroy_descriptor_set_layout(self.descriptors, None);
            self.api.destroy_sampler(self.sampler, None);
        }
    }
}

struct QueueLock<'a>(&'a VulkanDevice);
impl<'a> QueueLock<'a> {
    fn new(device: &'a VulkanDevice) -> Self {
        if let Some(lock) = device.0.hw().lock_queue {
            unsafe { lock(device.0.context(), device.0.info.queue_family, 0) };
        }
        Self(device)
    }
}
impl Drop for QueueLock<'_> {
    fn drop(&mut self) {
        if let Some(unlock) = self.0 .0.hw().unlock_queue {
            unsafe { unlock(self.0 .0.context(), self.0 .0.info.queue_family, 0) };
        }
    }
}

struct LockedFrame {
    pool: *mut ffi::AVHWFramesContext,
    raw: *mut ffi::AVVkFrame,
    images: usize,
    rgba: bool,
    formats: [i32; 2],
}
impl LockedFrame {
    fn new(frame: &frame::Video, device: &VulkanDevice) -> Result<Self, String> {
        unsafe {
            let source = &*frame.as_ptr();
            if frame.format() != ffmpeg::format::Pixel::VULKAN
                || source.hw_frames_ctx.is_null()
                || source.data[0].is_null()
            {
                return Err("Compute requires owning Vulkan hardware frames".into());
            }
            let pool = (*source.hw_frames_ctx)
                .data
                .cast::<ffi::AVHWFramesContext>();
            if pool.is_null()
                || (*pool).hwctx.is_null()
                || (*pool).device_ctx.is_null()
                || (*(*pool).device_ctx).hwctx.is_null()
                || !matches!(
                    (*pool).sw_format,
                    ffi::AVPixelFormat::AV_PIX_FMT_RGBA | ffi::AVPixelFormat::AV_PIX_FMT_NV12
                )
            {
                return Err("Invalid compute frame pool".into());
            }
            let hw = &*(*pool).hwctx.cast::<ffi::AVVulkanFramesContext>();
            let owner = &*(*(*pool).device_ctx)
                .hwctx
                .cast::<ffi::AVVulkanDeviceContext>();
            if owner.act_dev != device.0.hw().act_dev {
                return Err("Foreign compute device".into());
            }
            let raw = source.data[0].cast::<ffi::AVVkFrame>();
            if let Some(lock) = hw.lock_frame {
                lock(pool, raw);
            }
            let mut result = Self {
                pool,
                raw,
                images: 0,
                rgba: (*pool).sw_format == ffi::AVPixelFormat::AV_PIX_FMT_RGBA,
                formats: [hw.format[0], hw.format[1]],
            };
            result.images = plane_layout(result.rgba, result.formats)?;
            for index in 0..result.images {
                if (*raw).img[index] == 0
                    || (*raw).sem[index] == 0
                    || ![vk::QUEUE_FAMILY_IGNORED, device.0.info.queue_family]
                        .contains(&(*raw).queue_family[index])
                {
                    return Err(
                        "Invalid compute image/timeline or unsupported exclusive queue ownership"
                            .into(),
                    );
                }
            }
            if (*raw).img[result.images] != 0 {
                return Err("Unexpected compute frame planes".into());
            }
            Ok(result)
        }
    }

    fn view(
        &self,
        api: &ash::Device,
        plane: usize,
        storage: bool,
    ) -> Result<vk::ImageView, String> {
        let (index, format, aspect) = if self.rgba {
            (0, vk::Format::R8G8B8A8_UNORM, vk::ImageAspectFlags::COLOR)
        } else {
            let format = if plane == 0 {
                vk::Format::R8_UNORM
            } else {
                vk::Format::R8G8_UNORM
            };
            if self.images == 1 {
                (
                    0,
                    format,
                    if plane == 0 {
                        vk::ImageAspectFlags::PLANE_0
                    } else {
                        vk::ImageAspectFlags::PLANE_1
                    },
                )
            } else {
                (plane, format, vk::ImageAspectFlags::COLOR)
            }
        };
        let mut usage = vk::ImageViewUsageCreateInfo::default().usage(if storage {
            vk::ImageUsageFlags::STORAGE
        } else {
            vk::ImageUsageFlags::SAMPLED
        });
        let create = vk::ImageViewCreateInfo::default()
            .push_next(&mut usage)
            .image(vk::Image::from_raw(unsafe { (*self.raw).img[index] }))
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(full_range(aspect));
        vk_result(
            unsafe { api.create_image_view(&create, None) },
            "Create compute plane view",
        )
    }
}
impl Drop for LockedFrame {
    fn drop(&mut self) {
        unsafe {
            let hw = &*(*self.pool).hwctx.cast::<ffi::AVVulkanFramesContext>();
            if let Some(unlock) = hw.unlock_frame {
                unlock(self.pool, self.raw);
            }
        }
    }
}

fn full_range(aspect: vk::ImageAspectFlags) -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(aspect)
        .level_count(1)
        .layer_count(1)
}

fn plane_layout(rgba: bool, formats: [i32; 2]) -> Result<usize, String> {
    match (rgba, formats) {
        (true, [37, 0]) => Ok(1),             // R8G8B8A8_UNORM
        (false, [1_000_156_003, 0]) => Ok(1), // NV12 multi-planar (Windows)
        (false, [9, 16]) => Ok(2),            // R8 + RG8 (MoltenVK)
        _ => Err("Unsupported compute plane format".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plane_views_cover_both_platform_layouts_without_reinterpreting_rgba() {
        assert_eq!(plane_layout(false, [1_000_156_003, 0]).unwrap(), 1);
        assert_eq!(plane_layout(false, [9, 16]).unwrap(), 2);
        assert_eq!(plane_layout(true, [37, 0]).unwrap(), 1);
        assert!(plane_layout(true, [44, 0]).is_err());
        assert!(plane_layout(false, [37, 0]).is_err());
    }

    // The sole readback in this module is an explicit test oracle. Production
    // never downloads the compositor output. Non-workgroup-sized geometry
    // also exercises the dispatch bounds and padded software row strides.
    #[test]
    #[ignore = "requires an accessible Vulkan GPU (MoltenVK on macOS)"]
    fn gpu_compute_matches_bt709_premultiplied_reference() {
        crate::metadata::initialize().unwrap();
        let options = VulkanRenderOptions {
            width: 18,
            height: 10,
            ..Default::default()
        };
        let device = VulkanDevice::new_with_mode("", VulkanSubmissionMode::Asynchronous).unwrap();
        let rgb_pool = FramePool::new(device.clone(), &options).unwrap();
        let nv12_pool = FramePool::new_nv12(device.clone(), &options, false).unwrap();
        let mut compute = ComputeCompositor::new(device, 3).unwrap();
        let mut base = frame::Video::new(ffmpeg::format::Pixel::NV12, 18, 10);
        for plane in 0..2 {
            let stride = base.stride(plane);
            for y in 0..if plane == 0 { 10 } else { 5 } {
                for x in 0..18 {
                    base.data_mut(plane)[y * stride + x] = if plane == 0 {
                        (16 + (x * 13 + y * 17) % 220) as u8
                    } else {
                        (16 + (x * 19 + y * 23) % 225) as u8
                    };
                }
            }
        }
        fn allocate(pool: &FramePool) -> frame::Video {
            let mut gpu = frame::Video::empty();
            unsafe {
                check(
                    ffi::av_hwframe_get_buffer(pool.raw.0, gpu.as_mut_ptr(), 0),
                    "test allocation",
                )
                .unwrap();
            }
            gpu
        }
        fn upload(pool: &FramePool, cpu: &frame::Video) -> frame::Video {
            let mut gpu = allocate(pool);
            unsafe {
                check(
                    ffi::av_hwframe_transfer_data(gpu.as_mut_ptr(), cpu.as_ptr(), 0),
                    "test upload",
                )
                .unwrap();
            }
            gpu
        }
        let gpu_base = upload(&nv12_pool, &base);
        let mut checked = 0;
        let mut maximum_error = 0;
        for with_base in [true, false] {
            let mut pending = Vec::new();
            for case in 0..19 {
                let mut rgba = frame::Video::new(ffmpeg::format::Pixel::RGBA, 18, 10);
                let stride = rgba.stride(0);
                for y in 0..10 {
                    for x in 0..18 {
                        let color = [
                            [255, 0, 0],
                            [0, 255, 0],
                            [0, 0, 255],
                            [255, 255, 255],
                            [0, 0, 0],
                            [53, 179, 241],
                        ][case % 6];
                        let alpha = if case < 6 {
                            0
                        } else if case < 12 {
                            255
                        } else if case < 18 {
                            128
                        } else {
                            [0, 64, 128, 255][(y % 2) * 2 + x % 2]
                        };
                        let offset = y * stride + x * 4;
                        for (c, value) in color.into_iter().enumerate() {
                            rgba.data_mut(0)[offset + c] = ((value * alpha + 127) / 255) as u8;
                        }
                        rgba.data_mut(0)[offset + 3] = alpha as u8;
                    }
                }
                let overlay = upload(&rgb_pool, &rgba);
                let output = allocate(&nv12_pool);
                compute
                    .submit(
                        &overlay,
                        with_base.then_some(&gpu_base),
                        &output,
                        SdrTransfer::Bt709,
                    )
                    .unwrap();
                pending.push((case, rgba, output));
                // Three in-flight submissions and a one-frame tail. Reusing
                // the same base exercises updated input timeline dependencies.
                if pending.len() < 3 && case != 18 {
                    continue;
                }
                for (case, rgba, output) in pending.drain(..) {
                    let mut actual = frame::Video::empty();
                    unsafe {
                        check(
                            ffi::av_hwframe_transfer_data(actual.as_mut_ptr(), output.as_ptr(), 0),
                            "test-only download",
                        )
                        .unwrap();
                    }
                    assert_eq!(actual.format(), ffmpeg::format::Pixel::NV12);
                    let channel = |x: usize, y: usize, c: usize| {
                        rgba.data(0)[y * rgba.stride(0) + x * 4 + c] as f64 / 255.0
                    };
                    let pixel = |x: usize, y: usize| {
                        let (r, g, b, a) = (
                            channel(x, y, 0),
                            channel(x, y, 1),
                            channel(x, y, 2),
                            channel(x, y, 3),
                        );
                        // Independent BT.709 matrix in byte units. The
                        // unpremultiplication oracle blends explicit YCbCr;
                        // it does not mirror the shader's affine shortcut.
                        if a == 0.0 {
                            return ([16.0, 128.0, 128.0], a);
                        }
                        let (r, g, b) = (r / a, g / a, b / a);
                        (
                            [
                                16.0 + 219.0 * (0.2126 * r + 0.7152 * g + 0.0722 * b),
                                128.0
                                    + 224.0
                                        * (-0.11457210605734 * r - 0.38542789394266 * g + 0.5 * b),
                                128.0
                                    + 224.0
                                        * (0.5 * r - 0.454152908305817 * g - 0.045847091694183 * b),
                            ],
                            a,
                        )
                    };
                    for y in 0..10 {
                        for x in 0..18 {
                            let (foreground, a) = pixel(x, y);
                            let background = if with_base {
                                base.data(0)[y * base.stride(0) + x] as f64
                            } else {
                                16.0
                            };
                            let expected =
                                (foreground[0] * a + background * (1.0 - a)).round() as i32;
                            let value = actual.data(0)[y * actual.stride(0) + x] as i32;
                            let error = (value - expected).abs();
                            maximum_error = maximum_error.max(error);
                            assert!(
                                error <= i32::from(case >= 6),
                                "Y base={with_base} case={case} ({x},{y}): {value} vs {expected}"
                            );
                            checked += 1;
                        }
                    }
                    for y in 0..5 {
                        for x in 0..9 {
                            for c in 0..2 {
                                let background = if with_base {
                                    base.data(1)[y * base.stride(1) + 2 * x + c] as f64
                                } else {
                                    128.0
                                };
                                let mut expected = 0.0;
                                for dy in 0..2 {
                                    for dx in 0..2 {
                                        let (foreground, a) = pixel(2 * x + dx, 2 * y + dy);
                                        expected +=
                                            (foreground[c + 1] * a + background * (1.0 - a)) * 0.25;
                                    }
                                }
                                let expected = expected.round() as i32;
                                let value = actual.data(1)[y * actual.stride(1) + 2 * x + c] as i32;
                                let error = (value - expected).abs();
                                maximum_error = maximum_error.max(error);
                                assert!(error <= i32::from(case >= 6), "UV base={with_base} case={case} ({x},{y},{c}): {value} vs {expected}");
                                checked += 1;
                            }
                        }
                    }
                }
            }
        }
        compute.drain().unwrap();
        println!("GPU compute oracle: 38 frames, {checked} samples, maximum byte error {maximum_error}; transparent identity exact");
    }
}

#[cfg(test)]
#[path = "compute_srgb_tests.rs"]
mod srgb_tests;
