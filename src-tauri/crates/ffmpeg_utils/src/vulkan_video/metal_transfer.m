#import <CoreVideo/CoreVideo.h>
#import <Metal/Metal.h>
#include <stdio.h>
#include <time.h>

static double bsr_profile_now(void) {
    return (double)clock_gettime_nsec_np(CLOCK_UPTIME_RAW) / 1e9;
}

// Pixel buffer base addresses are never locked or mapped, including on failure.
@interface BSRMetalTransfer : NSObject
@property(nonatomic, strong) id<MTLCommandQueue> queue;
@property(nonatomic, assign) CVMetalTextureCacheRef cache;
@end
@implementation BSRMetalTransfer
- (void)dealloc { if (_cache) CFRelease(_cache); }
@end

// Rust owns both AVFrames until this ticket is destroyed. Retain the CoreVideo
// views and command independently so cancellation cannot release GPU resources
// while the command still uses them. All entry points run on the media worker.
@interface BSRMetalCopy : NSObject
@property(nonatomic, strong) BSRMetalTransfer *transfer;
@property(nonatomic, strong) id<MTLCommandBuffer> command;
@property(nonatomic, assign) CVMetalTextureRef luma;
@property(nonatomic, assign) CVMetalTextureRef chroma;
@property(nonatomic) BOOL committed;
@property(nonatomic) double prepareSeconds;
@property(nonatomic) double viewsSeconds;
@property(nonatomic) double commitSeconds;
- (void)releaseViews;
@end
@implementation BSRMetalCopy
- (void)releaseViews {
    BOOL hadViews = _luma || _chroma;
    if (_luma) { CFRelease(_luma); _luma = NULL; }
    if (_chroma) { CFRelease(_chroma); _chroma = NULL; }
    if (hadViews) CVMetalTextureCacheFlush(_transfer.cache, 0);
}
- (void)dealloc {
    if (_committed && _command.status < MTLCommandBufferStatusCompleted) {
        [_command waitUntilCompleted];
    }
    [self releaseViews];
}
@end

void *bsr_metal_transfer_create(void *source, char *error, size_t capacity) {
    @autoreleasepool {
        id<MTLTexture> texture = (__bridge id<MTLTexture>)source;
        BSRMetalTransfer *transfer = [BSRMetalTransfer new];
        transfer.queue = [texture.device newCommandQueue];
        CVMetalTextureCacheRef cache = NULL;
        CVReturn status = CVMetalTextureCacheCreate(NULL, NULL, texture.device, NULL, &cache);
        transfer.cache = cache;
        if (!transfer.queue || status != kCVReturnSuccess) {
            snprintf(error, capacity, "Create Metal transfer queue/cache (%d)", status);
            return NULL;
        }
        return (__bridge_retained void *)transfer;
    }
}

void bsr_metal_transfer_destroy(void *context) {
    if (context) { (void)CFBridgingRelease(context); }
}

void *bsr_metal_transfer_submit(void *context, void *luma, void *chroma,
                               void *pixel_buffer, char *error, size_t capacity, int profile) {
    @autoreleasepool {
        BSRMetalTransfer *transfer = (__bridge BSRMetalTransfer *)context;
        CVPixelBufferRef pixel = (CVPixelBufferRef)pixel_buffer;
        id<MTLTexture> sources[2] = {(__bridge id<MTLTexture>)luma,
                                    (__bridge id<MTLTexture>)chroma};
        if (!CVPixelBufferGetIOSurface(pixel) || CVPixelBufferGetPlaneCount(pixel) != 2) {
            snprintf(error, capacity, "VideoToolbox output requires a two-plane IOSurface");
            return NULL;
        }
        double started = profile ? bsr_profile_now() : 0;
        BSRMetalCopy *copy = [BSRMetalCopy new];
        copy.transfer = transfer;
        copy.command = [transfer.queue commandBuffer];
        id<MTLBlitCommandEncoder> blit = [copy.command blitCommandEncoder];
        if (!blit) {
            snprintf(error, capacity, "Create Metal blit encoder");
            return NULL;
        }
        for (size_t plane = 0; plane < 2; ++plane) {
            size_t width = CVPixelBufferGetWidthOfPlane(pixel, plane);
            size_t height = CVPixelBufferGetHeightOfPlane(pixel, plane);
            MTLPixelFormat format = plane == 0 ? MTLPixelFormatR8Unorm : MTLPixelFormatRG8Unorm;
            if (sources[plane].device != transfer.queue.device ||
                sources[plane].width != width || sources[plane].height != height ||
                sources[plane].pixelFormat != format) {
                snprintf(error, capacity, "MoltenVK plane %zu has incompatible device/size/format", plane);
                [blit endEncoding];
                return NULL;
            }
            double view_start = profile ? bsr_profile_now() : 0;
            CVMetalTextureRef view = NULL;
            CVReturn status = CVMetalTextureCacheCreateTextureFromImage(
                NULL, transfer.cache, pixel, NULL, format, width, height, plane, &view);
            if (plane == 0) copy.luma = view; else copy.chroma = view;
            id<MTLTexture> target = view ? CVMetalTextureGetTexture(view) : nil;
            if (profile) copy.viewsSeconds += bsr_profile_now() - view_start;
            if (status != kCVReturnSuccess || !target) {
                snprintf(error, capacity, "Create IOSurface Metal plane %zu (%d)", plane, status);
                [blit endEncoding];
                return NULL;
            }
            [blit copyFromTexture:sources[plane] sourceSlice:0 sourceLevel:0
                    sourceOrigin:MTLOriginMake(0, 0, 0) sourceSize:MTLSizeMake(width, height, 1)
                       toTexture:target destinationSlice:0 destinationLevel:0
               destinationOrigin:MTLOriginMake(0, 0, 0)];
        }
        [blit endEncoding];
        double mark = profile ? bsr_profile_now() : 0;
        if (profile) copy.prepareSeconds = mark - started;
        [copy.command commit];
        copy.committed = YES;
        if (profile) copy.commitSeconds = bsr_profile_now() - mark;
        return (__bridge_retained void *)copy;
    }
}

// 1 = pending, 0 = completed, -1 = GPU error. No spin loop. The caller waits
// only for queue backpressure/EOF; ordinary progress checks status once.
int bsr_metal_copy_finish(void *ticket, int wait, char *error, size_t capacity, double *timings) {
    @autoreleasepool {
        BSRMetalCopy *copy = (__bridge BSRMetalCopy *)ticket;
        if (!wait && copy.command.status < MTLCommandBufferStatusCompleted) return 1;
        double mark = timings ? bsr_profile_now() : 0;
        if (wait) [copy.command waitUntilCompleted];
        if (timings) {
            timings[0] = copy.prepareSeconds;
            timings[1] = copy.viewsSeconds;
            timings[2] = copy.commitSeconds;
            timings[3] = wait ? bsr_profile_now() - mark : 0;
            double begin = copy.command.GPUStartTime, end = copy.command.GPUEndTime;
            if (begin > 0 && end >= begin) timings[5] = end - begin;
        }
        int result = 0;
        if (copy.command.status != MTLCommandBufferStatusCompleted) {
            snprintf(error, capacity, "Metal copy failed: %s", copy.command.error.description.UTF8String);
            result = -1;
        }
        mark = timings ? bsr_profile_now() : 0;
        [copy releaseViews];
        if (timings) timings[4] = bsr_profile_now() - mark;
        return result;
    }
}

void bsr_metal_copy_destroy(void *ticket) {
    @autoreleasepool {
        if (ticket) { (void)CFBridgingRelease(ticket); }
    }
}
