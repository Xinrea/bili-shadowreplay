# Windows Vulkan Video PoC

This standalone harness exercises the real overlay scene compositor and linked
FFmpeg libraries without starting Tauri or the speech runtimes. The application
includes the Vulkan backend by default on Windows through the existing native
MediaPlan for real-video danmaku, project exports and uploads; see
[native-overlay.md](../../docs/development/native-overlay.md). The standalone
generated-background benchmark remains a separate subpipeline measurement.

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

The default Windows application backend uses a bounded three-frame asynchronous
submission path; the standalone harness above retains synchronous submission
as its baseline. The application uploads CPU-decoded NV12 video once and
composites the transparent Skia scene on the GPU before encoding through the
same native MediaPlan. Slot reuse and final drain still provide backpressure;
this does not imply hardware source decoding or the absence of internal waits.

To exercise the actual application export functions with the prepared SDK and
Skia package, run from the repository root:

```powershell
# Correctness: real video, alpha, audio, project cache and cancellation.
cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --features headless native_gpu_ -- --ignored --nocapture --test-threads=1 --skip native_gpu_benchmark
# Timing: no Vulkan validation/API-dump layers; warmup plus three alternating pairs.
cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --features headless native_gpu_benchmark -- --ignored --nocapture --test-threads=1
```

The second test compares synchronous and asynchronous composition through the
same native MediaPlan using a moving-video fixture normalized to 1920×1080, 60 fps,
10 Mbps, GOP 120 and encoder `async_depth=3`. Both modes copy the same audio.
Timing includes device/renderer construction, decode, composition, encode,
drain and atomic publication. Full frame grids, decoded audio and temporal
image samples are checked outside each timed interval. API-dump runs use stdout
begin/end markers for attribution; their timings are diagnostic only.
The retained `gpu_wait` statistic also includes acquire-barrier CPU submission
time, so a nonzero value in asynchronous mode is not evidence of a per-frame
host completion wait. Frame-slot reuse and final-drain times are separate.

The 2026-10-30 Windows application comparison on the RTX 5070 produced these
medians over three alternating pairs, after warming both modes:

| Codec | Synchronous | Three-frame asynchronous | Speed ratio |
| --- | ---: | ---: | ---: |
| H.264 Vulkan | 2646.940 ms | 2637.082 ms | 1.0037× |
| HEVC Vulkan | 2827.522 ms | 2733.826 ms | 1.0343× |

The measurement uses the debug test binary. This is a short, CPU-decoded
64×48/25 fps moving-video fixture normalized by the actual export graph to
1080p60, with a short subtitle. It checks complete
export behavior and bounded frame overlap, not representative full-HD source
decode cost or dense danmaku. The small differences are observations from this
run, not a general speed guarantee. Both modes use the same Vulkan encoder and
`async_depth=3`; this table does not compare against CPU composition or NVENC.
Full parameters and validation evidence are in the
[application asynchronous receipt](validation/windows-production-async-2026-10-30.json).

All five Windows export regressions passed. The asynchronous path reached three
in-flight frames and reused three RGB images, with no recorded image-to-host
copies in either the full regression capture or the sixteen per-mode exports.
Core/synchronization validation reported only the existing FFmpeg 9.0.2 encoder
image-view cleanup issue (`05137`); there were no other VUIDs or sync hazards.
That unresolved SDK issue still prevents calling the validation run clean.

## Prerequisites

- Windows x64, a Vulkan Video capable GPU and current vendor driver.
- The project's FFmpeg 9 SDK, including headers, import libraries, runtime DLLs,
  `libplacebo`, and `h264_vulkan` / `hevc_vulkan` support.
- Vulkan SDK with `VK_LAYER_LUNARG_api_dump`, Python 3.9+, Node 22+, Rust,
  MSVC and LLVM. The recorded build used Node 22.16.0 and Rust 1.90.0.
- The `prepare-windows-skia.mjs --vulkan` helper prepares the pinned official
  Skia 0.82 textlayout/Vulkan archive and applies the project's ICU symbol fix.

Windows application target dependencies enable `native-overlay/vulkan` and
`ffmpeg_utils/vulkan-video` automatically, for desktop and headless builds. The
application has no `vulkan-export` opt-in feature. Other platforms retain their
existing backend. The
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

## Default-parameter performance comparison

`vulkan-benchmark` compares the current CPU path with the shared-image Vulkan
path using the project's HEVC / speed / 10000 Kbps / GOP 120 settings at
1920×1080, 60 fps. The CPU path retains the production three recyclable BGRA
buffers, one-frame channel, native frame copy, RGB overlay and NV12 conversion;
rendering and encoding overlap. Its encoder is `hevc_nvenc` with `p1`/VBR and
`hvc1`. The Vulkan path uses `hevc_vulkan`, VBR and the encoder's default quality
level 0, with the existing PoC's B0 policy and default `hev1` tag. These are
backend-specific speed policies, not equivalent quality presets.

Run from an ordinary PowerShell with the SDK configured as above:

```powershell
$skia = node scripts/prepare-windows-skia.mjs --vulkan
$env:SKIA_BINARIES_URL = ($skia | Where-Object { $_ -like 'SKIA_BINARIES_URL=*' }) -replace '^SKIA_BINARIES_URL=', ''
$env:CARGO_TARGET_DIR = "$PWD\src-tauri\target\vulkan-poc"
cargo build --release --locked --manifest-path scripts/vulkan-video-poc/Cargo.toml --bin vulkan-benchmark
python scripts/vulkan-video-poc/benchmark.py `
  --executable "$env:CARGO_TARGET_DIR\release\vulkan-benchmark.exe" `
  --ffmpeg "$env:FFMPEG_DIR\bin\ffmpeg.exe" --ffprobe "$env:FFMPEG_DIR\bin\ffprobe.exe" `
  --output src-tauri/target/vulkan-default-benchmark
```

The driver warms up both paths, alternates their order over five runs, and uses
the median **complete job** time, including initialization, drain, muxing and
resource destruction. API tracing and validation are disabled for timings.
Post-run checks independently verify every output frame's timestamp, frame
count and GOP, and compare decoded scene samples outside the measured interval.

The two synthetic loads use the default danmaku style (font size 36 in 720p ASS
coordinates, bold 微软雅黑, opacity 0.8), at 2 or 10 arrivals per second with
eight-second flights. These rates describe test content, not application
scheduler defaults. Both paths use the same generated background and scene.
There is no source-video decode, audio or project-cache work here: the measured
ratio describes the render/composite/convert/encode subpipeline, not a whole
project export or an equal-quality codec comparison.

### Measured on 2026-10-30

On the Windows Ryzen 9 5900X / RTX 5070 machine, using the defaults above and
five trials per backend per scene, the completed-job medians were:

| Synthetic arrivals | CPU Skia + NVENC | Skia Vulkan + Vulkan Video | Speed ratio | Time reduction |
| --- | ---: | ---: | ---: | ---: |
| 2 messages/sec | 16.934 s / 106.3 fps | 7.854 s / 229.2 fps | 2.156× | 53.62% |
| 10 messages/sec | 21.495 s / 83.7 fps | 7.916 s / 227.4 fps | 2.715× | 63.17% |

Each job encoded 1800 frames / 30 seconds. Every measured output passed the
frame-count, timestamp and GOP checks. The five outputs in each scene/backend
cohort had identical SHA-256 values; the sampled CPU/GPU scenes also passed the
content sanity check. Observed B-frame counts were zero on both backends.

The corresponding median VBR bitrates were 8.571/9.133 Mbps (CPU/Vulkan) for
the 2/sec load and 10.490/9.673 Mbps for 10/sec. Both requested 10 Mbps; these
are observed default-policy results, not quality-matched encodes. CPU raster
surface readback took 0.75–0.89 s and native raw-frame allocation/copy took
3.59–3.73 s per job. Those overlapping stages are not additive savings; the
complete-job timings above establish the speed ratio.

The [benchmark receipt](validation/windows-default-benchmark-2026-10-30.json)
preserves all twenty trials, encoder settings, output metadata/hashes, stage
timings, hardware, binary/source checks and interpretation limits. The full
raw receipt/logs remain in the ignored `output/benchmark-default-20261030`
directory and on the Windows host.
