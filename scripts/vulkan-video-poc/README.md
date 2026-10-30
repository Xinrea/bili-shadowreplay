# Windows Vulkan Video PoC

This standalone harness exercises the real overlay scene compositor and linked
FFmpeg libraries without starting Tauri or the speech runtimes. It is an opt-in
development path, not yet the application's export backend.

```text
ASS / images / fonts
        ↓
Skia Ganesh Vulkan (including cached text/effect sprite surfaces)
        ↓ borrowed VkImage, same VkDevice / graphics queue
FFmpeg AVHWFramesContext RGB pool
        ↓ libplacebo GPU colour conversion
NV12 Vulkan frames
        ↓ h264_vulkan / hevc_vulkan
compressed AVPackets → MP4
```

The producer executable does not invoke FFmpeg CLI, request CPU frame pixels,
or use `read_pixels`, `av_hwframe_transfer_data`, or a rawvideo pipe. Image/font
assets still upload to the GPU, compressed packets return to the CPU for muxing,
and conversion/encoding can perform internal GPU copies. The first PoC waits on
GPU completion on the CPU to establish correctness; it is not an asynchronous
pipeline or a claim about opaque driver internals.

## Prerequisites

- Windows x64, a Vulkan Video capable GPU and current vendor driver.
- The project's FFmpeg 9 SDK, including headers, import libraries, runtime DLLs,
  `libplacebo`, and `h264_vulkan` / `hevc_vulkan` support.
- Vulkan SDK with `VK_LAYER_LUNARG_api_dump`, Python 3.9+, Node 22+, Rust,
  MSVC and LLVM. The recorded build used Node 22.16.0 and Rust 1.90.0.
- The `prepare-windows-skia.mjs --vulkan` helper prepares the pinned official
  Skia 0.82 textlayout/Vulkan archive and applies the project's ICU symbol fix.

The feature boundaries are `native-overlay/vulkan` and
`ffmpeg_utils/vulkan-video`. Other builds retain their existing backend. The
native handle interface is independent of Windows external-memory handles:
rendering, conversion and encoding share one Vulkan device. A future platform
backend can use that interface after checking its device/codec support.

## Run

From the repository root in an ordinary, non-elevated PowerShell:

```powershell
$env:FFMPEG_DIR = "$PWD\src-tauri\target\ffmpeg-sdk"
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/vulkan-video-poc/run.ps1
# Reuse the executable and exercise HEVC:
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/vulkan-video-poc/run.ps1 `
  -SkipBuild -Encoder hevc_vulkan -OutputDirectory src-tauri/target/vulkan-poc-hevc
```

The driver captures only the producer process, starting before device creation
and ending after device destruction. It requests that implicit overlays be
disabled for that child process. Windows Vulkan Loader ignores `VK_LAYER_PATH`
and `VK_LOADER_LAYERS_DISABLE` for high-integrity processes, including an elevated
SSH session. Use ordinary user privileges for an attributable capture and for
selecting a separate validation layer; setting the variables in an elevated
shell does not prove that the requested layer was loaded. The recorded final
runs used medium integrity (`S-1-16-8192`).

Verification runs in separate processes with capture disabled. Process
environment overrides are restored when the driver exits.

Each result directory contains the MP4, producer statistics, API dump and two
verification receipts. `verify.py` requires graphics draws, Vulkan Video encode,
queue submission, device creation/destruction, and rejects image-to-buffer/host
readback calls. It independently decodes every output frame with `ffprobe` to
check codec, dimensions, pixel format, frame count, timestamp grid and duration.
`verify_scene.py` selects decoded frame numbers 0, 60 and 118 in the default
120-frame run, checking the background, moving translucent sprite and subtitle
cue. These validators intentionally
use CLI and CPU decoding outside the captured producer; they are development
test tools, not application runtime dependencies.

## Ownership and synchronization

FFmpeg owns the instance, device, RGB pool, frame images and timeline semaphores.
Skia borrows their native handles, queries functions with the requested instance
or device (including null-instance global queries), and never frees the images.
Only one frame lease may be active. The bridge waits/transitions before Skia
writes, publishes its final image layout/access state, and synchronizes before
passing the frame to the GPU filter and encoder. Skia is destroyed before the
session/device; failures do not publish the temporary MP4 over an existing file.

The RGB image's actual Vulkan format determines Skia's channel mapping:
`R8G8B8A8_UNORM` uses `RGBA8888`; `B8G8R8A8_UNORM` uses `BGRA8888`. FFmpeg's
software-format name alone is insufficient to identify the native image format.
Before rendering, the bridge performs a real GPU transition into
`COLOR_ATTACHMENT_OPTIMAL`, avoiding Skia m133's `GENERAL`-layout inference of
host-write access. After Skia completes, it transitions back to `GENERAL` for
the GPU converter.

The initial same-size `scale_vulkan` conversion failed with this SDK. The PoC
uses `libplacebo`, attaching the shared hardware device **before** initializing
the filter. Automatic software converters are disabled, and the output pool is
checked to belong to the same `VkDevice` before opening the encoder.

For the single-image, two-plane NV12 output pool, the bridge removes the unused
`ALIAS` image flag before allocating the first frame. The tested driver's video
profile allows `MUTABLE_FORMAT` and `EXTENDED_USAGE`, but not `ALIAS`.
`VIDEO_PROFILE_INDEPENDENT` and all rendering/encoding usage bits remain in
place. Multi-image or preallocated pools are rejected rather than modified.
This changes image creation flags without adding an image copy.

## Verification record

The [2026-10-30 receipt](validation/windows-rtx5070-2026-10-30.json) records the
final runs on an NVIDIA GeForce RTX 5070, driver `32.0.16.1742`, using Gyan's
FFmpeg 9.0.2 shared SDK and Skia 0.82. The H.264 run exercised the complete
Skia-archive preparation and standalone Cargo release build; the HEVC run reused
that executable. Both ran at 1920×1080, 60 fps, 120 frames / 2 seconds.

| Observation | H.264 | HEVC |
| --- | ---: | ---: |
| Independently decoded frames | 120 | 120 |
| Graphics draw records | 672 | 672 |
| `vkCmdEncodeVideoKHR` records | 120 | 120 |
| Image-to-buffer/host readback records | 0 | 0 |
| Distinct reused RGB pool images | 2 | 2 |
| Render/convert/encode wall time under capture | 461.32 ms | 613.45 ms |

Both outputs passed checks for `yuv420p`, limited-range BT.709 tags, the complete
timestamp grid and duration. Frames 0 / 60 / 118 contained the expected moving
75%-opaque red sprite; the subtitle appeared only in the middle sample, with
zero missing glyphs. These simple-scene wall times include CPU synchronization
and capture overhead; they are not a production export benchmark.

The trace records six asset uploads and ten buffer mappings in each run.
Those are compatible with glyph/image uploads and compressed packet handling;
they are not evidence of raw-frame readback. The conclusion combines the
producer's GPU-only frame interface with the complete API dump. A zero-valued
readback timing counter alone would not prove it. Complete traces, videos and
logs remain in the ignored `output/final-*` directories and on the Windows
validation host; the committed receipt preserves their SHA-256 values.

### Remaining FFmpeg SDK cleanup issue

Separate runs with Khronos validation layer 1.4.366 and synchronization
validation enabled completed for both codecs without a synchronization hazard,
image-layout diagnostic or video-profile diagnostic. They are **not entirely
validation-clean**: each reports one grouped
`VUID-vkDestroyDevice-device-05137` error listing six leaked `VkImageView`
objects at device destruction. The receipt preserves that diagnostic and the
validation layer DLL hash.

Source inspection attributes this to FFmpeg's remaining reference pictures:
[`ff_hw_base_encode_close`](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavcodec/hw_base_encode.c)
frees the last picture records without the operation-specific cleanup used by
the normal retirement path, while
[`vulkan_encode_free`](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavcodec/vulkan_encode.c)
owns the input/DPB view destruction. The same cleanup diagnostic was reproduced
with a CLI baseline; it is not a Skia surface lifetime issue. Public
`avcodec_flush_buffers` does not release those private picture views. This SDK
needs an upstream cleanup fix before promoting the PoC into the application's
production export path; the PoC does not access FFmpeg private structures to
hide the problem.

### Local checks and hardware lifecycle check

```powershell
python -B -m unittest discover -s scripts/vulkan-video-poc -p test_verify.py -v
cargo clippy --manifest-path scripts/vulkan-video-poc/Cargo.toml --all-targets --locked -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml -p ffmpeg_utils --release --locked `
  --features vulkan-video vulkan_gpu_frame_lease_lifecycle_keeps_existing_output `
  -- --ignored --nocapture
```

The ignored test needs the real Vulkan Video device and SDK. It checks repeated
acquisition, finishing or dropping a session with a live lease, and cancellation
before or during a lease, preserving an existing output and removing temporary
MP4 files. The final hardware run passed all five cases. A separate end-to-end
run aborted after ten rendered frames, returned exit code 1, preserved the
existing video's SHA-256 and left no temporary MP4. A subsequent successful run
atomically replaced that same path with an independently decoded 640×480,
12-frame video, again leaving no temporary MP4. Both results are included in the
committed receipt. The Windows source check also matched all 70 checked source,
manifest, helper and validator files against the local copies; the receipt
preserves the source-manifest digest.
