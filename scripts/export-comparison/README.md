# Five-minute production export comparison

Measured results and caveats: [2026-10-31 report](REPORT.md). PR459's median
throughput was 4.522× current on the M2 Max and 1.299× current on the Windows
5900X/RTX 5070 host in this scenario; the report includes default-encoder
differences and complete per-run validation receipts.

This compares current commit `730ebd23` with PR
[#459](https://github.com/Xinrea/bili-shadowreplay/pull/459), pinned to
`c73c7aaff9b9dd9abb561c90a48f19b2d6374112`. Both use release builds and their
actual production render/decode/encode/mux implementations. A small runner
avoids launching Tauri, V8 and speech runtimes; it does not replace the media
algorithms. Preparing assets and scheduling danmaku are outside timing on both
sides; scene parsing, device creation, real video/audio work and final close are
inside the measured child-process wall time.
The operation is a single-video danmaku burn; project segment-cache reuse,
uploading and GUI startup are outside this comparison.

## Scenario

- A continuous 300-second section (60–360 seconds) of the native 1080p60
  [Big Buck Bunny Sunflower video](https://peach.blender.org/download/).
  No short-loop or low-resolution upscaling. Input preparation retains 1920×1080
  and 60 fps, normalizes SDR BT.709 tags, encodes H.264/AAC once, then distributes
  that same file to both platforms. It contains exactly 18,000 video frames.
- Twenty arrivals per second at 50 ms intervals: sixteen text danmaku and four
  PNG emoticons, totaling 4,800 text and 1,200 image events. Text includes Chinese,
  English and Unicode emoji. Every message rolls for eight seconds (up to 160
  active events); events naturally extending past the movie end are clipped by
  the movie. The last arrivals begin outside the right edge, just like earlier
  arrivals.
- One identical ASS/image scene on both sides, 36-point bold text in 720p ASS
  units, 80% opacity, 54×54 image events and 12 shared emoticon assets. The same
  system font manager/Arial/CJK fallback is used within each platform. This does
  not install fonts or patch the renderer to force identical cross-platform
  font files.
- HEVC, speed priority, 10 Mbps, 1080p60 and copied source audio. Other encoder
  settings retain each version's actual defaults: these are product-default
  comparisons, not an equal-encoding-quality claim. Actual GOP, picture types,
  bitrate and file sizes are recorded for every output.

Source and scene identities are pinned in `fixtures.json`. The original movie
credits are **Blender Foundation 2008, Janus Bager Kristensen 2013**, Creative
Commons Attribution 3.0. PNG artwork is Twemoji 15.1.0, licensed under
[CC BY 4.0](https://github.com/jdecked/twemoji/blob/v15.1.0/LICENSE-GRAPHICS).
The preparation script downloads its graphics license with the assets.

## Prepare and build

```sh
python3 scripts/export-comparison/prepare.py --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe
python3 scripts/export-comparison/prepare_pr459.py
cargo build --release --locked --manifest-path scripts/export-comparison/current/Cargo.toml
cargo build --release --locked --manifest-path scripts/export-comparison/pr459/runner/Cargo.toml
```

On Windows, the PR worker build also needs `--features vulkan`; the current
Windows production path enables Vulkan through target dependencies. Both need
the same matching FFmpeg 9 SDK and prepared Skia Vulkan package. The PR C++
build needs Vulkan SDK headers/glslc and native pkg-config metadata for the
matching SDK; adding `.pc` files must not swap headers or DLLs. Keep source
and build work isolated from the user's normal Windows checkout.

On macOS, use the same `ffmpeg-full` SDK for both sides:

```sh
export PKG_CONFIG_PATH=/opt/homebrew/opt/ffmpeg-full/lib/pkgconfig
export BSR_FFMPEG_PREFIXES=/opt/homebrew/opt/ffmpeg-full
export BSR_FFMPEG_LIBRARY_DIR=/opt/homebrew/opt/ffmpeg-full/lib
```

Current macOS uses its existing CPU Skia/VideoToolbox path. PR459 uses its
actual Metal/VideoToolbox path; the comparison does not add Metal to current.
Use separate macOS targets for the CPU and Metal Skia archives. The PR worker
source is extracted unchanged from its pinned commit; the five-line wrapper
only invokes its exported `worker_entry()`.

## Measure

```sh
python3 scripts/export-comparison/run.py \
  --current /path/to/current-export-comparison \
  --pr459 /path/to/bsr-pr459-worker \
  --scenario scripts/export-comparison/assets/scenario.json \
  --warmup scripts/export-comparison/assets/warmup.json \
  --output scripts/export-comparison/output/NEW-RUN \
  --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe \
  --platform macos --pairs 3
```

Use `--platform windows` for Windows and set `BSR_FFMPEG_LIBRARY_DIR` to that
SDK's DLL `bin` directory. Run one export at a time, without Vulkan validation,
API capture, profiler, or other GPU work. The driver strips profiling variables
and uses a lock to prevent concurrent benchmark drivers. Both versions receive
a separate exact 600-frame warmup, then three alternating pairs: current/PR,
PR/current, current/PR. Reuse the same generated input bytes on both machines.

The primary metric is process wall time through successful exit, including
startup and final media close. Version-specific internal timings remain
diagnostics. macOS also records `/usr/bin/time -l` CPU seconds and peak resident
memory. Compare the two versions within each machine; differing host hardware
prevents interpreting macOS-versus-Windows numbers as an OS-only effect.

Validation happens outside measured intervals: all decoded frames and the
complete PTS grid, HEVC format, copied audio payload, and eight samples at
2/30/60/120/180/240/298/299.9 seconds. Each sample must show both text and image
components closer to an independent CPU scene reference than to a missing
overlay. Per-pair image differences are retained; GPU rasterization and lossy
encoding need not be pixel-identical. FFmpeg CLI is used only for input
preparation and independent verification, never by the current producer.

Raw media/logs stay in ignored `assets/` and `output/` directories. Results
receipts preserve exact hashes, timings, output structure and validation data.
