# native-overlay

CPU Skia compositor for this application's generated subtitle/danmaku ASS, not a general ASS engine. Skia is pinned to **0.82.0** with `textlayout`; Windows links `advapi32` for the prebuilt ICU dependency. No network access is needed at runtime.

## Linux build baseline

Linux builds require Ubuntu 24.04+ (or equivalent system libraries) and source-build Skia with `FORCE_SKIA_BUILD=1 SKIA_USE_SYSTEM_LIBRARIES=1`. The old upstream Linux archive has no COLRv1 paint implementation and its bundled ICU collides with V8; do not use it for this renderer. CI/release set those variables only on Linux, and the Docker builder/runtime both use Debian trixie. macOS and Windows keep the validated prebuilt configuration. See `docs/development/native-overlay.md` for packages and build/runtime ABI details before running the commands below on Linux.

## Integration

```rust,ignore
use native_overlay::{Renderer, RenderInput, RENDERER_VERSION};
let mut renderer = Renderer::new(width, height, input, font_paths)?;
// Allocate once for streaming/export; every successful call overwrites all bytes.
let mut straight_rgba = vec![0; width as usize * height as usize * 4];
renderer.frame_into(time_seconds, &mut straight_rgba)?;
// Convenience API for callers that want a new Vec for each frame.
let allocated_rgba = renderer.frame(time_seconds)?;
let preview_png = renderer.png(time_seconds)?;
// Optional cumulative stage timings; disabled by default.
renderer.set_profiling(true);
renderer.frame_into(next_time_seconds, &mut straight_rgba)?;
let stats = renderer.stats();
```

The public scene types are in `src/lib.rs`. The stable renderer API is:

```rust,ignore
Renderer::new(width: u32, height: u32, input: RenderInput,
              font_paths: Vec<PathBuf>) -> Result<Renderer, String>
Renderer::frame(&mut self, time: f64) -> Result<Vec<u8>, String>
Renderer::frame_into(&mut self, time: f64, bytes: &mut [u8]) -> Result<(), String>
Renderer::png(&mut self, time: f64) -> Result<Vec<u8>, String>
Renderer::set_profiling(&mut self, enabled: bool)
Renderer::stats(&self) -> RenderStats
Renderer::missing_glyph_count(&self) -> usize
```

Times are seconds and must be finite. `frame_into` requires exactly `width × height × 4` bytes; a different length is rejected before changing the destination, output surface, timeline, caches or statistics. `frame` allocates this buffer and delegates to `frame_into`. A reused buffer is cleared through composition even on empty frames, so no old pixels remain after events end or a seek. `png` uses the same compositor but returns encoded PNG bytes.

`RENDERER_VERSION` is intended for persistent segment-cache keys; bump it when rendering semantics/assets change. The additional `missing_glyph_count()` diagnostic reports unresolved glyphs encountered on cache misses (re-rasterization may count them again).

### Metrics

`stats()` returns a cumulative snapshot, not a live handle. `set_profiling` changes timing collection for subsequent RGBA calls without resetting any totals; timings are disabled by default and there is no per-frame logging.

| Field | Meaning |
| --- | --- |
| `frames` | Successfully completed RGBA frames from either `frame` or `frame_into`, including empty frames, whether profiling is enabled or not. PNG calls and failed RGBA calls do not increment it. |
| `text_layout` | `Duration` spent constructing and laying out foreground/mask paragraphs on text cache misses while profiling is enabled. |
| `text_raster` | `Duration` spent rasterizing text and its outline/shadow/background effects on text cache misses while profiling is enabled. |
| `image_prepare` | `Duration` spent reading, decoding and rasterizing image assets on image cache misses while profiling is enabled. |
| `composition` | `Duration` of composition excluding the nested text-layout, text-raster and image-preparation timings, while profiling is enabled. Includes active-event scheduling and drawing cached sprites. |
| `readback` | `Duration` spent copying the surface into straight RGBA bytes while profiling is enabled. |
| `text_cache_hits`, `text_cache_misses` | Number of text sprite lookups satisfied by the LRU or requiring preparation, respectively. |
| `image_cache_hits`, `image_cache_misses` | Number of image sprite lookups satisfied by the LRU or requiring preparation, respectively. Shared assets avoid repeated decode/rasterization while resident. |

Cache counters are always collected, including during PNG calls; a miss counts an attempted preparation even if it fails. PNG calls never contribute stage timings or `frames`, even when profiling is enabled and their cache is cold. Timings are cumulative work, not end-to-end export latency: they exclude renderer construction, frame-buffer allocation, PNG encoding, downstream FFmpeg work and profiling-disabled calls. A failed profiled RGBA call may still contribute timings/cache accesses for work already attempted.

This crate is a member of the application workspace and uses `src-tauri/Cargo.lock`. Its font and Unicode data are embedded in the binary; desktop bundles and the container image also ship the corresponding license notices. Application-generated preview, project export and SRT burn-in use this renderer. Arbitrary ASS input and portable ASS file export retain their existing compatibility paths.

## Rendering contract

- Events are active on `[start, end)`. Frames have exactly `width × height × 4` bytes, straight/unpremultiplied RGBA8888, no row padding. PNG and RGBA share the same compositor.
- Image events use normalized 1920 × 1080 top-left positions/sizes. Decoded artwork preserves its aspect ratio, centered inside the reserved transparent footprint (contain, not stretch). ASS positions, margins and movement use each document's `PlayResX/Y` and scale to the output.
- Text draws by ASS layer then document/read order. Images draw after text in stable start order. Subtitle collisions stack within the same layer/alignment (including rotated bounds, outlines and shadows); moving danmaku is not rescheduled. Excess overlapping cues can stack outside the frame rather than overlap.
- All generated style fields are parsed. SecondaryColour is validated but unused because there is no karaoke; Encoding is a legacy codepage hint and input is already UTF-8. Supported overrides: `b/i/u/s` toggles, `c` color/reset, `fn/fs` font/size/reset (for the controlled SRT adapter), four-coordinate `move`. Generated literal backslash, escaped braces, word-joiner escaping and `\\N` newlines are supported. Unsupported sections, tags, effects and malformed values return contextual errors.
- Body fonts use the system `FontMgr`, with explicitly provided TTF/OTF/TTC paths registered under canonical and localized family names. Provided faces and system CJK families are fallbacks. ASS cell height is converted to em using body font metrics. A CJK font must be installed or supplied; none is bundled.
- Unicode 16 emoji are recognized as **whole extended grapheme clusters**, even across inline style boundaries, and explicitly use bundled Noto COLRv1. A grapheme inherits its first scalar's inline style. Body runs are coalesced for shaping. Color emoji artwork remains colored; ASS alpha applies. Bold/italic select body faces and do not replace emoji artwork.
- Unsupported characters use Skia's visible missing-glyph/OS fallback rather than failing export. Unresolved glyphs are logged through `log` and counted. An OS LastResort font may cover a missing scalar without reporting it unresolved.

## Memory and scheduling

Only active events are visited on sequential frames; a backwards seek resets the time cursor once. Metadata remains proportional to input event count, not movie raster count. Text/image sprites share a **64 MiB byte LRU**; SkParagraph's separate paragraph cache is disabled. Image sprites are shared by exact asset `PathBuf` and output raster width/height (normalized dimensions scaled to the output and rounded up to whole pixels). Start/end times, trajectory, placement and opacity do not create separate sprites; different paths or rounded dimensions do. Paths are not canonicalized and equal file contents at different paths are not deduplicated.

**Image paths must refer to immutable preprocessed assets for the renderer's lifetime.** There is no per-frame filesystem/mtime validation or invalidation. Replacing a file at the same path can leave cached pixels stale and make a later eviction/redecode disagree with earlier frames. Use a new asset path and recreate the renderer when artwork changes. Evicted sprites are prepared again on demand, including after a backward seek; immutable input keeps the resulting pixels stable.

Text rasters are individually limited to 8 MiB to bound their simultaneous intermediate surfaces. Output surfaces/raw buffers are each limited to 128 MiB and dimensions to 16384. Encoded images are limited to 32 MiB; decoded dimensions are checked before pixel decode (64 MiB), and output image rasters to 64 MiB. Fonts are limited to 64 MiB/file, 256 MiB total, 128 paths and 128 faces/collection. Documents are limited to 128 MiB and dialogue visible text to 1 MiB. These are explicit allocation guards, not a promise of 64 MiB total process RSS: Skia/font/decoder caches and input metadata also use memory.

## Scope and validation

SkParagraph uses its Unicode line breaking/shaping and platform font metrics. It is **not pixel-identical to libass**, including smart-balanced wrapping, glyph metrics and border morphology (Skia dilation). Background boxes use the paragraph's rectangular bounds. Outlines/shadows use an independent opaque silhouette so translucent foreground does not accidentally double its alpha. Arbitrary ASS animations, drawings, transforms, positioning tags, effects and external ASS compatibility intentionally remain outside this crate.

Run the renderer tests within the application workspace:

```sh
cargo test --locked --manifest-path src-tauri/crates/native-overlay/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/crates/native-overlay/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/crates/native-overlay/Cargo.toml --check
```

Tests cover parsing, cache eviction/redecode pixel stability, active indexing/backwards seeks, layout/collision semantics, actual color emoji pixels, all Unicode 16 fully-qualified emoji shaping, PNG/raw parity, allocating/reused-buffer parity and invalid buffer rejection, alpha, nonfatal missing glyphs, image containment/movement, path-and-rounded-size sprite sharing with exact cache counters, and opt-in profiling with PNG excluded from timings/frame counts. Assets with pinned upstream URLs, SHA-256 hashes and licenses are listed in `assets/README.md`.

## Verified integration

- macOS: application GUI/headless compilation, renderer tests and main application tests; real FFmpeg workflows include standalone SRT, text/image danmaku, delayed video clocks, project clips/audio, segment caching and generated subtitles crossing segment boundaries.
- Windows x64: the independent prototype completed both 1080p60 videos and alpha/time checks; this production renderer crate compiled and passed 21 tests with Rust 1.89 / MSVC 14.42.34433. Whole Windows GUI installer/manual interaction is not covered by those results.
- Linux: CI checks the Ubuntu 24.04 source/system-library configuration, including actual color pixels and full application linking. Manual desktop acceptance, long-video sustained memory, 4K and all hardware encoders are not covered by those checks.

Useful integration commands from the repository root:

```sh
cargo check --locked --manifest-path src-tauri/Cargo.toml
cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --features headless
cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --features headless ffmpeg:: -- --ignored --test-threads=2
```

The last command deliberately runs FFmpeg-dependent tests normally marked ignored; it requires a working FFmpeg with the encoders/filters used by those tests. Failed/cancelled streaming jobs drop the child/stdin, signal the blocking raster thread and release the bounded channel; they do not write a movie-sized intermediate RGBA file.
