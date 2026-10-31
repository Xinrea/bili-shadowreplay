# macOS cross-implementation stage measurements

`../compare_stages.py` runs the same 300-second 1080p60 fixture (16 text + 4 PNG
events/second), using production defaults and serial GPU jobs. It alternates
current/PR459 order, records external `/usr/bin/time -l` and once-per-second
thermal state, low-power mode, host CPU counters and available GPU utilization. All products undergo the
existing complete frame/PTS/audio and text/image sampling checks outside the
timed interval; only then are generated videos removed.

Current uses `MediaProfiler` on the existing MediaPlan worker. On macOS each
span has wall and calling-thread CPU totals, with nested exclusive attribution.
Every 600 frames a cumulative checkpoint is recorded (bounded to 512 per job).
Hardware codec/driver threads are absent from calling-thread CPU; process CPU
is separate. CPU timing is `null` on other platforms. Profiling remains opt-in.

PR459 is extracted using `../prepare_pr459.py`, pinned to
`c73c7aaff9b9dd9abb561c90a48f19b2d6374112`. Run `instrument_pr459.py EXTRACTED_DIR`
on a **fresh copy**, never on the unmodified comparison binary. The script
verifies every original source hash, adds `pr459_profile.hpp` and timing scopes,
and saves an instrumentation manifest. Export algorithms, queue capacity,
thread QoS, CPU activity helper, encoding settings and synchronization are
unchanged. `BSR_STAGE_PROFILE=/absolute/output.json` enables this separate
benchmark build's collector. Without the variable it provides an observer
control; it is still an instrumented build, not the unmodified PR binary.

For the separate CPU-activity ablation, extract another fresh tree and pass
`--without-cpu-activity` to the instrumenter. This removes the busy-loop helper
and records `without_cpu_activity: true` in the manifest. Label this binary
separately; it is not the default PR459 control. No other queue, QoS, codec or
synchronization setting changes.

Decode, render, encode and CPU-activity threads have independent root spans.
Their exclusive partitions are checked **per thread**. They overlap and must
not be summed into export latency. `gpu_completion_wait` is a host wait;
`composite_gpu` uses Metal command-buffer timestamps after existing completion,
without adding a wait, and is an overlapping GPU observation. Current's Metal
blit observation has the same caveat. Neither measures Skia's complete GPU
execution. Requested QoS is not effective scheduling priority or core placement.

Example (builds must finish before any timed job):

```sh
python3 scripts/export-comparison/prepare_pr459.py --output /tmp/pr459-profile
python3 scripts/export-comparison/profiling/instrument_pr459.py /tmp/pr459-profile
BSR_FFMPEG_PREFIXES=/opt/homebrew/opt/ffmpeg-full \
  SKIA_BINARIES_URL=file:///path/to/cached-skia-metal.tar.gz \
  cargo build --release --locked --manifest-path /tmp/pr459-profile/runner/Cargo.toml
xcrun clang++ -std=c++17 -O2 -framework Foundation -framework IOKit \
  scripts/export-comparison/profiling/telemetry.mm -o /tmp/bsr-telemetry
python3 scripts/export-comparison/compare_stages.py \
  --current /path/to/packaged-current/Contents/MacOS/bili-shadowreplay \
  --pr459 /tmp/pr459-profile/runner/target/release/bsr-pr459-worker \
  --telemetry /tmp/bsr-telemetry --output /tmp/stage-comparison \
  --revision CURRENT_REVISION --pairs 3 --controls 3
```

Build/package current with the procedure in `../HANDOFF.md`. Keep an unmodified
binary for each implementation as well. Clock instrumentation has nonzero cost;
compare controls before interpreting small differences. Thermal state and load
are context, not proof of DVFS, GPU frequency or a scheduler cause.
