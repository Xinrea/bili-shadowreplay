#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#import <CoreVideo/CoreVideo.h>
#include <pthread.h>
#define BSR_METAL
#include "pipeline.h"
#include "shader.h"

namespace {
static std::string str(NSString* text) { return text ? text.UTF8String : ""; }
static void releasePixel(void*, uint8_t* pixel) {
    CVPixelBufferRelease(reinterpret_cast<CVPixelBufferRef>(pixel));
}

// The M4 Pro experiment improved hardware-encode throughput with one active CPU
// thread. Retain that scheduling aid on hosts with >=12 logical CPUs: it burns
// one CPU, does no video work, and must stop with this export, including errors.
class CPUActivity {
    std::atomic<bool> stopped{false};
    std::thread worker;
public:
    explicit CPUActivity(unsigned logicalCPUs) {
        if (logicalCPUs < 12) return;
        worker = std::thread([this] {
            pthread_setname_np("bsr.cpu_activity");
            pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0);
            volatile uint64_t value = 1;
            while (!stopped.load(std::memory_order_relaxed) &&
                   !interrupted.load(std::memory_order_relaxed)) {
                for (int i = 0; i < 2048; ++i) value = value * 1664525 + 1013904223;
            }
        });
    }
    ~CPUActivity() {
        stopped.store(true, std::memory_order_relaxed);
        if (worker.joinable()) worker.join();
    }
};

static int bFramesFor(AVCodecID codec) {
    // HEVC reordering improved quality in the local matched-input test. H.264
    // currently emits DTS > PTS with the supported FFmpeg 8 VideoToolbox backend:
    // https://code.ffmpeg.org/FFmpeg/FFmpeg/pulls/24807
    return codec == AV_CODEC_ID_HEVC ? 2 : 0;
}

struct FrameSlot {
    __strong id<MTLTexture> overlay;
    __strong id<MTLCommandBuffer> command;
    CVMetalTextureRef views[4] = {};
    AVFrame* frame = nullptr;
    CVPixelBufferRef uploadedSource = nullptr;
};

class MetalCompositor {
public:
    int width, height;
    __strong id<MTLDevice> device;
    __strong id<MTLCommandQueue> queue;
    __strong id<MTLComputePipelineState> pipeline;
    CVMetalTextureCacheRef textureCache = nullptr;
    CVPixelBufferPoolRef pool = nullptr;
    std::array<FrameSlot, FRAME_SLOTS> slots;
    AVBufferRef *deviceRef = nullptr, *encodeFrames = nullptr;
    void* renderer = nullptr;
    CPUActivity cpuActivity;

    MetalCompositor(int w, int h, const char* scene) : width(w), height(h), cpuActivity(scene ? std::thread::hardware_concurrency() : 0) {
      try {
        device = MTLCreateSystemDefaultDevice();
        require(device != nil, "Metal device");
        queue = [device newCommandQueue];
        require(queue != nil, "Metal queue");
        NSError* error = nil;
        NSString* source = [NSString stringWithUTF8String:COMPOSITE_SHADER];
        require(source != nil, "read shader: " + str(error.localizedDescription));
        MTLCompileOptions* options = [MTLCompileOptions new];
        options.languageVersion = MTLLanguageVersion2_4;
        id<MTLLibrary> library = [device newLibraryWithSource:source options:options error:&error];
        require(library != nil, "compile shader: " + str(error.localizedDescription));
        pipeline = [device newComputePipelineStateWithFunction:[library newFunctionWithName:@"composite_nv12"] error:&error];
        require(pipeline != nil, "compute pipeline: " + str(error.localizedDescription));
        require(CVMetalTextureCacheCreate(nullptr, nullptr, device, nullptr, &textureCache) == kCVReturnSuccess,
                "CVMetalTextureCacheCreate");
        NSDictionary* attributes = @{
            (id)kCVPixelBufferWidthKey: @(w), (id)kCVPixelBufferHeightKey: @(h),
            (id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange),
            (id)kCVPixelBufferMetalCompatibilityKey: @YES,
            (id)kCVPixelBufferIOSurfacePropertiesKey: @{}
        };
        require(CVPixelBufferPoolCreate(nullptr, nullptr, (__bridge CFDictionaryRef)attributes, &pool) == kCVReturnSuccess,
                "CVPixelBufferPoolCreate");
        auto descriptor = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatBGRA8Unorm
                                                                          width:w height:h mipmapped:NO];
        descriptor.storageMode = MTLStorageModePrivate;
        descriptor.usage = MTLTextureUsageRenderTarget | MTLTextureUsageShaderRead;
        for (auto& slot : slots) {
            slot.overlay = [device newTextureWithDescriptor:descriptor];
            require(slot.overlay != nil, "Skia render texture");
        }
        check(av_hwdevice_ctx_create(&deviceRef, AV_HWDEVICE_TYPE_VIDEOTOOLBOX, nullptr, nullptr, 0), "create VideoToolbox device");
        encodeFrames = av_hwframe_ctx_alloc(deviceRef); require(encodeFrames != nullptr, "allocate hardware frames");
        auto frames = reinterpret_cast<AVHWFramesContext*>(encodeFrames->data);
        frames->format = AV_PIX_FMT_VIDEOTOOLBOX; frames->sw_format = AV_PIX_FMT_NV12; frames->width = w; frames->height = h;
        static_cast<AVVTFramesContext*>(frames->hwctx)->color_range = AVCOL_RANGE_MPEG;
        check(av_hwframe_ctx_init(encodeFrames), "initialize hardware frames");
        renderer = bsr_skia_metal_new(scene, w, h, (__bridge void*)device, (__bridge void*)queue, (__bridge void*)slots[0].overlay);
        require(renderer != nullptr, "initialize Skia Metal");
      } catch (...) { cleanup(); throw; }
    }
    MetalCompositor(const MetalCompositor&) = delete;
    MetalCompositor& operator=(const MetalCompositor&) = delete;
    ~MetalCompositor() { cleanup(); }
    void cleanup() {
        for (auto& slot : slots) {
            if (slot.command) [slot.command waitUntilCompleted];
            release(slot);
        }
        if (renderer) bsr_skia_free(renderer);
        av_buffer_unref(&encodeFrames); av_buffer_unref(&deviceRef);
        if (pool) CVPixelBufferPoolRelease(pool);
        if (textureCache) CFRelease(textureCache);
    }
    void release(FrameSlot& slot) {
        av_frame_free(&slot.frame);
        for (auto& view : slot.views) {
            if (view) { CFRelease(view); view = nullptr; }
        }
        if(slot.uploadedSource){CVPixelBufferRelease(slot.uploadedSource);slot.uploadedSource=nullptr;}
        slot.command = nil;
    }
    CVPixelBufferRef upload(FrameSlot& slot, const AVFrame* frame) {
        require(frame->format==AV_PIX_FMT_NV12,"normalization must produce NV12");
        require(CVPixelBufferPoolCreatePixelBuffer(nullptr,pool,&slot.uploadedSource)==kCVReturnSuccess,"allocate software upload buffer");
        auto pixel=slot.uploadedSource;
        require(CVPixelBufferLockBaseAddress(pixel,0)==kCVReturnSuccess,"lock software upload buffer");
        auto y=(uint8_t*)CVPixelBufferGetBaseAddressOfPlane(pixel,0);
        auto uv=(uint8_t*)CVPixelBufferGetBaseAddressOfPlane(pixel,1);
        size_t ys=CVPixelBufferGetBytesPerRowOfPlane(pixel,0),uvs=CVPixelBufferGetBytesPerRowOfPlane(pixel,1);
        for(int row=0;row<height;row++)std::memcpy(y+row*ys,frame->data[0]+row*frame->linesize[0],width);
        for(int row=0;row<height/2;row++)std::memcpy(uv+row*uvs,frame->data[1]+row*frame->linesize[1],width);
        require(CVPixelBufferUnlockBaseAddress(pixel,0)==kCVReturnSuccess,"unlock software upload buffer");
        return pixel;
    }
    void wait(FrameSlot& slot) {
        if (slot.command.status != MTLCommandBufferStatusCompleted) {
            [slot.command waitUntilCompleted];
        }
        require(slot.command.status == MTLCommandBufferStatusCompleted,
                "GPU command: " + str(slot.command.error.localizedDescription));
    }
    CVMetalTextureRef view(CVPixelBufferRef pixel, int plane, bool destination) {
        CVMetalTextureRef result = nullptr;
        auto usage = destination ? MTLTextureUsageShaderWrite : MTLTextureUsageShaderRead;
        NSDictionary* attributes = @{(id)kCVMetalTextureUsage: @(usage)};
        CVReturn status = CVMetalTextureCacheCreateTextureFromImage(
            nullptr, textureCache, pixel, (__bridge CFDictionaryRef)attributes,
            plane ? MTLPixelFormatRG8Unorm : MTLPixelFormatR8Unorm,
            width >> plane, height >> plane, plane, &result);
        require(status == kCVReturnSuccess, "CVMetalTexture view: " + std::to_string(status));
        return result;
    }
    CVPixelBufferRef compose(FrameSlot& slot, CVPixelBufferRef source) {
        require(CVPixelBufferGetPixelFormatType(source) == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                "source is not video-range NV12");
        CVPixelBufferRef output = nullptr;
        require(CVPixelBufferPoolCreatePixelBuffer(nullptr, pool, &output) == kCVReturnSuccess,
                "allocate output frame");
        // Skia flushed onto this same command queue. The compute dispatch follows
        // its writes without a CPU readback or a per-frame completion wait.
        slot.command = [queue commandBuffer];
        for (int plane = 0; plane < 2; ++plane) {
            slot.views[plane] = view(source, plane, false);
            slot.views[plane + 2] = view(output, plane, true);
        }
        auto encoder = [slot.command computeCommandEncoder];
        [encoder setComputePipelineState:pipeline];
        for (int i = 0; i < 2; ++i)
            [encoder setTexture:CVMetalTextureGetTexture(slot.views[i]) atIndex:i];
        [encoder setTexture:slot.overlay atIndex:2];
        for (int i = 0; i < 2; ++i)
            [encoder setTexture:CVMetalTextureGetTexture(slot.views[i + 2]) atIndex:i + 3];
        [encoder dispatchThreads:MTLSizeMake(width / 2, height / 2, 1)
           threadsPerThreadgroup:MTLSizeMake(16, 16, 1)];
        [encoder endEncoding];
        [slot.command commit];
        return output;
    }
    Codec encoder(const ExportConfig& config, AVRational fps) {
        auto enc = videoEncoder(config.hevc ? "hevc_videotoolbox" : "h264_videotoolbox", config, width, height, fps);
        enc->pix_fmt = AV_PIX_FMT_VIDEOTOOLBOX; enc->hw_frames_ctx = av_buffer_ref(encodeFrames);
        enc->max_b_frames = bFramesFor(enc->codec_id);
        check(av_opt_set(enc->priv_data, "allow_sw", "0", 0), "require hardware encoder");
        check(av_opt_set(enc->priv_data, "prio_speed", "1", 0), "encoder speed priority");
        applyEncoderOptions(enc.get(), config);
        check(avcodec_open2(enc.get(), enc->codec, nullptr), "open VideoToolbox encoder"); return enc;
    }
    int qualityMax(AVCodecContext*) { return 0; }
    void submit(size_t index, AVFrame* cpu, double time) {
        auto& slot = slots[index];
        require(bsr_skia_metal_draw(renderer, time, (__bridge void*)slot.overlay) == 0, "draw Skia Metal frame");
        auto pixel = compose(slot, upload(slot, cpu));
        slot.frame = av_frame_alloc(); require(slot.frame != nullptr, "allocate hardware frame");
        auto f = slot.frame; f->format = AV_PIX_FMT_VIDEOTOOLBOX; f->width = width; f->height = height;
        f->hw_frames_ctx = av_buffer_ref(encodeFrames); f->data[3] = reinterpret_cast<uint8_t*>(pixel);
        f->buf[0] = av_buffer_create(reinterpret_cast<uint8_t*>(pixel), 1, releasePixel, nullptr, 0);
        if (!f->buf[0]) { CVPixelBufferRelease(pixel); throw std::runtime_error("retain pixel buffer"); }
        check(av_frame_copy_props(f, cpu), "copy frame metadata");
        f->pts = cpu->pts; f->duration = 1; f->time_base = cpu->time_base;
        f->pict_type = AV_PICTURE_TYPE_NONE; f->flags &= ~AV_FRAME_FLAG_KEY;
        check(av_vt_pixbuf_set_attachments(nullptr, pixel, f), "pixel buffer metadata");
    }
    Frame take(size_t index) {
        auto& slot = slots[index]; wait(slot);
        Frame result(slot.frame); slot.frame = nullptr; release(slot); return result;
    }
};

} // namespace

extern "C" const char* bsr_ffmpeg_load(const char* directory) { return loadFFmpeg(directory); }
extern "C" int bsr_gpu_export(const ExportConfig* config, char* error, size_t capacity) {
    @autoreleasepool {
        pthread_setname_np("bsr.render");
        pthread_set_qos_class_self_np(QOS_CLASS_USER_INITIATED, 0);
        return exportVideo<MetalCompositor>(config, error, capacity);
    }
}
extern "C" int bsr_gpu_probe(char* error, size_t capacity, int32_t* qualityMax) {
    @autoreleasepool { return probeCodecs<MetalCompositor>(error, capacity, qualityMax); }
}
