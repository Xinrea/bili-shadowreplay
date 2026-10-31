# KosmicKrisp runtime validation — 2026-10-31

The tested KosmicKrisp driver can run the existing Skia Vulkan renderer and
libplacebo GPU NV12 conversion, but cannot run the existing H.264/HEVC Vulkan
Video encoder. This is a measured driver capability limit, not a missing
FFmpeg build option. The complete no-readback hardware-encoded export path
therefore remains unavailable through this driver/version.

## Tested configuration

- Apple M2 Max, macOS 27.0.1 (26A434), arm64.
- Official LunarG macOS Vulkan SDK **1.4.363.0**; KosmicKrisp driver version
  **26.2.99**, driverInfo `vulkan-sdk-1.4.363 (git-269a955c89)`, Vulkan 1.4.363.
- Driver ID: `DRIVER_ID_MESA_KOSMICKRISP`, conformance 1.4.3.2.
- Homebrew FFmpeg **9.0.2**, ffmpeg-next 9.0.0 / ffmpeg-sys-next 9.0.1,
  Skia 0.82.0 with the project's existing Vulkan feature.
- Source baseline: `f6f4045c`, plus the diagnostic harness in this commit.
  Production renderer, compositor and encoder code were not changed.

The official SDK was downloaded from
[LunarG](https://sdk.lunarg.com/sdk/download/1.4.363.0/mac/vulkansdk-macos-1.4.363.0.zip)
and installed only under `/private/tmp/bsr-kosmickrisp-validation/sdk`.
Archive SHA-256:
`163431e0203c8215286a17cf393643ce6b951f77d1b4f74ab30821a1f0beb09b`.
Only the child processes' loader paths were overridden; no system driver was
replaced. Sandboxed Metal enumeration returned no devices, so the recorded GPU
tests ran with ordinary host GPU access outside the execution sandbox.

## Observations

| Check | Result |
| --- | --- |
| Vulkan device creation | Pass; Apple M2 Max, KosmicKrisp |
| Advertised device extensions | 145; **zero Vulkan Video extensions** |
| Queue families | One queue: graphics, compute, transfer; no video encode/decode flags |
| FFmpeg `h264_vulkan` and `hevc_vulkan` | Both fail opening the encoder: missing `VK_KHR_video_encode_queue` |
| Existing native `vulkan-video-poc`, both codecs | Same extension failure; exit 1 before rendering starts |
| Independent production Skia + libplacebo render probe | **60/60 frames, 1920×1080, GPU NV12 output**, zero missing glyphs |
| Render API capture | 338 `vkCmdDraw`, 4 `vkCmdDrawIndexed`, 601 queue submissions |
| Image-to-host copy calls during render probe | **0**; full instance/device creation through destruction captured |
| Image uploads during render probe | 6 `vkCmdCopyBufferToImage` calls; text/image assets still upload |
| Khronos validation layer | Confirmed loaded; no reported VUIDs in this render smoke test |

The two encoder probes report:

```text
Device does not support the VK_KHR_video_encode_queue extension!
Vulkan Video PoC: Open Vulkan video encoder: Function not implemented
```

The independent render probe directly reuses `VulkanCompositor`,
`VulkanRenderer` and the existing ASS/PNG scene. It draws text, emoji and a
moving image over a generated background. Hardware AVFrames are retained on
the GPU and discarded after synchronization; no encoder is opened. This
isolates rendering/conversion support from the unavailable encoding capability.

The trace proves absence of recorded application image-to-host copy commands
for this **render/conversion subpipeline**. It does not prove opaque driver
internals, decoded visual correctness, real-video export, performance, or a
complete encoded no-readback chain. CPU buffer mappings include asset uploads;
their existence alone does not indicate image readback.

The driver also advertises `VK_EXT_external_memory_metal`. That is a potential
building block for a separate VideoToolbox interoperability investigation;
IOSurface/CVPixelBuffer compatibility, lifetime and synchronization have **not**
been tested here. Changing the Vulkan driver alone does not supply that bridge.

## Reproduce

Build the standalone harness with FFmpeg 9 and the Skia Vulkan package:

```sh
FFMPEG_DIR=/opt/homebrew/opt/ffmpeg-full \
  cargo build --locked --manifest-path scripts/vulkan-video-poc/Cargo.toml \
  --bin vulkan-video-poc --bin vulkan-render-probe
```

The recorded build used the previously cached official Skia macOS Vulkan
archive via `SKIA_BINARIES_URL` and a temporary `CARGO_TARGET_DIR`; no production
application platform gates need changing to run these binaries.

From a host shell with GPU access, supply the SDK's `macOS` directory, both
built executables and an empty output directory:

```sh
python3 scripts/vulkan-video-poc/validate_macos.py \
  --sdk /path/to/VulkanSDK/1.4.363.0/macOS \
  --ffmpeg /opt/homebrew/opt/ffmpeg-full/bin/ffmpeg \
  --producer /path/to/target/debug/vulkan-video-poc \
  --render-probe /path/to/target/debug/vulkan-render-probe \
  --output /tmp/kosmickrisp-validation
```

The script selects only the KosmicKrisp ICD and records device capabilities,
individual exit codes, native/CLI errors, render statistics and API call counts.
Its own exit 0 means evidence collection completed; inspect the encoder exit
codes to determine support. FFmpeg CLI is used only as an independent diagnostic
oracle. The project producer and compositor continue to call linked libraries.

The [receipt](validation/macos-kosmickrisp-2026-10-31.json) retains all advertised
device extensions, queue flags, commands, hashes and API call counts.
[Initialization/render logs](validation/macos-kosmickrisp-2026-10-31/)
are committed alongside it with trailing whitespace removed and line endings
normalized to LF; their separate hashes are in `committed_logs`. Original log
hashes are in `artifacts`. The full Vulkan profile JSON and 1.3 MB API trace
remain in `/private/tmp/bsr-kosmickrisp-validation/recorded`, with their hashes
recorded in the receipt; the script regenerates them.

Validation: standalone probe build and Clippy with `-D warnings`, Rust format,
Python syntax, and the actual host GPU runs described above.
