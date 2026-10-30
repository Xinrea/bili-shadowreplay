//! Minimal Vulkan 1.3 dispatch used by the GPU-frame ownership bridge.
//!
//! These ABI structures match vulkan_core.h. Loading through FFmpeg's resolver
//! keeps every call on the same Vulkan instance/device and avoids a second
//! Vulkan loader or another graphics dependency in ordinary builds.

use ffmpeg_next::ffi;
use std::{
    ffi::{c_void, CStr},
    mem, ptr,
};

type Device = ffi::VkDevice;
type Queue = *mut c_void;
type CommandBuffer = *mut c_void;
type GetDeviceProc = unsafe extern "system" fn(Device, *const std::ffi::c_char) -> *const c_void;
type EnumerateInstanceExtensions =
    unsafe extern "system" fn(*const std::ffi::c_char, *mut u32, *mut c_void) -> i32;
type DeviceWaitIdle = unsafe extern "system" fn(Device) -> i32;
type GetDeviceQueue = unsafe extern "system" fn(Device, u32, u32, *mut Queue);
type GetDeviceQueue2 = unsafe extern "system" fn(Device, *const QueueInfo, *mut Queue);
type WaitSemaphores = unsafe extern "system" fn(Device, *const WaitInfo, u64) -> i32;
type CreateCommandPool =
    unsafe extern "system" fn(Device, *const PoolInfo, *const c_void, *mut u64) -> i32;
type DestroyCommandPool = unsafe extern "system" fn(Device, u64, *const c_void);
type AllocateCommandBuffers =
    unsafe extern "system" fn(Device, *const AllocateInfo, *mut CommandBuffer) -> i32;
type ResetCommandPool = unsafe extern "system" fn(Device, u64, u32) -> i32;
type BeginCommandBuffer = unsafe extern "system" fn(CommandBuffer, *const BeginInfo) -> i32;
type EndCommandBuffer = unsafe extern "system" fn(CommandBuffer) -> i32;
type CmdPipelineBarrier = unsafe extern "system" fn(
    CommandBuffer,
    u32,
    u32,
    u32,
    u32,
    *const c_void,
    u32,
    *const c_void,
    u32,
    *const ImageBarrier,
);
type QueueSubmit = unsafe extern "system" fn(Queue, u32, *const SubmitInfo, u64) -> i32;

#[repr(C)]
struct QueueInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    family: u32,
    index: u32,
}
#[repr(C)]
struct WaitInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    count: u32,
    semaphores: *const u64,
    values: *const u64,
}
#[repr(C)]
struct PoolInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    family: u32,
}
#[repr(C)]
struct AllocateInfo {
    s_type: i32,
    p_next: *const c_void,
    pool: u64,
    level: i32,
    count: u32,
}
#[repr(C)]
struct BeginInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    inheritance: *const c_void,
}
#[repr(C)]
struct TimelineInfo {
    s_type: i32,
    p_next: *const c_void,
    wait_count: u32,
    wait_values: *const u64,
    signal_count: u32,
    signal_values: *const u64,
}
#[repr(C)]
struct SubmitInfo {
    s_type: i32,
    p_next: *const c_void,
    wait_count: u32,
    waits: *const u64,
    wait_stages: *const u32,
    command_count: u32,
    commands: *const CommandBuffer,
    signal_count: u32,
    signals: *const u64,
}
#[repr(C)]
struct ImageRange {
    aspects: u32,
    base_mip: u32,
    level_count: u32,
    base_layer: u32,
    layer_count: u32,
}
#[repr(C)]
struct ImageBarrier {
    s_type: i32,
    p_next: *const c_void,
    src_access: u32,
    dst_access: u32,
    old_layout: i32,
    new_layout: i32,
    src_family: u32,
    dst_family: u32,
    image: u64,
    range: ImageRange,
}

pub(super) struct VulkanSync {
    device: Device,
    instance: ffi::VkInstance,
    instance_proc: ffi::PFN_vkGetInstanceProcAddr,
    device_proc: GetDeviceProc,
    queue: Queue,
    pool: u64,
    command: CommandBuffer,
    wait_idle: DeviceWaitIdle,
    wait_semaphores: WaitSemaphores,
    destroy_pool: DestroyCommandPool,
    reset_pool: ResetCommandPool,
    begin: BeginCommandBuffer,
    end: EndCommandBuffer,
    barrier: CmdPipelineBarrier,
    submit: QueueSubmit,
}

impl VulkanSync {
    pub(super) unsafe fn new(
        context: &ffi::AVVulkanDeviceContext,
        family: u32,
    ) -> Result<Self, String> {
        let resolve = context
            .get_proc_addr
            .ok_or("FFmpeg Vulkan resolver is null")?;
        let get = |name: &CStr| unsafe {
            resolve(context.inst, name.as_ptr()).map_or(ptr::null(), |p| p as *const c_void)
        };
        unsafe fn function<T: Copy>(pointer: *const c_void, name: &str) -> Result<T, String> {
            if pointer.is_null() {
                return Err(format!("Vulkan function {name} is unavailable"));
            }
            assert_eq!(mem::size_of::<T>(), mem::size_of::<*const c_void>());
            Ok(unsafe { mem::transmute_copy(&pointer) })
        }
        // Skia's extension discovery queries this with a null instance and
        // calls the result directly. Validate that exact global dispatch
        // route now, before passing it to a borrowed renderer.
        let global_extensions = unsafe {
            resolve(
                ptr::null_mut(),
                c"vkEnumerateInstanceExtensionProperties".as_ptr(),
            )
            .map_or(ptr::null(), |p| p as *const c_void)
        };
        let enumerate: EnumerateInstanceExtensions = unsafe {
            function(
                global_extensions,
                "vkEnumerateInstanceExtensionProperties (null instance)",
            )?
        };
        let mut extension_count = 0;
        check(
            unsafe { enumerate(ptr::null(), &mut extension_count, ptr::null_mut()) },
            "Query global Vulkan instance extensions",
        )?;
        if get(c"vkEnumerateDeviceExtensionProperties").is_null() {
            return Err(
                "Vulkan function vkEnumerateDeviceExtensionProperties is unavailable".into(),
            );
        }
        // CStr literals are used explicitly below; this dispatch is tiny and
        // all loaded signatures follow Vulkan 1.3's stable C ABI.
        let device_proc: GetDeviceProc =
            unsafe { function(get(c"vkGetDeviceProcAddr"), "vkGetDeviceProcAddr")? };
        let wait_idle: DeviceWaitIdle =
            unsafe { function(get(c"vkDeviceWaitIdle"), "vkDeviceWaitIdle")? };
        let wait_semaphores: WaitSemaphores =
            unsafe { function(get(c"vkWaitSemaphores"), "vkWaitSemaphores")? };
        let destroy_pool: DestroyCommandPool =
            unsafe { function(get(c"vkDestroyCommandPool"), "vkDestroyCommandPool")? };
        let reset_pool: ResetCommandPool =
            unsafe { function(get(c"vkResetCommandPool"), "vkResetCommandPool")? };
        let begin: BeginCommandBuffer =
            unsafe { function(get(c"vkBeginCommandBuffer"), "vkBeginCommandBuffer")? };
        let end: EndCommandBuffer =
            unsafe { function(get(c"vkEndCommandBuffer"), "vkEndCommandBuffer")? };
        let barrier: CmdPipelineBarrier =
            unsafe { function(get(c"vkCmdPipelineBarrier"), "vkCmdPipelineBarrier")? };
        let submit: QueueSubmit = unsafe { function(get(c"vkQueueSubmit"), "vkQueueSubmit")? };
        let create_pool: CreateCommandPool =
            unsafe { function(get(c"vkCreateCommandPool"), "vkCreateCommandPool")? };
        let allocate: AllocateCommandBuffers =
            unsafe { function(get(c"vkAllocateCommandBuffers"), "vkAllocateCommandBuffers")? };
        let mut queue = ptr::null_mut();
        if context.queue_flags == 0 {
            let get_queue: GetDeviceQueue =
                unsafe { function(get(c"vkGetDeviceQueue"), "vkGetDeviceQueue")? };
            unsafe { get_queue(context.act_dev, family, 0, &mut queue) };
        } else {
            let get_queue: GetDeviceQueue2 =
                unsafe { function(get(c"vkGetDeviceQueue2"), "vkGetDeviceQueue2")? };
            let info = QueueInfo {
                s_type: 1_000_145_003,
                p_next: ptr::null(),
                flags: context.queue_flags,
                family,
                index: 0,
            };
            unsafe { get_queue(context.act_dev, &info, &mut queue) };
        }
        if queue.is_null() {
            return Err("FFmpeg Vulkan graphics queue is null".into());
        }
        let info = PoolInfo {
            s_type: 39,
            p_next: ptr::null(),
            flags: 2,
            family,
        };
        let mut pool = 0;
        check(
            unsafe { create_pool(context.act_dev, &info, ptr::null(), &mut pool) },
            "Create Vulkan bridge command pool",
        )?;
        let info = AllocateInfo {
            s_type: 40,
            p_next: ptr::null(),
            pool,
            level: 0,
            count: 1,
        };
        let mut command = ptr::null_mut();
        let result = unsafe { allocate(context.act_dev, &info, &mut command) };
        if result != 0 {
            unsafe { destroy_pool(context.act_dev, pool, ptr::null()) };
            check(result, "Allocate Vulkan bridge command buffer")?;
        }
        Ok(Self {
            device: context.act_dev,
            instance: context.inst,
            instance_proc: context.get_proc_addr,
            device_proc,
            queue,
            pool,
            command,
            wait_idle,
            wait_semaphores,
            destroy_pool,
            reset_pool,
            begin,
            end,
            barrier,
            submit,
        })
    }

    pub(super) fn queue(&self) -> usize {
        self.queue as usize
    }
    pub(super) fn get_proc_addr(&self, device_scope: bool, name: &CStr) -> *const c_void {
        let handle = if device_scope {
            self.device as usize
        } else {
            self.instance as usize
        };
        self.get_proc_addr_for(device_scope, handle, name)
    }
    pub(super) fn get_proc_addr_for(
        &self,
        device_scope: bool,
        handle: usize,
        name: &CStr,
    ) -> *const c_void {
        unsafe {
            if device_scope {
                (self.device_proc)(handle as Device, name.as_ptr())
            } else {
                self.instance_proc.unwrap()(handle as ffi::VkInstance, name.as_ptr())
                    .map_or(ptr::null(), |p| p as *const c_void)
            }
        }
    }
    pub(super) fn wait_idle(&self) -> Result<(), String> {
        check(
            unsafe { (self.wait_idle)(self.device) },
            "Wait for Vulkan device",
        )
    }
    pub(super) fn wait(&self, semaphore: u64, value: u64) -> Result<(), String> {
        let info = WaitInfo {
            s_type: 1_000_207_004,
            p_next: ptr::null(),
            flags: 0,
            count: 1,
            semaphores: &semaphore,
            values: &value,
        };
        // A bounded wait turns a lost/stalled GPU into a failed PoC instead of
        // silently publishing incomplete frames. No host pixel data is read.
        check(
            unsafe { (self.wait_semaphores)(self.device, &info, 30_000_000_000) },
            "Wait for Vulkan frame timeline",
        )
    }

    /// Submit a real image memory barrier, waiting and signalling the frame's
    /// timeline. The caller holds the graphics-queue external-synchronization
    /// lock and waits returned value before reusing this command buffer.
    pub(super) fn transition(
        &self,
        image: u64,
        semaphore: u64,
        value: u64,
        old_layout: u32,
        new_layout: u32,
        src_access: u64,
    ) -> Result<u64, String> {
        let next = value
            .checked_add(1)
            .ok_or("Vulkan frame timeline exhausted")?;
        check(
            unsafe { (self.reset_pool)(self.device, self.pool, 0) },
            "Reset Vulkan bridge command pool",
        )?;
        let begin = BeginInfo {
            s_type: 42,
            p_next: ptr::null(),
            flags: 1,
            inheritance: ptr::null(),
        };
        check(
            unsafe { (self.begin)(self.command, &begin) },
            "Begin Vulkan frame barrier",
        )?;
        let barrier = ImageBarrier {
            s_type: 45,
            p_next: ptr::null(),
            src_access: if old_layout == 0 {
                0
            } else {
                (src_access as u32) | 0x10000
            },
            dst_access: if new_layout == 2 {
                0x80 | 0x100 // COLOR_ATTACHMENT_READ | COLOR_ATTACHMENT_WRITE
            } else {
                0x8000 | 0x10000 // MEMORY_READ | MEMORY_WRITE
            },
            old_layout: old_layout as i32,
            new_layout: new_layout as i32,
            src_family: u32::MAX,
            dst_family: u32::MAX,
            image,
            range: ImageRange {
                aspects: 1,
                base_mip: 0,
                level_count: 1,
                base_layer: 0,
                layer_count: 1,
            },
        };
        unsafe {
            (self.barrier)(
                self.command,
                0x10000,
                0x10000,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                &barrier,
            )
        };
        check(
            unsafe { (self.end)(self.command) },
            "End Vulkan frame barrier",
        )?;
        let timeline = TimelineInfo {
            s_type: 1_000_207_003,
            p_next: ptr::null(),
            wait_count: 1,
            wait_values: &value,
            signal_count: 1,
            signal_values: &next,
        };
        let stages = 0x10000;
        let submit = SubmitInfo {
            s_type: 4,
            p_next: (&timeline as *const TimelineInfo).cast(),
            wait_count: 1,
            waits: &semaphore,
            wait_stages: &stages,
            command_count: 1,
            commands: &self.command,
            signal_count: 1,
            signals: &semaphore,
        };
        check(
            unsafe { (self.submit)(self.queue, 1, &submit, 0) },
            "Submit Vulkan frame barrier",
        )?;
        Ok(next)
    }
}
impl Drop for VulkanSync {
    fn drop(&mut self) {
        unsafe { (self.destroy_pool)(self.device, self.pool, ptr::null()) }
    }
}
fn check(value: i32, operation: &str) -> Result<(), String> {
    if value == 0 {
        Ok(())
    } else {
        Err(format!("{operation}: VkResult {value}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vulkan_bridge_structures_match_the_64_bit_c_abi() {
        assert_eq!(mem::size_of::<QueueInfo>(), 32);
        assert_eq!(mem::size_of::<WaitInfo>(), 40);
        assert_eq!(mem::size_of::<PoolInfo>(), 24);
        assert_eq!(mem::size_of::<AllocateInfo>(), 32);
        assert_eq!(mem::size_of::<BeginInfo>(), 32);
        assert_eq!(mem::size_of::<TimelineInfo>(), 48);
        assert_eq!(mem::size_of::<SubmitInfo>(), 72);
        assert_eq!(mem::size_of::<ImageBarrier>(), 72);
    }
}
