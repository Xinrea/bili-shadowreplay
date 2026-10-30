# FFmpeg Vulkan encoder cleanup

FFmpeg 9.0.2 can leave the last queued H.264/HEVC Vulkan picture's input and DPB
image views alive at device destruction. Draining the encoder does not release
every retained reference picture. The Windows validation baseline reproduces
`VUID-vkDestroyDevice-device-05137` outside Skia as well.

The included patch is the unchanged upstream fix
[`ddfa842088cd86bd48bc4533f10a4b5badcbd02d`](https://github.com/FFmpeg/FFmpeg/commit/ddfa842088cd86bd48bc4533f10a4b5badcbd02d).
It releases queued picture views before the base context discards their records,
and releases the packet buffer pool before the base context. The application
continues to use public FFmpeg APIs; it does not inspect private picture layouts.
The `n9.0.2` tag and `release/9.0` branch still lack this fix as checked on
2026-10-30. Current master contains it.

## Backport to a source build

Extract an unmodified FFmpeg 9.0.2 source tree with LF line endings, then run:

```sh
python scripts/ffmpeg-sdk-patches/apply.py /path/to/ffmpeg-9.0.2 --check
python scripts/ffmpeg-sdk-patches/apply.py /path/to/ffmpeg-9.0.2
```

The offline helper verifies the upstream patch, source and patched-file SHA-256
values from `manifest.json`. It refuses unknown source contents and is idempotent
after a successful application. It patches source only: building a replacement
SDK still requires the intended compiler, complete FFmpeg configuration and all
third-party development dependencies.

Rebuild and validate the complete SDK before using it for exports. Replacing one
official `avcodec` DLL with a differently configured custom DLL has not been
validated and can remove decoders/encoders or mismatch dependencies. Keep a full
matching set of headers, import libraries and runtime DLLs in a separate prefix.

## Isolated Windows comparison SDK

The initial comparison uses the complete BtbN master GPL shared SDK from the
2026-10-30 build, independently of the existing Gyan 9.0.2 SDK:

- Asset: `ffmpeg-master-latest-win64-gpl-shared.zip` from
  [BtbN FFmpeg Builds](https://github.com/BtbN/FFmpeg-Builds/releases/tag/latest).
- Archive SHA-256:
  `6ba61dd19a10cc2858a7868d0327e7b85bacda7dcc12b1894ea36a031653e964`.
- Reported source version: `N-128212-g17a4a474f4-20261030`.
- Public ABI majors: avcodec 63, avutil 61, avformat 63, avfilter 12,
  swresample 7 and swscale 10.

The `latest` asset URL is mutable; the pinned archive digest is the identity of
this comparison, and a different digest requires a new validation record. This
does not change the project's release SDK policy. A complete compile/runtime,
Vulkan core/synchronization-layer and export regression run is required before
using any replacement SDK for production exports.

The initial compile probe rejects this particular master snapshot with the
currently pinned Rust bindings: `ffmpeg-sys-next` 9.0.1 lacks two CUDA public
types in its header wrapper, and `ffmpeg-next` 9.0.0 has exhaustive matches that
do not cover the snapshot's new sample/pixel formats, codec IDs and side data.
Matching public ABI majors alone therefore does not establish build
compatibility. This snapshot is retained for comparison, not selected as a
drop-in application SDK.

API capture and validation must run in an ordinary medium-integrity Windows
process. High-integrity SSH processes can cause the Vulkan loader to ignore
layer-selection environment variables. Record the actual process integrity,
loaded layer version and library hashes with the result.
