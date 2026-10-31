# Production GPU stage measurements

`current-export-comparison --measure-stages --require-gpu` attaches a per-job
collector to the production MediaPlan graph worker. Without the flag, the same
export binding runs without timers or a collector. Capability probing is outside
the session. No decoder/encoder/filter options or synchronization policy change.

`stages.py` uses the existing complete 300 s, 1920×1080/60 fps fixture (4,800 text
and 1,200 PNG events), and a 10 s warmup for each mode. Three pairs alternate
profiling off/on on the **same release binary**. Only one GPU job runs per host.
Each output is checked with `verify.py`: all decoded frame timestamps, copied AAC
packet hashes and sampled text/image composition. Validation is outside timing.
`--discard-verified-media` removes only newly generated media after validation and
SHA-256 recording; reports and samples remain.

Example (all paths refer to prepared release binaries / assets):

```sh
python3 scripts/export-comparison/stages.py \
  --current /path/to/current-export-comparison \
  --scenario scripts/export-comparison/assets/scenario.json \
  --warmup scripts/export-comparison/assets/warmup.json \
  --output /tmp/stage-results --revision "$(git rev-parse HEAD)" \
  --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe --pairs 3
```

## Interpretation

- `spans`: monotonic **worker API wall time**, including time blocked in native
  calls. Nested child time is subtracted in `exclusive_ms`; exclusive totals sum
  to `worker.inclusive_ms`. These are not process CPU-seconds. `process_frame`,
  `encoder_frame_bridge`, `initialize_graph`, etc. are inclusive parents.
- `observations`: supplemental overlapping intervals; **do not add these to the
  span partition**. Skia scene time includes text/image preparation. Skia flush
  includes CPU submission and any blocking within the driver. Metal preparation
  includes creation of texture views; its wait interval overlaps device work.
- `metal_blit_gpu`: Metal `GPUStartTime` to `GPUEndTime`, read only after command
  completion, detected by polling or the bridge's required wait. It adds no extra synchronization. Missing
  or invalid device timestamps are omitted (check the observation count).
- Skia Vulkan, libplacebo, upload and hardware encoder **GPU execution time is
  unavailable** in this instrumentation. The bound Skia `FlushInfo` API does not
  expose its statistics callback safely, and FFmpeg's libplacebo filter does not
  expose renderer dispatch statistics here. Host submission markers would not
  isolate GPU execution. `libplacebo_pull`, `nv12_upload`, `encode_send` and
  `encode_receive` therefore remain API wall times, including internal waits.
- `vt_vulkan_wait` is host waiting for both NV12 planes before Metal import;
  it can reflect earlier upload/Skia/composition work and queue scheduling, not
  exclusively libplacebo shader execution. `frame_slot_wait` is retirement of
  a bounded in-flight slot and can include encoding backpressure.
- The asynchronous VideoToolbox handoff separates nonblocking `vt_vulkan_poll`,
  `vt_metal_submit` and `vt_metal_poll` from blocking `vt_vulkan_wait` and
  `vt_metal_wait`. A missing wait span means that wait did not occur in the
  measured session. See [HANDOFF.md](HANDOFF.md) and the
  [handoff benchmark](REPORT-handoff.md).
- p50/p95 are per-call histogram **upper bounds**, with eight bins per power of
  two; they are not exact percentiles or per-frame end-to-end latency. Storage
  stays bounded for long recordings. Counts include unsuccessful API attempts
  (`EAGAIN`/EOF) except the explicit `process_frame` count.
- The worker session includes setup, flush and native object teardown. External
  process wall also includes loading, scene parsing, capability probe and file
  publication. `worker` exclusive time is otherwise uninstrumented worker work
  plus profiler bookkeeping; do not call it idle time.

The collector is worker-local; async jobs and capability probes do not share
state. Unit tests check nested exclusive attribution, session isolation, disabled
collection and cleanup through panic. The driver checks frame counts and the
exclusive partition on every measured run. Timed control runs quantify observer
overhead rather than assuming profiling is free.
