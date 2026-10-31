#import <CoreVideo/CoreVideo.h>
#import <Metal/Metal.h>
#include <stdio.h>
#include <time.h>

static double bsr_profile_now(void) {
    return (double)clock_gettime_nsec_np(CLOCK_UPTIME_RAW) / 1e9;
}

// This bridge only copies between GPU textures. Pixel buffer base addresses
// are never locked or mapped, including on failure.
@interface BSRMetalTransfer : NSObject
@property(nonatomic, strong) id<MTLCommandQueue> queue;
@property(nonatomic, assign) CVMetalTextureCacheRef cache;
@end
@implementation BSRMetalTransfer
- (void)dealloc {
    if (_cache) CFRelease(_cache);
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

int bsr_metal_transfer_copy(void *context, void *luma, void *chroma,
                            void *pixel_buffer, char *error, size_t capacity, double *timings) {
    @autoreleasepool {
        BSRMetalTransfer *transfer = (__bridge BSRMetalTransfer *)context;
        CVPixelBufferRef pixel = (CVPixelBufferRef)pixel_buffer;
        id<MTLTexture> sources[2] = {(__bridge id<MTLTexture>)luma,
                                    (__bridge id<MTLTexture>)chroma};
        CVMetalTextureRef planes[2] = {NULL, NULL};
        if (!CVPixelBufferGetIOSurface(pixel) || CVPixelBufferGetPlaneCount(pixel) != 2) {
            snprintf(error, capacity, "VideoToolbox output requires a two-plane IOSurface");
            return -1;
        }
        int result = -1;
        double started = timings ? bsr_profile_now() : 0;
        double mark = started;
        id<MTLCommandBuffer> command = [transfer.queue commandBuffer];
        id<MTLBlitCommandEncoder> blit = [command blitCommandEncoder];
        if (!blit) {
            snprintf(error, capacity, "Create Metal blit encoder");
            return -1;
        }
        for (size_t plane = 0; plane < 2; ++plane) {
            size_t width = CVPixelBufferGetWidthOfPlane(pixel, plane);
            size_t height = CVPixelBufferGetHeightOfPlane(pixel, plane);
            MTLPixelFormat format = plane == 0 ? MTLPixelFormatR8Unorm : MTLPixelFormatRG8Unorm;
            if (sources[plane].device != transfer.queue.device ||
                sources[plane].width != width || sources[plane].height != height ||
                sources[plane].pixelFormat != format) {
                snprintf(error, capacity, "MoltenVK plane %zu has incompatible device/size/format", plane);
                goto cleanup;
            }
            double view_start = timings ? bsr_profile_now() : 0;
            CVReturn status = CVMetalTextureCacheCreateTextureFromImage(
                NULL, transfer.cache, pixel, NULL, format, width, height, plane, &planes[plane]);
            id<MTLTexture> target = planes[plane] ? CVMetalTextureGetTexture(planes[plane]) : nil;
            if (timings) timings[1] += bsr_profile_now() - view_start;
            if (status != kCVReturnSuccess || !target) {
                snprintf(error, capacity, "Create IOSurface Metal plane %zu (%d)", plane, status);
                goto cleanup;
            }
            [blit copyFromTexture:sources[plane] sourceSlice:0 sourceLevel:0
                    sourceOrigin:MTLOriginMake(0, 0, 0) sourceSize:MTLSizeMake(width, height, 1)
                       toTexture:target destinationSlice:0 destinationLevel:0
               destinationOrigin:MTLOriginMake(0, 0, 0)];
        }
        [blit endEncoding];
        blit = nil;
        if (timings) { mark = bsr_profile_now(); timings[0] = mark - started; }
        [command commit];
        if (timings) { double now = bsr_profile_now(); timings[2] = now - mark; mark = now; }
        // Completion wait only: no GPU pixels cross into CPU memory. Keep
        // source AVFrame and both CVMetalTextures alive until the copy ends.
        [command waitUntilCompleted];
        if (timings) {
            timings[3] = bsr_profile_now() - mark;
            // Device timestamp interval, not host wait time. No extra wait added.
            double begin = command.GPUStartTime, end = command.GPUEndTime;
            if (begin > 0 && end >= begin) timings[5] = end - begin;
        }
        if (command.status != MTLCommandBufferStatusCompleted) {
            snprintf(error, capacity, "Metal copy failed: %s", command.error.description.UTF8String);
            goto cleanup;
        }
        result = 0;
    cleanup:
        if (timings) mark = bsr_profile_now();
        if (blit) [blit endEncoding];
        for (size_t plane = 0; plane < 2; ++plane) if (planes[plane]) CFRelease(planes[plane]);
        CVMetalTextureCacheFlush(transfer.cache, 0);
        if (timings) timings[4] = bsr_profile_now() - mark;
        return result;
    }
}
