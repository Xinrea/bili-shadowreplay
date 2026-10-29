#define VK_NO_PROTOTYPES
#include <vulkan/vulkan.h>
static_assert(VK_HEADER_VERSION_COMPLETE >= VK_MAKE_API_VERSION(0, 1, 3, 277), "Vulkan export requires headers >= 1.3.277");
#include "pipeline.h"
#include "shader.h"

namespace {
static void vkcheck(VkResult value, const char* message) {
    if (value != VK_SUCCESS) throw std::runtime_error(std::string(message) + ": " + std::to_string(value));
}
#define DEVICE_FUNCTIONS(X) \
    X(GetDeviceQueue2) X(CreateCommandPool) X(DestroyCommandPool) X(AllocateCommandBuffers) \
    X(ResetCommandBuffer) X(BeginCommandBuffer) X(EndCommandBuffer) \
    X(CreateFence) X(DestroyFence) X(WaitForFences) X(ResetFences) X(QueueSubmit) X(DeviceWaitIdle) \
    X(CreateImageView) X(DestroyImageView) X(CreateShaderModule) X(DestroyShaderModule) \
    X(CreateDescriptorSetLayout) X(DestroyDescriptorSetLayout) X(CreateDescriptorPool) \
    X(DestroyDescriptorPool) X(AllocateDescriptorSets) X(UpdateDescriptorSets) \
    X(CreatePipelineLayout) X(DestroyPipelineLayout) X(CreateComputePipelines) X(DestroyPipeline) \
    X(CmdPipelineBarrier) X(CmdBindPipeline) X(CmdBindDescriptorSets) X(CmdPushConstants) X(CmdDispatch)
struct VulkanAPI {
    PFN_vkGetInstanceProcAddr instance = nullptr;
    PFN_vkGetDeviceProcAddr device = nullptr;
#define DECLARE(name) PFN_vk##name name = nullptr;
    DEVICE_FUNCTIONS(DECLARE)
#undef DECLARE
    void load(AVVulkanDeviceContext* ctx) {
        instance = ctx->get_proc_addr;
        require(instance != nullptr, "Vulkan loader unavailable");
        device = reinterpret_cast<PFN_vkGetDeviceProcAddr>(instance(ctx->inst, "vkGetDeviceProcAddr"));
        require(device != nullptr, "Vulkan device resolver unavailable");
#define LOAD(name) name = reinterpret_cast<PFN_vk##name>(device(ctx->act_dev, "vk" #name)); require(name != nullptr, "Missing vk" #name);
        DEVICE_FUNCTIONS(LOAD)
#undef LOAD
    }
};
static const void* resolve(void* opaque, const char* name, void* instance, void* device) {
    auto api = static_cast<VulkanAPI*>(opaque);
    return device ? reinterpret_cast<const void*>(api->device(static_cast<VkDevice>(device), name))
        : reinterpret_cast<const void*>(api->instance(static_cast<VkInstance>(instance), name));
}
struct Slot {
    Frame overlay, source, output;
    VkCommandBuffer command = VK_NULL_HANDLE;
    VkDescriptorSet descriptors = VK_NULL_HANDLE;
    VkFence fence = VK_NULL_HANDLE;
    std::array<VkImageView, 5> views{};
    bool submitted = false;
};

// Skia and compute use one graphics+compute queue. FFmpeg owns the frame pools
// and each AVVkFrame's timeline semaphore, including the encoder's final use.
class VulkanCompositor {
    AVBufferRef *deviceRef = nullptr, *videoPool = nullptr, *overlayPool = nullptr;
    AVHWDeviceContext* deviceContext = nullptr;
    AVVulkanDeviceContext* context = nullptr;
    VulkanAPI vk;
    VkQueue queue = VK_NULL_HANDLE;
    uint32_t family = 0;
    VkCommandPool commands = VK_NULL_HANDLE;
    VkDescriptorSetLayout descriptorLayout = VK_NULL_HANDLE;
    VkDescriptorPool descriptorPool = VK_NULL_HANDLE;
    VkPipelineLayout pipelineLayout = VK_NULL_HANDLE;
    VkPipeline pipeline = VK_NULL_HANDLE;
    void* renderer = nullptr;
    std::array<Slot, FRAME_SLOTS> slots;
    int width = 0, height = 0, videoWidth = 0, videoHeight = 0;

    AVBufferRef* pool(AVPixelFormat format) {
        AVBufferRef* ref = av_hwframe_ctx_alloc(deviceRef);
        require(ref != nullptr, "allocate Vulkan frame pool");
        auto frames = reinterpret_cast<AVHWFramesContext*>(ref->data);
        frames->format = AV_PIX_FMT_VULKAN; frames->sw_format = format;
        frames->width = format == AV_PIX_FMT_NV12 ? videoWidth : width;
        frames->height = format == AV_PIX_FMT_NV12 ? videoHeight : height;
        auto settings = static_cast<AVVulkanFramesContext*>(frames->hwctx);
        settings->flags = AV_VK_FRAME_FLAG_NONE;
        settings->format[0] = format == AV_PIX_FMT_NV12 ? VK_FORMAT_G8_B8R8_2PLANE_420_UNORM : VK_FORMAT_R8G8B8A8_UNORM;
        if (format == AV_PIX_FMT_RGBA) settings->usage = static_cast<VkImageUsageFlagBits>(
            VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_STORAGE_BIT | VK_IMAGE_USAGE_SAMPLED_BIT |
            VK_IMAGE_USAGE_TRANSFER_SRC_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT);
        int result = av_hwframe_ctx_init(ref);
        if (result < 0) { av_buffer_unref(&ref); check(result, "initialize Vulkan frame pool"); }
        const auto required = VK_IMAGE_USAGE_STORAGE_BIT | (format == AV_PIX_FMT_NV12 ? VK_IMAGE_USAGE_VIDEO_ENCODE_SRC_BIT_KHR : 0);
        if ((settings->usage & required) != required) { av_buffer_unref(&ref); throw std::runtime_error("Vulkan frame pool lacks storage or video encode usage"); }
        return ref;
    }
    Frame allocate(AVBufferRef* poolRef) {
        auto f = frame(); check(av_hwframe_get_buffer(poolRef, f.get(), 0), "allocate Vulkan frame");
        f->width = width; f->height = height; return f;
    }
    void retire(Slot& slot) {
        if (slot.submitted) {
            vkcheck(vk.WaitForFences(context->act_dev, 1, &slot.fence, VK_TRUE, UINT64_MAX), "retire Vulkan frame");
            slot.submitted = false;
        }
        slot.source.reset(); slot.output.reset();
        for (auto& view : slot.views) if (view) { vk.DestroyImageView(context->act_dev, view, nullptr); view = VK_NULL_HANDLE; }
    }
    SkiaVulkanImage skiaImage(AVFrame* frame) {
        auto f = reinterpret_cast<AVVkFrame*>(frame->data[0]);
        return {reinterpret_cast<uint64_t>(f->img[0]), reinterpret_cast<uint64_t>(f->mem[0]),
            static_cast<uint64_t>(f->offset[0]), f->size[0], static_cast<uint32_t>(f->layout[0]), f->queue_family[0]};
    }
    VkImageView view(AVFrame* frame, int plane, bool rgba = false) {
        auto f = reinterpret_cast<AVVkFrame*>(frame->data[0]);
        require(rgba || f->img[1] == VK_NULL_HANDLE, "Vulkan Video requires a multiplane NV12 image");
        VkImageViewCreateInfo info{VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO};
        VkImageViewUsageCreateInfo usage{VK_STRUCTURE_TYPE_IMAGE_VIEW_USAGE_CREATE_INFO};
        usage.usage = VK_IMAGE_USAGE_STORAGE_BIT; info.pNext = &usage;
        info.image = f->img[0]; info.viewType = VK_IMAGE_VIEW_TYPE_2D;
        info.format = rgba ? VK_FORMAT_R8G8B8A8_UNORM : (plane ? VK_FORMAT_R8G8_UNORM : VK_FORMAT_R8_UNORM);
        info.subresourceRange = {rgba ? VK_IMAGE_ASPECT_COLOR_BIT : (plane ? VK_IMAGE_ASPECT_PLANE_1_BIT : VK_IMAGE_ASPECT_PLANE_0_BIT), 0, 1, 0, 1};
        VkImageView result; vkcheck(vk.CreateImageView(context->act_dev, &info, nullptr, &result), "create image plane view"); return result;
    }
public:
    VulkanCompositor(int w, int h, const char* scene) { try { init(w, h, scene); } catch (...) { cleanup(); throw; } }
    VulkanCompositor(const VulkanCompositor&) = delete;
    VulkanCompositor& operator=(const VulkanCompositor&) = delete;
    ~VulkanCompositor() { cleanup(); }
    void cleanup() {
        if (context && vk.DeviceWaitIdle) vk.DeviceWaitIdle(context->act_dev);
        if (renderer) bsr_skia_free(renderer);
        if (context && vk.DestroyImageView) for (auto& slot : slots) {
            slot.submitted = false; retire(slot);
            if (slot.fence) vk.DestroyFence(context->act_dev, slot.fence, nullptr);
            slot.overlay.reset();
        }
        if (pipeline) vk.DestroyPipeline(context->act_dev, pipeline, nullptr);
        if (pipelineLayout) vk.DestroyPipelineLayout(context->act_dev, pipelineLayout, nullptr);
        if (descriptorPool) vk.DestroyDescriptorPool(context->act_dev, descriptorPool, nullptr);
        if (descriptorLayout) vk.DestroyDescriptorSetLayout(context->act_dev, descriptorLayout, nullptr);
        if (commands) vk.DestroyCommandPool(context->act_dev, commands, nullptr);
        av_buffer_unref(&overlayPool); av_buffer_unref(&videoPool); av_buffer_unref(&deviceRef);
    }
    void init(int w, int h, const char* scene) {
        width = videoWidth = w; height = videoHeight = h;
        check(av_hwdevice_ctx_create(&deviceRef, AV_HWDEVICE_TYPE_VULKAN, nullptr, nullptr, 0), "create Vulkan device");
        deviceContext = reinterpret_cast<AVHWDeviceContext*>(deviceRef->data);
        context = static_cast<AVVulkanDeviceContext*>(deviceContext->hwctx);
        require(context->device_features.features.shaderStorageImageReadWithoutFormat && context->device_features.features.shaderStorageImageWriteWithoutFormat,
            "Vulkan device requires storage-image access without format");
        vk.load(context);
        auto properties = reinterpret_cast<PFN_vkGetPhysicalDeviceProperties>(vk.instance(context->inst, "vkGetPhysicalDeviceProperties"));
        VkPhysicalDeviceProperties propertiesValue{}; properties(context->phys_dev, &propertiesValue);
        require(propertiesValue.deviceType == VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU || propertiesValue.deviceType == VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU,
            "A hardware Vulkan GPU is required; software Vulkan is not supported");
        auto formatProperties = reinterpret_cast<PFN_vkGetPhysicalDeviceFormatProperties2>(vk.instance(context->inst, "vkGetPhysicalDeviceFormatProperties2"));
        for (auto format : {VK_FORMAT_R8_UNORM, VK_FORMAT_R8G8_UNORM}) {
            VkFormatProperties3 extended{VK_STRUCTURE_TYPE_FORMAT_PROPERTIES_3};
            VkFormatProperties2 properties{VK_STRUCTURE_TYPE_FORMAT_PROPERTIES_2}; properties.pNext = &extended;
            formatProperties(context->phys_dev, format, &properties);
            const auto required = VK_FORMAT_FEATURE_2_STORAGE_IMAGE_BIT | VK_FORMAT_FEATURE_2_STORAGE_READ_WITHOUT_FORMAT_BIT | VK_FORMAT_FEATURE_2_STORAGE_WRITE_WITHOUT_FORMAT_BIT;
            require((extended.optimalTilingFeatures & required) == required, "Vulkan NV12 planes require storage-image access without format");
        }
        auto queueProperties = reinterpret_cast<PFN_vkGetPhysicalDeviceQueueFamilyProperties>(vk.instance(context->inst, "vkGetPhysicalDeviceQueueFamilyProperties"));
        uint32_t count = 0; queueProperties(context->phys_dev, &count, nullptr);
        std::vector<VkQueueFamilyProperties> families(count); queueProperties(context->phys_dev, &count, families.data());
        bool found = false;
        for (int i = 0; i < context->nb_qf; ++i) {
            auto index = context->qf[i].idx;
            if (index >= 0 && context->qf[i].num > 0 &&
                (families[index].queueFlags & (VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_COMPUTE_BIT)) == (VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_COMPUTE_BIT)) {
                family = index; found = true; break;
            }
        }
        require(found, "Vulkan device has no enabled graphics+compute queue");
        VkDeviceQueueInfo2 queueInfo{VK_STRUCTURE_TYPE_DEVICE_QUEUE_INFO_2};
        queueInfo.queueFamilyIndex = family;
#if LIBAVUTIL_VERSION_MAJOR >= 61
        queueInfo.flags = context->queue_flags;
#endif
        vk.GetDeviceQueue2(context->act_dev, &queueInfo, &queue);
        videoPool = pool(AV_PIX_FMT_NV12); overlayPool = pool(AV_PIX_FMT_RGBA);
        VkCommandPoolCreateInfo commandInfo{VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO};
        commandInfo.flags = VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT; commandInfo.queueFamilyIndex = family;
        vkcheck(vk.CreateCommandPool(context->act_dev, &commandInfo, nullptr, &commands), "create command pool");
        std::array<VkDescriptorSetLayoutBinding, 5> bindings{};
        for (uint32_t i = 0; i < bindings.size(); ++i) bindings[i] = {i, VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, 1, VK_SHADER_STAGE_COMPUTE_BIT, nullptr};
        VkDescriptorSetLayoutCreateInfo layoutInfo{VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO};
        layoutInfo.bindingCount = bindings.size(); layoutInfo.pBindings = bindings.data();
        vkcheck(vk.CreateDescriptorSetLayout(context->act_dev, &layoutInfo, nullptr, &descriptorLayout), "create descriptor layout");
        VkDescriptorPoolSize size{VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, 15};
        VkDescriptorPoolCreateInfo poolInfo{VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO}; poolInfo.maxSets = 3; poolInfo.poolSizeCount = 1; poolInfo.pPoolSizes = &size;
        vkcheck(vk.CreateDescriptorPool(context->act_dev, &poolInfo, nullptr, &descriptorPool), "create descriptor pool");
        VkPipelineLayoutCreateInfo pipelineInfo{VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO}; pipelineInfo.setLayoutCount = 1; pipelineInfo.pSetLayouts = &descriptorLayout;
        VkPushConstantRange dimensions{VK_SHADER_STAGE_COMPUTE_BIT, 0, 2 * sizeof(int32_t)};
        pipelineInfo.pushConstantRangeCount = 1; pipelineInfo.pPushConstantRanges = &dimensions;
        vkcheck(vk.CreatePipelineLayout(context->act_dev, &pipelineInfo, nullptr, &pipelineLayout), "create pipeline layout");
        VkShaderModuleCreateInfo shaderInfo{VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO}; shaderInfo.codeSize = sizeof(COMPOSITE_SHADER); shaderInfo.pCode = COMPOSITE_SHADER;
        VkShaderModule shader; vkcheck(vk.CreateShaderModule(context->act_dev, &shaderInfo, nullptr, &shader), "create compositor shader");
        VkComputePipelineCreateInfo compute{VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO};
        compute.stage = {VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO, nullptr, 0, VK_SHADER_STAGE_COMPUTE_BIT, shader, "main", nullptr}; compute.layout = pipelineLayout;
        auto result = vk.CreateComputePipelines(context->act_dev, VK_NULL_HANDLE, 1, &compute, nullptr, &pipeline);
        vk.DestroyShaderModule(context->act_dev, shader, nullptr); vkcheck(result, "create compositor pipeline");
        for (auto& slot : slots) {
            slot.overlay = allocate(overlayPool);
            VkFenceCreateInfo fence{VK_STRUCTURE_TYPE_FENCE_CREATE_INFO};
            vkcheck(vk.CreateFence(context->act_dev, &fence, nullptr, &slot.fence), "create frame fence");
            VkCommandBufferAllocateInfo cmd{VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO}; cmd.commandPool = commands; cmd.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY; cmd.commandBufferCount = 1;
            vkcheck(vk.AllocateCommandBuffers(context->act_dev, &cmd, &slot.command), "allocate frame commands");
            VkDescriptorSetAllocateInfo set{VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO}; set.descriptorPool = descriptorPool; set.descriptorSetCount = 1; set.pSetLayouts = &descriptorLayout;
            vkcheck(vk.AllocateDescriptorSets(context->act_dev, &set, &slot.descriptors), "allocate frame descriptors");
        }
        SkiaVulkanContext skia{context->inst, context->phys_dev, context->act_dev, queue, family,
            static_cast<uint32_t>(w), static_cast<uint32_t>(h), &vk, resolve,
            context->enabled_inst_extensions, static_cast<uint32_t>(context->nb_enabled_inst_extensions),
            context->enabled_dev_extensions, static_cast<uint32_t>(context->nb_enabled_dev_extensions)};
        auto image = skiaImage(slots[0].overlay.get());
        renderer = bsr_skia_vulkan_new(scene, &skia, &image);
        require(renderer != nullptr, "create Skia Vulkan renderer");
    }
    Codec encoder(const ExportConfig& config, AVRational fps) {
        auto enc = videoEncoder(config.hevc ? "hevc_vulkan" : "h264_vulkan", config, width, height, fps);
        enc->pix_fmt = AV_PIX_FMT_VULKAN; enc->hw_frames_ctx = av_buffer_ref(videoPool);
        applyEncoderOptions(enc.get(), config);
        check(avcodec_open2(enc.get(), enc->codec, nullptr), "open Vulkan Video encoder");
        if (enc->coded_width > videoWidth || enc->coded_height > videoHeight) {
            videoWidth = enc->coded_width; videoHeight = enc->coded_height;
            enc.reset(); av_buffer_unref(&videoPool); videoPool = pool(AV_PIX_FMT_NV12);
            return encoder(config, fps);
        }
        return enc;
    }
    int qualityMax(AVCodecContext* enc) {
        const bool hevc = enc->codec_id == AV_CODEC_ID_HEVC;
        VkVideoEncodeH264ProfileInfoKHR h264{VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_PROFILE_INFO_KHR};
        h264.stdProfileIdc = static_cast<StdVideoH264ProfileIdc>(enc->profile & ~(AV_PROFILE_H264_CONSTRAINED | AV_PROFILE_H264_INTRA));
        VkVideoEncodeH265ProfileInfoKHR h265{VK_STRUCTURE_TYPE_VIDEO_ENCODE_H265_PROFILE_INFO_KHR};
        h265.stdProfileIdc = static_cast<StdVideoH265ProfileIdc>(enc->profile);
        VkVideoProfileInfoKHR profile{VK_STRUCTURE_TYPE_VIDEO_PROFILE_INFO_KHR};
        profile.pNext = hevc ? static_cast<void*>(&h265) : static_cast<void*>(&h264);
        profile.videoCodecOperation = hevc ? VK_VIDEO_CODEC_OPERATION_ENCODE_H265_BIT_KHR : VK_VIDEO_CODEC_OPERATION_ENCODE_H264_BIT_KHR;
        profile.chromaSubsampling = VK_VIDEO_CHROMA_SUBSAMPLING_420_BIT_KHR;
        profile.lumaBitDepth = profile.chromaBitDepth = VK_VIDEO_COMPONENT_BIT_DEPTH_8_BIT_KHR;
        VkVideoEncodeH264CapabilitiesKHR h264Caps{VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_CAPABILITIES_KHR};
        VkVideoEncodeH265CapabilitiesKHR h265Caps{VK_STRUCTURE_TYPE_VIDEO_ENCODE_H265_CAPABILITIES_KHR};
        VkVideoEncodeCapabilitiesKHR encodeCaps{VK_STRUCTURE_TYPE_VIDEO_ENCODE_CAPABILITIES_KHR};
        encodeCaps.pNext = hevc ? static_cast<void*>(&h265Caps) : static_cast<void*>(&h264Caps);
        VkVideoCapabilitiesKHR caps{VK_STRUCTURE_TYPE_VIDEO_CAPABILITIES_KHR}; caps.pNext = &encodeCaps;
        auto getCaps = reinterpret_cast<PFN_vkGetPhysicalDeviceVideoCapabilitiesKHR>(vk.instance(context->inst, "vkGetPhysicalDeviceVideoCapabilitiesKHR"));
        require(getCaps != nullptr, "Vulkan video capabilities unavailable");
        vkcheck(getCaps(context->phys_dev, &profile, &caps), "query encoder quality levels");
        require(encodeCaps.maxQualityLevels > 0, "Vulkan encoder has no quality levels");
        return static_cast<int>(encodeCaps.maxQualityLevels - 1);
    }
    void submit(size_t index, AVFrame* cpu, double time) {
        auto& slot = slots[index]; retire(slot);
        slot.source = allocate(videoPool);
        // This is the sole full-frame host transfer: software decode -> GPU.
        check(av_hwframe_transfer_data(slot.source.get(), cpu, 0), "upload decoded NV12");
        auto output = allocate(videoPool);
        auto base = reinterpret_cast<AVVkFrame*>(slot.source->data[0]);
        auto overlay = reinterpret_cast<AVVkFrame*>(slot.overlay->data[0]);
        auto out = reinterpret_cast<AVVkFrame*>(output->data[0]);
        struct Lock {
            VulkanCompositor& gpu;
            explicit Lock(VulkanCompositor& g) : gpu(g) { g.context->lock_queue(g.deviceContext, g.family, 0); }
            ~Lock() { gpu.context->unlock_queue(gpu.deviceContext, gpu.family, 0); }
        } lock(*this);
        // Allocation can schedule an initial layout transition on FFmpeg's
        // compute queue. Make Skia's queue wait on it without a host wait.
        VkTimelineSemaphoreSubmitInfo ready{VK_STRUCTURE_TYPE_TIMELINE_SEMAPHORE_SUBMIT_INFO};
        ready.waitSemaphoreValueCount = 1; ready.pWaitSemaphoreValues = &overlay->sem_value[0];
        VkPipelineStageFlags readyStage = VK_PIPELINE_STAGE_ALL_COMMANDS_BIT;
        VkSubmitInfo wait{VK_STRUCTURE_TYPE_SUBMIT_INFO}; wait.pNext = &ready;
        wait.waitSemaphoreCount = 1; wait.pWaitSemaphores = &overlay->sem[0]; wait.pWaitDstStageMask = &readyStage;
        vkcheck(vk.QueueSubmit(queue, 1, &wait, VK_NULL_HANDLE), "wait for Skia image preparation");
        auto image = skiaImage(slot.overlay.get());
        require(bsr_skia_vulkan_draw(renderer, time, &image) == 0, "draw Skia Vulkan frame");
        overlay->layout[0] = VK_IMAGE_LAYOUT_GENERAL; overlay->access[0] = VK_ACCESS_MEMORY_WRITE_BIT;
        slot.views[0] = view(slot.source.get(), 0); slot.views[1] = view(slot.source.get(), 1);
        slot.views[2] = view(slot.overlay.get(), 0, true);
        slot.views[3] = view(output.get(), 0); slot.views[4] = view(output.get(), 1);
        std::array<VkDescriptorImageInfo, 5> images{}; std::array<VkWriteDescriptorSet, 5> writes{};
        for (uint32_t i = 0; i < images.size(); ++i) {
            images[i].imageView = slot.views[i]; images[i].imageLayout = VK_IMAGE_LAYOUT_GENERAL;
            writes[i] = {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, nullptr, slot.descriptors, i, 0, 1, VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, &images[i], nullptr, nullptr};
        }
        vk.UpdateDescriptorSets(context->act_dev, writes.size(), writes.data(), 0, nullptr);
        vkcheck(vk.ResetCommandBuffer(slot.command, 0), "reset compositor commands");
        VkCommandBufferBeginInfo begin{VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO}; begin.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;
        vkcheck(vk.BeginCommandBuffer(slot.command, &begin), "begin compositor commands");
        std::array<AVVkFrame*, 3> frames{base, overlay, out}; std::array<VkImageMemoryBarrier, 3> barriers{};
        std::array<VkSemaphore, 3> semaphores{}; std::array<uint64_t, 3> waits{}, signals{};
        std::array<VkPipelineStageFlags, 3> stages{};
        for (size_t i = 0; i < frames.size(); ++i) {
            auto f = frames[i];
            require(f->queue_family[0] == VK_QUEUE_FAMILY_IGNORED || f->queue_family[0] == family, "unsupported exclusive Vulkan queue ownership");
            auto& b = barriers[i]; b.sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER;
            b.srcAccessMask = f->access[0]; b.dstAccessMask = i == 2 ? VK_ACCESS_SHADER_WRITE_BIT : VK_ACCESS_SHADER_READ_BIT;
            b.oldLayout = f->layout[0]; b.newLayout = VK_IMAGE_LAYOUT_GENERAL;
            b.srcQueueFamilyIndex = b.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
            b.image = f->img[0]; b.subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
            semaphores[i] = f->sem[0]; waits[i] = f->sem_value[0]; signals[i] = waits[i] + 1; stages[i] = VK_PIPELINE_STAGE_ALL_COMMANDS_BIT;
        }
        vk.CmdPipelineBarrier(slot.command, VK_PIPELINE_STAGE_ALL_COMMANDS_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, 0, 0, nullptr, 0, nullptr, barriers.size(), barriers.data());
        vk.CmdBindPipeline(slot.command, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline);
        vk.CmdBindDescriptorSets(slot.command, VK_PIPELINE_BIND_POINT_COMPUTE, pipelineLayout, 0, 1, &slot.descriptors, 0, nullptr);
        int32_t dimensions[2] = {width, height};
        vk.CmdPushConstants(slot.command, pipelineLayout, VK_SHADER_STAGE_COMPUTE_BIT, 0, sizeof(dimensions), dimensions);
        vk.CmdDispatch(slot.command, (videoWidth / 2 + 15) / 16, (videoHeight / 2 + 15) / 16, 1);
        vkcheck(vk.EndCommandBuffer(slot.command), "end compositor commands");
        VkTimelineSemaphoreSubmitInfo timeline{VK_STRUCTURE_TYPE_TIMELINE_SEMAPHORE_SUBMIT_INFO};
        timeline.waitSemaphoreValueCount = waits.size(); timeline.pWaitSemaphoreValues = waits.data();
        timeline.signalSemaphoreValueCount = signals.size(); timeline.pSignalSemaphoreValues = signals.data();
        VkSubmitInfo submit{VK_STRUCTURE_TYPE_SUBMIT_INFO}; submit.pNext = &timeline;
        submit.waitSemaphoreCount = semaphores.size(); submit.pWaitSemaphores = semaphores.data(); submit.pWaitDstStageMask = stages.data();
        submit.commandBufferCount = 1; submit.pCommandBuffers = &slot.command;
        submit.signalSemaphoreCount = semaphores.size(); submit.pSignalSemaphores = semaphores.data();
        vkcheck(vk.ResetFences(context->act_dev, 1, &slot.fence), "reset frame fence");
        vkcheck(vk.QueueSubmit(queue, 1, &submit, slot.fence), "submit Vulkan compositor"); slot.submitted = true;
        for (size_t i = 0; i < frames.size(); ++i) {
            frames[i]->sem_value[0] = signals[i]; frames[i]->layout[0] = VK_IMAGE_LAYOUT_GENERAL;
            frames[i]->access[0] = static_cast<VkAccessFlagBits>(i == 2 ? VK_ACCESS_SHADER_WRITE_BIT : VK_ACCESS_SHADER_READ_BIT);
        }
        check(av_frame_copy_props(output.get(), cpu), "copy frame metadata");
        output->pts = cpu->pts; output->duration = 1; output->time_base = cpu->time_base;
        output->pict_type = AV_PICTURE_TYPE_NONE; output->flags &= ~AV_FRAME_FLAG_KEY;
        slot.output = std::move(output);
    }
    Frame take(size_t index) { return std::move(slots[index].output); }
};

} // namespace

extern "C" const char* bsr_ffmpeg_load(const char* directory) { return loadFFmpeg(directory); }
extern "C" int bsr_gpu_export(const ExportConfig* config, char* error, size_t capacity) {
    return exportVideo<VulkanCompositor>(config, error, capacity);
}
extern "C" int bsr_gpu_probe(char* error, size_t capacity, int32_t* qualityMax) {
    return probeCodecs<VulkanCompositor>(error, capacity, qualityMax);
}
