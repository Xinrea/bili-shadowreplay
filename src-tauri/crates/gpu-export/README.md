# Native GPU export

One FFmpeg pipeline performs software decode, display-matrix rotation/mirroring,
aspect-preserving scale/pad to 1920×1080, limited-range NV12 conversion and frame
rate normalization to at least 60 fps (preserving higher source rates up to 240).
Decode, GPU rendering and encode/mux run concurrently with bounded three-frame
queues. Audio packets are copied into MP4 against the same container timestamp
origin, preserving delayed video and danmaku timing. Applied orientation metadata
is removed before encoding. Input must be SDR and audio must support MP4 copy.

Project exports also feed uncompressed NV12/NUT video directly from the existing
FFmpeg segment filter graph into the worker's stdin. FFmpeg retains cutting,
concatenation, picture-in-picture, static sticker/image layers and external ASS;
Skia GPU draws generated subtitles, danmaku and image danmaku, and the GPU performs
the final composition and hardware-encoder handoff. Prepared frames retain the
project's frame rate (1–60 fps), exact frame count and applied orientation. Audio
still follows the existing WAV segment/mix path; there is no intermediate video
file or second video encode. A project with no native overlay uses the same GPU
path with an empty scene. Export and project upload share this implementation.

The scene carries an ordered array of ASS documents, their subtitle/danmaku
source kinds, image events and the segment's timeline offset. Both backends draw
at that offset while output video timestamps remain local to the segment. Cache
keys distinguish CPU and GPU renders.

`native/pipeline.h` owns that shared flow. `metal.mm` and `vulkan.cpp` only supply
their compositor, hardware-frame ownership and encoder configuration. All Skia
parsing, fonts, layout and texture caches remain in `native-overlay`.

The app launches `--gpu-export <job.json>` before GUI/server initialization.
`ffmpeg::native_gpu` owns progress, cancellation and temporary output. Dropping
the task kills the worker and removes its incomplete file. There is no separate
benchmark executable. Generated ASS/PNG assets live until the worker exits.

## Backends

**macOS:** Skia Metal → Metal NV12 compositor → IOSurface-backed CVPixelBuffer →
VideoToolbox. Software-decoded video is uploaded once; composed frames are never
read back to the CPU. Defaults remain HEVC, target 10 Mbps and speed priority;
GOP is fixed at 240. HEVC uses two B frames; H.264 keeps them disabled because the
supported FFmpeg 8 VideoToolbox implementation can emit invalid DTS with
reordering. Hosts with at least 12 logical CPUs retain the existing single CPU
activity thread during export. Native deployment target is macOS 13.3.

**Linux/Windows:** opt in with the application's `vulkan-export` Cargo feature.
Skia Vulkan and the NV12 compute compositor share FFmpeg's Vulkan device and
timeline semaphores with `h264_vulkan` / `hevc_vulkan`. Composed video frames
remain on the GPU through Vulkan Video encoding; only compressed packets return
to the CPU for muxing. Codec alignment padding is filled on the GPU. GOP uses
the encoder's FFmpeg default, as does the B-frame count. Export controls expose
target bitrate and the device's supported encoding quality levels;
VideoToolbox's speed toggle does not apply.

The Vulkan implementation targets NVIDIA, AMD and Intel through Vulkan Video.
Specific GPUs/drivers need Vulkan 1.3, graphics/compute queues, the selected
Video encoding extensions, video maintenance support and R8/RG8 storage-image
reads/writes without format. Software Vulkan devices are rejected. The first
version uses FFmpeg's default device. Desktop releases enable this feature. Linux on
RTX 4090 has been exercised with FFmpeg 8/9; Windows, AMD and Intel still need
target validation. Unsupported Vulkan encoders are omitted from the settings
controls; CPU software encoders remain selectable.

Both backends discover codecs by creating Skia, composing a GPU frame and
encoding/flushing one frame for each codec. The probe does not create video files.

The encoder selector also probes FFmpeg's VideoToolbox, NVENC, QSV, AMF, Media
Foundation, VAAPI, V4L2 and libx264/libx265 encoders. These retain CPU danmaku
rendering and composition. The render backend and encoder are saved separately:
CPU+VideoToolbox and Metal+VideoToolbox remain independent choices, and available
CPU+hardware-encoder combinations bypass Metal/Vulkan rendering and composition.
At least one usable software encoder is required.
Their GOP and B-frame count also use the encoder's FFmpeg defaults. Neither is a UI setting.
Each encoder supplies its own UI controls, defaults and validation; switching
encoders resets their specific options. The same validated options are passed
to the FFmpeg command or the native worker. Software CRF uses `-1` for target
bitrate mode. When enabled, CRF retains a maximum rate of 1.25 times the configured
bitrate and a buffer of twice that value.

## SDKs and FFmpeg ABIs

Build with shared **FFmpeg >= 8** development files (headers and pkg-config files)
and the app's normal Rust/Node dependencies. Cargo does not download or build
FFmpeg; release CI provisions prebuilt shared SDKs. The Vulkan backend also needs Vulkan SDK headers >= 1.3.277 and
`glslc`; use `VULKAN_SDK`, or Vulkan pkg-config files plus `glslc` on PATH.
`GLSLC` can select the shader compiler. On macOS, install `ffmpeg` and `pkg-config`
with Homebrew. Linux still needs the system-library Skia source-build environment
in [native-overlay.md](../../../docs/development/native-overlay.md).

The build stages only the selected Vulkan C headers ahead of each FFmpeg SDK's
includes, preventing a combined SDK prefix from substituting another FFmpeg ABI
or an older Vulkan header set. Vulkan loader/tool versions alone do not establish
the installed header version.

By default, **one SDK** is selected: `brew --prefix ffmpeg` on macOS, or the
current pkg-config configuration on Linux/Windows. `PKG_CONFIG_PATH` can select a
custom single SDK. Explicit **`BSR_FFMPEG_PREFIXES`** opts into several SDKs, each
with `include/` and `lib/pkgconfig/`. Use the platform path separator: `:` on
macOS/Linux, `;` on Windows. SDKs with the same four library ABI majors are
deduplicated. Only the requested SDKs produce adapters; no second SDK is required.

```sh
BSR_FFMPEG_PREFIXES="/opt/ffmpeg8:/opt/ffmpeg9" \
  cargo build --manifest-path src-tauri/Cargo.toml --no-default-features \
  --features headless,vulkan-export
```

On Windows prepare the official Skia Vulkan archive with the existing ICU symbol
fix, set its printed URL, then build against the desired SDKs:

```powershell
rustup component add llvm-tools
node scripts/prepare-windows-skia.mjs --vulkan
# Set SKIA_BINARIES_URL to the printed file:// URL.
$env:BSR_FFMPEG_PREFIXES="C:\ffmpeg8;C:\ffmpeg9"
yarn tauri build --features vulkan-export
```

macOS uses the same variable with its SDK prefixes and no Vulkan feature. The
release workflow requests both FFmpeg 8 and 9 adapters; local defaults request one.

Each adapter compiles the same native sources against its SDK's public types,
with internal C++ linkage and unique entry points. Rust exchanges only stable
configuration structs and opaque GPU handles; FFmpeg structs remain inside their
adapter. No backend has a load-time FFmpeg dependency. One installation can use
either FFmpeg 8 or 9 **when both adapters were included at build time**. A single
adapter build still needs its matching ABI. Future SDK majors must compile
against the APIs used here and be validated; there is no hard-coded upper bound.

At runtime a complete four-library set is selected for one compiled ABI, newest
first. Filenames follow the platform: `libavcodec.62.dylib`, `libavcodec.so.62`,
or `avcodec-62.dll`, and corresponding avformat/avfilter/avutil files. Library
majors and the FFmpeg 8 minimums are verified before creating any FFmpeg objects.
Libraries are kept loaded for the process lifetime. Initialization, including a
failure, is cached once per process.

Search includes build-time library directories, the executable's directory,
Homebrew `opt/ffmpeg*` locations on macOS, system library directories and
`LD_LIBRARY_PATH` on Linux, and `PATH` on Windows. The runtime-only
**`BSR_FFMPEG_LIBRARY_DIR`** selects an explicit complete library set and replaces
automatic discovery. It points to `lib/` (or the distribution's multiarch lib
directory) on Unix, and the directory containing DLLs, usually `bin/`, on Windows.
It cannot add an ABI that was not compiled. An FFmpeg executable alone is
insufficient; matching shared libraries and their dependencies are required.

Windows releases place FFmpeg 9 DLLs beside the executable through the existing
`windows-runtime` resource mapping. Linux releases place FFmpeg 9 shared libraries
in `../lib/bili-shadowreplay/ffmpeg-runtime` relative to the executable (including
inside AppImage), with `$ORIGIN` RPATHs for their dependencies. The Linux Vulkan
bundle config is `src-tauri/tauri.linux.vulkan.conf.json`. Both releases compile
FFmpeg 8/9 adapters, but bundle only the newer runtime. macOS uses the user's
Homebrew libraries. Skia/Noto/Unicode notices remain with `native-overlay`.
