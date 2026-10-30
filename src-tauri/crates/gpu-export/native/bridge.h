#pragma once
#include <stdint.h>
#include <stddef.h>

// Stable C ABI: no FFmpeg structs cross between SDK adapters or Rust.
struct EncoderOption {
    const char *name, *value;
};
struct ExportConfig {
    const char *source, *output, *scene;
    double fps;
    int64_t bitrate;
    int32_t hevc;
    const EncoderOption *options;
    size_t option_count;
    int32_t prepared;
};
struct SkiaVulkanContext {
    void *instance, *physical_device, *device, *queue;
    uint32_t queue_family, width, height;
    void *resolver;
    const void *(*get_proc)(void *, const char *, void *, void *);
    const char *const *instance_extensions;
    uint32_t instance_extension_count;
    const char *const *device_extensions;
    uint32_t device_extension_count;
};
struct SkiaVulkanImage {
    uint64_t image, memory, offset, size;
    uint32_t layout, queue_family;
};
extern "C" {
void *bsr_skia_metal_new(const char *, uint32_t, uint32_t, void *, void *, void *);
int bsr_skia_metal_draw(void *, double, void *);
void *bsr_skia_vulkan_new(const char *, const SkiaVulkanContext *, const SkiaVulkanImage *);
int bsr_skia_vulkan_draw(void *, double, const SkiaVulkanImage *);
void bsr_skia_free(void *);
void bsr_gpu_progress(int64_t, double, double, int);
}
