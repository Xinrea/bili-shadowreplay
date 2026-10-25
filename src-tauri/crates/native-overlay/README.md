# native-overlay

CPU Skia compositor for this application's generated subtitle/danmaku ASS, not a general ASS engine. Skia is pinned to **0.82.0** with `textlayout`; Windows links `advapi32` for the prebuilt ICU dependency. No network access is needed at runtime.

## Integration

```rust,ignore
use native_overlay::{Renderer, RenderInput, RENDERER_VERSION};
let mut renderer = Renderer::new(width, height, input, font_paths)?;
let straight_rgba = renderer.frame(time_seconds)?;
let preview_png = renderer.png(time_seconds)?;
```

The agreed public types and signatures are in `src/lib.rs`. `RENDERER_VERSION` is intended for persistent segment-cache keys; bump it when rendering semantics/assets change. The additional `missing_glyph_count()` diagnostic reports unresolved glyphs encountered on cache misses (re-rasterization may count them again).

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

Only active events are visited on sequential frames; a backwards seek resets the time cursor once. Metadata remains proportional to input event count, not movie raster count. Text/image sprites share a **64 MiB byte LRU**; SkParagraph's separate paragraph cache is disabled. Text rasters are individually limited to 8 MiB to bound their simultaneous intermediate surfaces. Output surfaces/raw buffers are each limited to 128 MiB and dimensions to 16384. Encoded images are limited to 32 MiB; decoded dimensions are checked before pixel decode (64 MiB), and output image rasters to 64 MiB. Fonts are limited to 64 MiB/file, 256 MiB total, 128 paths and 128 faces/collection. Documents are limited to 128 MiB and dialogue visible text to 1 MiB. These are explicit allocation guards, not a promise of 64 MiB total process RSS: Skia/font/decoder caches and input metadata also use memory.

## Scope and validation

SkParagraph uses its Unicode line breaking/shaping and platform font metrics. It is **not pixel-identical to libass**, including smart-balanced wrapping, glyph metrics and border morphology (Skia dilation). Background boxes use the paragraph's rectangular bounds. Outlines/shadows use an independent opaque silhouette so translucent foreground does not accidentally double its alpha. Arbitrary ASS animations, drawings, transforms, positioning tags, effects and external ASS compatibility intentionally remain outside this crate.

Run the renderer tests within the application workspace:

```sh
cargo test --locked --manifest-path src-tauri/crates/native-overlay/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/crates/native-overlay/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/crates/native-overlay/Cargo.toml --check
```

Tests cover parsing, cache eviction, active indexing/backwards seeks, layout/collision semantics, actual color emoji pixels, all Unicode 16 fully-qualified emoji shaping, PNG/raw parity, alpha, nonfatal missing glyphs, image containment and movement. Assets with pinned upstream URLs, SHA-256 hashes and licenses are listed in `assets/README.md`.

## Verified integration

- macOS: application GUI/headless compilation, renderer tests and main application tests; real FFmpeg workflows include standalone SRT, text/image danmaku, delayed video clocks, project clips/audio, segment caching and generated subtitles crossing segment boundaries.
- Windows x64: the independent prototype completed both 1080p60 videos and alpha/time checks; this production renderer crate compiled and passed 21 tests with Rust 1.89 / MSVC 14.42.34433. Whole Windows GUI installer/manual interaction is not covered by those results.
- Linux validation is explicitly deferred. Long-video sustained memory, 4K and all hardware encoders are not benchmarked by the six-second prototype.

Useful integration commands from the repository root:

```sh
cargo check --locked --manifest-path src-tauri/Cargo.toml
cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --features headless
cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --features headless ffmpeg:: -- --ignored --test-threads=2
```

The last command deliberately runs FFmpeg-dependent tests normally marked ignored; it requires a working FFmpeg with the encoders/filters used by those tests. Failed/cancelled streaming jobs drop the child/stdin, signal the blocking raster thread and release the bounded channel; they do not write a movie-sized intermediate RGBA file.
