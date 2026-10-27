#pragma once

#ifdef __cplusplus
extern "C" {
#endif
void *skia_renderer_new(const char *config, void *device, void *queue, void *texture);
int skia_renderer_draw(void *renderer, double time, void *texture);
void skia_renderer_free(void *renderer);
#ifdef __cplusplus
}
#endif
