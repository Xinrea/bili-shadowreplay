# Asynchronous GPU encoder handoff

The production MediaPlan / Skia Vulkan path now uses a three-frame bounded
VideoToolbox handoff on macOS, without a feature flag. Windows keeps the direct
Vulkan Video frame/timeline path. `HardwareEncoderFrames::process` returns ready
frames in submission order; `flush` drains the remaining frames before encoder
EOF. Both are called on the existing media worker.

On macOS each pending frame advances from Vulkan output to an asynchronous Metal
blit and then a ready VideoToolbox IOSurface. The worker checks readiness once per
advance, without spinning. It waits only when the three-frame handoff queue is
full or at EOF. Each pending frame retains its source AVFrame, destination frame
and Metal command/texture views until GPU completion, including cancellation,
errors and early drop. The compositor's existing three-frame limit and the media
processor's eight-frame safety limit are unchanged. GPU pixels are never mapped
or read back by the handoff; software-decoded NV12 input still uploads once.

This deliberately uses Vulkan timeline readiness and Metal command completion on
the owning worker. FFmpeg 9.0.2's [Vulkan frame allocation](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavutil/hwcontext_vulkan.c#L2454)
does not attach a Metal export declaration to its semaphores. The
[Vulkan Metal export contract](https://docs.vulkan.org/refpages/latest/refpages/source/VkExportMetalObjectCreateInfoEXT.html)
requires that declaration during object creation. The adapter therefore does not
export FFmpeg's undeclared semaphores or change the SDK to enable direct Metal
event waits.

`EncoderFrameHandoff::Blocking` preserves a per-job validation baseline. It does
not change Skia's asynchronous submission or decoder/encoder settings. The
comparison runner's `--blocking-handoff` selects it; ordinary exports use the
asynchronous default. Windows passes Vulkan frames through in either setting.

## Validation and measurement

- Native GPU tests cover 1, 2, 3 and 7-frame tails, H.264 and HEVC, exact decoded
  PTS/duration agreement, queue bounds and complete drain. Separate tests cancel
  or inject an error after real handoffs, checking original output preservation
  and staging cleanup. Existing moving-video/audio/content tests still apply.
- `handoff.py` uses the pinned five-minute 1080p60 fixture with 20 events/s,
  default HEVC/speed/10 Mbps/GOP 120, and copied AAC. Both variants use automatic
  video decoding threads.
- macOS uses one release binary, blocking versus default asynchronous handoff.
  Windows uses a separately retained, SHA-256-pinned previous binary as the
  baseline, since its direct Vulkan adapter is not the optimization target.
- Each host warms up both variants, runs three alternating pairs with profiling
  off, then one diagnostic pair with profiling on. The headline is the median
  of the three unprofiled full exports. Every output is validated outside the
  timer for frame count/PTS, copied AAC and text/PNG content samples.
- New exclusive spans distinguish `vt_vulkan_poll`, `vt_vulkan_wait`,
  `vt_metal_submit`, `vt_metal_poll`, and `vt_metal_wait`. These are host API wall
  intervals, including blocking. Metal GPU blit remains a separate overlapping
  device-timestamp observation. See [STAGES.md](STAGES.md).

```sh
python3 scripts/export-comparison/handoff.py \
  --current /path/to/current-export-comparison \
  --scenario scripts/export-comparison/assets/scenario.json \
  --warmup scripts/export-comparison/assets/warmup.json \
  --output /tmp/handoff-results --revision "$(git rev-parse HEAD)" \
  --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe --pairs 3
```

On Windows also pass `--baseline /path/to/previous-export-comparison.exe` and
`--baseline-revision COMMIT`. `--discard-verified-media` removes only this run's
new media after successful validation and hashing, retaining receipts/samples.
