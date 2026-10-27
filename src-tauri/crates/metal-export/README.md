# macOS Metal export

The application starts its own executable with `--metal-export <job.json>` before
GUI/server initialization. `ffmpeg::metal` owns the child, its job directory and
incomplete output. Dropping a task kills the child and removes temporary files.
This uses the same cancellation mechanism as external FFmpeg jobs.

The worker performs software decode (automatic thread count), applies input
display-matrix rotation/mirroring, then performs aspect-preserving scale/pad to
1920 × 1080, limited-range NV12 conversion and timestamp normalization
to at least 60 fps (preserving higher source frame rates). Its three bounded stages are decode → render → encode/mux. Three
overlay textures allow Skia Metal and the Metal NV12 compositor to run ahead;
completed IOSurface-backed CVPixelBuffers go directly to VideoToolbox. The video
upload is a CPU copy; the composed frames are never read back to the CPU. Audio
packets are copied, shifted by the same container timestamp origin, and muxed into MP4.
The adapter returns the actual output path with a `.mp4` extension, including for
imported MOV/MKV sources.

Orientation is taken from the first decoded frame, falling back to stream codec
parameters. Quarter turns transpose the pixels and their sample aspect ratio
before scaling. Applied display-matrix side data is removed from normalized
frames before Skia composition and encoding, so players display the output in
the same upright coordinate system as the danmaku.

The defaults are HEVC, target 10 Mbps, GOP 120, no B frames and speed priority.
Settings select H.264/HEVC, target bitrate and speed priority. The latter is only
visible for a codec whose actual VideoToolbox hardware session can be opened.
Codec/bitrate controls also apply to the existing FFmpeg export on other platforms.
The setting applies to danmaku exports; subtitle-only exports and stream copies
retain their existing path.

The worker handles the application's generated ASS/PNG scene. Input codecs and
containers are supplied by the loaded Homebrew FFmpeg build. Output is 8-bit SDR
1080p at the selected frame rate with copied source audio; audio must be supported by MP4. This backend
does not perform HDR tone mapping.

## Build

Use Xcode command-line tools, Homebrew **FFmpeg >= 8.0** and pkg-config, in addition
to the application's usual Rust/Node tools. By default, `build.rs` uses
`brew --prefix ffmpeg` and compiles **one ABI adapter** with that installation's
headers. No second FFmpeg installation is required. The runtime bindings are
generated from the adapters actually built; a single-version build contains no
references to other FFmpeg ABIs. There is no FFmpeg/dav1d download or source build
in this project.

Set **`BSR_FFMPEG_PREFIXES`** at build time to select explicit installation
prefixes, separated by `:` on macOS. Multiple prefixes opt in to building multiple
ABI adapters; installations sharing the same four library ABI majors produce one
adapter. Each adapter compiles the same `export.mm` with its own headers and
internal C++ implementation linkage. All adapters share one Rust Skia renderer.
Changing or unsetting the variable triggers Cargo to rebuild the adapter set.

| Library | Minimum version (FFmpeg 8.0) |
| --- | --- |
| libavformat | 62.3.100 |
| libavcodec | 62.11.100 |
| libavfilter | 11.4.100 |
| libavutil | 60.8.100 |

The executable has no load-time FFmpeg dependency. At runtime it finds a complete
library set matching **one of the built adapters**, under the standard Homebrew
`opt/ffmpeg/lib` and `opt/ffmpeg@8/lib` paths or the supplied SDK prefixes' `lib`
directories. It prefers the newest built ABI when several are available. Only
the chosen adapter is initialized. Each function is resolved from its
owning library with `dlopen`/`dlsym`; runtime ABI majors and the 8.0 version floor
are checked before creating any FFmpeg objects. Loaded libraries remain alive
for the process lifetime. Initialization is cached once per process, including
failures, so a broken installation reports its error without trying another ABI
in the same process.

Users need **a version matching an adapter in their application build**, on the
same architecture as the app. An explicitly built 8/9 multi-version application
can use either version without rebuilding; a default single-version application
requires its corresponding ABI. FFmpeg 7 and earlier are unsupported. Builds
have no hard-coded upper version limit, but a new major must still compile
against the public APIs used here and pass runtime validation. A copied `ffmpeg`
executable alone does not supply the required shared libraries. Missing or
unbuilt ABIs produce an export error instead of preventing the app from starting.
`BSR_FFMPEG_LIBRARY_DIR` selects an explicit library directory for a non-default
runtime installation; it replaces automatic discovery.

Build natively for aarch64 or x86_64 with matching Homebrew libraries. The release
workflow uses separate Apple Silicon and Intel runners. The native component's
deployment target remains macOS 13.3; the installed Homebrew libraries may impose
a higher minimum OS version.

```sh
brew install ffmpeg pkg-config
export MACOSX_DEPLOYMENT_TARGET=13.3
cargo build --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml -p native-overlay -p metal-export
```

To build one application for both 8 and 9:

```sh
brew install ffmpeg@8 ffmpeg@9 pkg-config
BSR_FFMPEG_PREFIXES="$(brew --prefix ffmpeg@8):$(brew --prefix ffmpeg@9)" \
  cargo build --manifest-path src-tauri/Cargo.toml
```

The same variable can name a single custom prefix. It is separate from
`BSR_FFMPEG_LIBRARY_DIR`, which selects libraries at runtime and cannot add an ABI
that was not built. The macOS release workflow installs both major versions and
exports `BSR_FFMPEG_PREFIXES` for each architecture's build, so each release
contains both ABI adapters. Local builds without the variable still build only
the default `ffmpeg` installation's ABI.

Job JSON is produced by the application from upstream's `OverlaySpec`: it contains
`source`, `output`, `ass_file`, `fps`, `overlays`, `encoder`, `bitrate_bps` and
`speed_priority`. Shared application settings live in `ffmpeg::encoding`; the
VideoToolbox switch is named `videotoolbox_speed_priority` there. Each image has
`path`, `start`, `end`, `from`/`to` objects with `x`/`y`, `width`, `height` and
`opacity`, using the planner's rectangular footprints. ASS and image paths
remain valid until the worker exits. Video and copied audio use the container
start as their common origin, preserving delayed video and danmaku timing.

The worker reports media time and elapsed time for progress, then emits a
completion record with frame count, duration and total elapsed time.

FFmpeg and its codec libraries are supplied by the user's Homebrew installation.
Skia/Noto/Unicode notices remain with native-overlay.
