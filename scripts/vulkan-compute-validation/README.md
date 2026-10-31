# Shared Vulkan NV12 compositor

Production MediaPlan and the standalone Vulkan Video session now share the
same embedded compute shader. Skia still renders premultiplied RGBA through
Vulkan (MoltenVK on macOS). No runtime GLSL compiler or libplacebo filter is
used. A full FFmpeg SDK may independently retain a libplacebo dynamic dependency.

## Contract

- Inputs have identical, even dimensions: NV12 video with BT.709 primaries,
  BT.709 matrix and limited range, and nonlinear premultiplied RGBA8. Video
  transfer may be BT.709, unspecified (interpreted as BT.709), or sRGB
  (`iec61966-2-1`). Scaling, padding and cadence remain in MediaPlan.
- For sRGB video the same shader reconstructs RGB from each 2x2 NV12 block,
  applies the inverse sRGB transfer followed by the BT.709 encoding transfer,
  then blends Skia's existing premultiplied BT.709 overlay. Alpha is coverage;
  it is never gamma-converted. Reconstructed RGB is clipped to [0,1] under the
  SDR contract. No RGB image intermediate, extra pass or CPU pixel access is
  introduced. Output pixels **and** metadata are BT.709 limited range.
- Transfer selection uses each actual filtered AVFrame, before upload can lose
  its metadata. sRGB/BT.709 concat therefore changes conversion per frame.
  HDR and other primaries remain unsupported by this specialized shader.
  Whole-recording/danmaku exports and projects using native Skia overlays accept
  sRGB. Projects with upstream CPU image/video overlays or external libass
  subtitles retain the BT.709-only preflight: converting their already-blended
  pixels cannot correctly normalize layers with different transfers.
- One invocation covers a 2x2 block. Luma blends at full resolution. Chroma uses
  a coverage-weighted 2x2 box average. The original BT.709 fast path still
  preserves transparent NV12 samples exactly; sRGB samples require conversion.
  No RGB video intermediate, tone mapping, film grain, linear-light blending or
  generic resampling is needed for this operation.
- This is an explicit 4:2:0 box-filter contract, **not pixel equivalence to
  libplacebo's generic chroma reconstruction/downsampling**. Thin saturated
  edges can differ. Existing output chroma metadata is preserved. HDR and
  unnormalized inputs are rejected by the existing compositor input contract.
- Both platforms use the same shader and dispatch. Windows uses plane views
  of a multi-planar NV12 image; MoltenVK uses separate R8/RG8 images. The
  encoder adapters are unchanged: Vulkan Video reads the output directly;
  macOS GPU-copies it into a VideoToolbox IOSurface. This is no CPU readback,
  not a claim of eliminating the CPU-decoded input upload or the Mac GPU blit.
- The existing FFmpeg device, queue/frame locks and timeline semaphores are
  shared. Three retained command slots bound asynchronous work. Input **read**
  dependencies are published as well as output writes; command resources stay
  alive until fences complete. Cancel/drop drains before destroying resources.
- Capability failure still uses the existing GPU probe and CPU fallback.
  The default renderer/cache identity is `vulkan-nv12-compute-v4-srgb`.

## Shader reproducibility

`composite.spv` is checked in next to its GLSL source. Rebuild only when changing
that source; no shader compiler is needed to build or run the application.
The validation machine used Vulkan SDK 1.4.363.0, glslang 16.6.0.

```sh
GLSLANG="$SDK/bin/glslang" SPIRV_VAL="$SDK/bin/spirv-val" \
  sh scripts/vulkan-compute-validation/build-shader.sh --check
# Use --write to intentionally regenerate the embedded binary.
```

## macOS validation

Run native tests with the FFmpeg SDK configured, then explicitly select the
ignored GPU oracle on a machine with an accessible GPU. Set
`BSR_VULKAN_LIBRARY` to an absolute `libvulkan.dylib` path for an unpackaged test
binary. Production bundles always use their own MoltenVK.

```sh
cargo test --manifest-path src-tauri/Cargo.toml -p ffmpeg_utils \
  --features vulkan-video --lib
cargo test --manifest-path src-tauri/Cargo.toml -p ffmpeg_utils \
  --features vulkan-video gpu_compute_ \
  -- --ignored --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features \
  --features headless native_gpu_ -- --ignored --test-threads=1 \
  --skip native_gpu_benchmark_submission_modes_with_default_export_parameters
```

The BT.709 raw oracle deliberately downloads 38 GPU outputs: transparent/opaque/half
alpha primary colors, gray/white, mixed alpha within a chroma block, varying
NV12 bases and no-base black, row padding, non-workgroup-sized images, three
queued frames and a one-frame tail. It compares against a separate double
precision, unpremultiply/YCbCr/blend/average reference (one-byte rounding
tolerance; transparent identity is exact). The sRGB oracle downloads another
38 frames and tests transfer knees, dark/gray ramps, out-of-range/gamut samples,
coloured chroma, and the same alpha cases against a double precision reference.
These downloads are test-only. The MediaPlan integration test additionally
encodes real tagged files, concatenates sRGB then BT.709, and checks 0/0.5/1
opacity through both hardware codecs. It compares decoded NV12 (two-byte
lossy-encoding tolerance), verifies per-frame conversion counters and the
output header, including that BT.709 frames are not converted twice.

`readback-audit.c` interposes FFmpeg transfers/maps without modifying their
results. Build with `clang -dynamiclib -std=c11`, the SDK include/lib paths and
`-lavutil`. For a bundled exporter, rewrite its libavutil dependency to that
bundle's **actual versioned dylib**, then ad-hoc sign the audit dylib. Launch
with `DYLD_INSERT_LIBRARIES=<audit.dylib>` and `BSR_READBACK_AUDIT=<report.json>`.
The raw oracle is the positive control: downloads must equal 76 when running
both `gpu_compute_` tests. The actual export must report zero downloads/maps
and nonzero CPU NV12 uploads. These
counters cover FFmpeg APIs; inspection of the Skia/Metal adapter verifies that
it has no pixel readback/base-address mapping. GPU timestamps and waits are
not pixel transfers. Do not inject this diagnostic into timed benchmarks.

For Khronos validation, use the SDK loader as `BSR_VULKAN_LIBRARY`, put its lib
folder on `DYLD_LIBRARY_PATH`, select the MoltenVK ICD with `VK_DRIVER_FILES`,
and set `VK_LAYER_PATH` / `VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation`.
FFmpeg 9.0.2 emits an existing device-creation diagnostic
`VUID-VkDeviceCreateInfo-pNext-02830` (duplicate promoted timeline feature).
This reproduces in an independent FFmpeg device-only invocation; the compute
oracle produced no additional validation errors. Do not report a completely
clean validation-layer run or silently suppress this SDK diagnostic.

Results are recorded in `validation/macos-correctness.json`; full-load timing
and quality evidence are in `../export-comparison/REPORT-macos-compute.md` once
that comparison is complete. Windows device execution is not part of this
macOS validation pass.

The sRGB validation and real-recording audit are recorded in
[`REPORT-macos-srgb.md`](REPORT-macos-srgb.md).
