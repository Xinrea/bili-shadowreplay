# Native overlay CPU benchmark

This harness measures the **real native renderer** before/after the image-cache and
frame-reuse changes, and profiles the **current production RGBA-to-FFmpeg stream**.
It does not use a browser, GPU, hardware encoder, or a replacement renderer.

## Run

Prerequisites: Python 3.10+, Rust/Cargo, Git history containing `c9b516a6` (the merged color-emoji implementation), and
already-installed `ffmpeg`/`ffprobe` with `libx264`. Use the native-overlay crate's
[platform build prerequisites](../../src-tauri/crates/native-overlay/README.md).
In particular Linux requires its documented Skia source-build environment and
system libraries; this script does not install them.

From the repository root:

```sh
python3 scripts/overlay-perf/run.py
```

Default: three repetitions per case, release binaries, Cargo offline. Build,
fixture generation, source extraction and validation are outside timed render
loops. On a typical development machine the cached-build run is intended to take
less than two minutes; slower CPUs and first builds can take longer. Each benchmark
subprocess has a 90-second safety timeout, not a universal performance guarantee.
No `pip` packages or media downloads are needed.

Useful options:

```sh
# Generate fixtures + immutable historical source without building/running.
python3 scripts/overlay-perf/run.py --prepare-only
# Quick end-to-end smoke run (not a statistically useful benchmark).
python3 scripts/overlay-perf/run.py --runs 1
# Diagnose/build historical core independently of current profiling APIs.
python3 scripts/overlay-perf/run.py --baseline-only --runs 1
# Explicitly allow Cargo downloads if the dependency cache is incomplete.
python3 scripts/overlay-perf/run.py --online
# Use a particular preinstalled toolchain.
python3 scripts/overlay-perf/run.py --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe
```

All generated fixtures, extracted source, executables, Cargo artifacts, MP4s and
JSON results are under ignored `output/`. The local benchmark `Cargo.lock` is
also ignored. On its first run the script seeds it from `src-tauri/Cargo.lock`
(the app pins a yanked transitive dependency of sidecar 0.0.3); it then preserves
that resolution and uses the **same lock for both binaries**. Later builds use
`--locked`. The result records the lock hash; retain the local lock alongside
results if comparing across machines or future dependency upgrades. No checkout,
branch switch, source patch, commit or upload is performed.

## Workload and validation

All cases use exactly 1920×1080, 60 fps, six seconds / 360 frames:

- `repeated`: four PNG paths shared across 100 image events.
- `unique`: 100 PNG paths across 100 image events.
- `mixed`: the repeated-image scene plus 24 moving Chinese/English/color-emoji text events, to measure layout and text raster work.

The events form two non-overlapping waves of 50, starting at 0 and 3 seconds,
each lasting 2.8 seconds. Images move diagonally, use 0.8 event opacity, and scale
64×64 source pixels to 96×96 output pixels. Generated PNG pixels and alpha are
deterministic and use only Python's standard library. Each new renderer starts
with empty caches; filesystem/font caches are warmed by the separate equality
pass. The first two are image-only workloads, so their text stages report zero.
The mixed case exercises the text stages as well; no shaping algorithm is changed.

The baseline is extracted with `git archive c9b516a6 src-tauri/crates/native-overlay`
into ignored output. The same harness scene and loop are compiled against that
crate, calling its allocating `frame`. Current uses `frame_into` and a single
reused buffer for the serial core benchmark. Core A/B timing is **unprofiled**
and alternating baseline/current order limits systematic run-order drift.
Renderer initialization is recorded separately from the rendering loop. Current's
one-time buffer allocation is outside that loop; baseline's per-frame allocations
are inside it. Neither core result includes FFmpeg or asynchronous backpressure.

Before timing, the harness checks byte-level RGBA SHA-256 equality at 14 sampled
frame indices per scene, including both wave boundaries, movement, and empty
frames. These are **sampled comparisons, not an exhaustive 360-frame equality
proof**, and hashes are computed outside timed benchmarks. Samples exercise
straight alpha and image-cache equivalence. Current image hit/miss counts and
preparation time are captured from the production stream; baseline has no such
instrumentation and no baseline stage numbers are invented.

Every current stream run uses the actual app `runner.rs` and `native_overlay.rs`
through Rust `#[path]` imports, with actual `ffmpeg_utils` and sidecar 0.0.3. Only
app-owned progress types/trait and danmu image structs/constants are stubbed.
The benchmark supplies already-scheduled native image events, passes no progress
reporter, and **does not validate the real danmu scheduler, task queue, database,
UI progress reporting or cancellation**. Those need application tests.

FFmpeg composites over a fixed dark background, software-encodes using
`libx264 -preset veryfast -crf 18 -threads 2`, uses one filter-complex thread,
and writes YUV420p H.264. Exact arguments are saved per run. It must feed exactly
360 frames / 2,985,984,000 raw bytes with exactly three production buffer
allocations. `ffprobe -count_frames` must confirm H.264, YUV420p, 1920×1080,
60 fps, 360 decoded frames, and six seconds. Each output replaces the prior run's
MP4 for that scene to avoid accumulating media.

## Results and correct interpretation

`output/results.json` contains raw repetitions, per-field medians, baseline/current
core speedup, RGBA sample hashes, `ffprobe` metadata, exact FFmpeg arguments, source
hashes, Git revisions/status, Rust/FFmpeg versions, platform and lock/fixture hashes.
Console output shows the core comparison and current stream median wall time.
No fixed speedup threshold is asserted: noise or a genuine regression must remain
visible. Do not compare runs with different build modes, hardware, source hashes,
encoders or fixture hashes without noting the differences.

Current stream measurements are explicitly opt-in via
`run_overlay_command_profiled`; ordinary production exports collect timings only
when debug logging is enabled (one summary per job, no per-frame logs):

| Field | What it measures |
| --- | --- |
| `initialization_ms` | Native renderer construction |
| `buffer_allocation_ms` | Allocate the fixed production buffer pool |
| `render.text_layout_ms` | Text layout work; zero for the first two image-only scenes |
| `render.text_raster_ms` | Text sprite raster work; exercised by the mixed scene |
| `render.image_prepare_ms` | Image file read, decode and scale on cache miss |
| `render.composition_ms` | Exclusive composition, not nested preparation/layout/raster |
| `render.readback_ms` | Skia surface to straight RGBA buffer readback |
| `buffer_wait_ms` | Producer wait for a recycled free buffer |
| `queue_send_wait_ms` | Producer send/backpressure wait |
| `queue_receive_wait_ms` | Consumer wait for a rendered frame |
| `pipe_write_ms` | Async raw-video write elapsed time, including downstream backpressure |
| `ffmpeg_drain_ms` | Post-feed process drain elapsed time when available |
| `wall_ms` | Overall current stream operation elapsed time |

These are **CPU-path elapsed durations**, not CPU utilization samples or GPU
measurements. Producer, pipe consumer and FFmpeg run concurrently; waiting stages
overlap and **must not be summed** to claim end-to-end time. In particular pipe
write time, process drain and stream wall time are **not pure encoder CPU time**.
This harness does not claim to isolate pure encoding cost.

An additional **separate run** per scene enables FFmpeg `-benchmark_all`. It sums
FFmpeg's reported `encode_video` and `flush_video` call intervals into
`encoder_profile_separate_run.ffmpeg_encoder_calls` (user/sys/real milliseconds).
These are FFmpeg-reported intervals, not an exclusive whole-job encoder budget:
its threads and decoder/filter activity can overlap. Instrumented wall time is
not included in the ordinary throughput medians.

The optimized production stream reuses three frame buffers (about 23.7 MiB of
pixel storage at 1080p). It is not zero-copy: the Skia readback and conversion from
premultiplied surface alpha to straight RGBA are still necessary, as is writing
those bytes into FFmpeg's pipe. Image cache reuse changes preparation/retention,
not the output alpha contract.

## Recorded local result

One macOS / Apple M2 Max run (FFmpeg 9.0.2, release builds, three repetitions,
median) against baseline `553e1b30` produced (its renderer tree is identical to
the merged `c9b516a6` baseline used by the script):

| Scene | Baseline core loop | Optimized core loop | Core speedup | Current stream wall |
| --- | ---: | ---: | ---: | ---: |
| 100 events, 4 repeated images | 631.3 ms | 595.3 ms | 1.06× | 2098.4 ms |
| 100 events, 100 unique images | 631.6 ms | 606.7 ms | 1.04× | 2099.6 ms |
| Repeated images + 24 text/emoji events | 786.7 ms | 750.9 ms | 1.05× | 2272.1 ms |

There are **50 simultaneous images in two waves**, not 100 simultaneous images.
These are modest core-loop gains, not measured end-to-end speedups: no historical
production-stream wall-time comparison is claimed. The 42 sampled RGBA hashes
(14 per scene) were identical. The repeated scene had 4 image cache misses;
the unique scene had 100. Every full stream used exactly 3 frame buffers instead
of one new buffer per frame.

For the repeated-image production stream, exclusive image preparation was 0.60 ms,
composition 148.7 ms and RGBA readback/conversion 549.4 ms across 360 frames.
Async pipe writes occupied 1994.2 ms **including downstream backpressure**, while
the whole operation took 2098.4 ms. For the mixed scene, text layout was 5.40 ms,
text rasterization 72.59 ms, composition 246.6 ms and readback 551.6 ms.
The separately instrumented encoder pass reported encode/flush real-time interval
sums of roughly 260 / 295 / 377 ms for repeated / unique / mixed scenes; these
are not exclusive encoder CPU budgets and must not be added to the other run's
times. Readback and transport remain more significant than repeated-image setup
in this small-image workload. No Windows/Linux, long-video, 4K, or RSS result is
claimed for this optimization run. Raw numbers/source hashes stay in the ignored
`output/results.json` for local inspection and the script regenerates them.

## Lightweight checks

```sh
python3 -m py_compile scripts/overlay-perf/run.py
python3 -m unittest discover -s scripts/overlay-perf -p 'test_*.py'
cargo fmt --manifest-path scripts/overlay-perf/Cargo.toml -- --check
```

The full `run.py` command is the integration validation; compiling this harness
alone is not a substitute for measuring/probing the produced stream. The source
includes production modules but is not a replacement for their application test
suite. Do not invoke `cargo test` here as an application test proxy.
