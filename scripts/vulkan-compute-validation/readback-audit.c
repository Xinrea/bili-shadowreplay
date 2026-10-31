// Test-only dyld interposition. It never changes frame data or return codes.
#include <libavutil/hwcontext.h>
#include <libavutil/pixdesc.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
static atomic_ulong uploads, downloads, maps, hardware_transfers;
static int hardware(const AVFrame *f) {
    const AVPixFmtDescriptor *d = f ? av_pix_fmt_desc_get(f->format) : NULL;
    return d && (d->flags & AV_PIX_FMT_FLAG_HWACCEL);
}
static int audited_transfer(AVFrame *dst, const AVFrame *src, int flags) {
    if (hardware(src) && !hardware(dst)) atomic_fetch_add(&downloads, 1);
    else if (!hardware(src) && hardware(dst)) atomic_fetch_add(&uploads, 1);
    else if (hardware(src)) atomic_fetch_add(&hardware_transfers, 1);
    return av_hwframe_transfer_data(dst, src, flags);
}
static int audited_map(AVFrame *dst, const AVFrame *src, int flags) {
    atomic_fetch_add(&maps, 1);
    return av_hwframe_map(dst, src, flags);
}
#define INTERPOSE(new, old) \
    __attribute__((used)) static struct { const void *replacement; const void *original; } \
    interpose_##old __attribute__((section("__DATA,__interpose"))) = { (void *)&new, (void *)&old };
INTERPOSE(audited_transfer, av_hwframe_transfer_data)
INTERPOSE(audited_map, av_hwframe_map)
__attribute__((destructor)) static void report(void) {
    const char *path = getenv("BSR_READBACK_AUDIT");
    if (!path) return;
    FILE *f = fopen(path, "w");
    if (!f) abort();
    fprintf(f, "{\"uploads\":%lu,\"downloads\":%lu,\"maps\":%lu,\"hardware_transfers\":%lu}\n",
        atomic_load(&uploads), atomic_load(&downloads), atomic_load(&maps), atomic_load(&hardware_transfers));
    fclose(f);
}
