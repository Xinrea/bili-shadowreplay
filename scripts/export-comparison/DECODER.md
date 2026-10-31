# Automatic video decoding comparison

MediaPlan file video now requests FFmpeg's automatic decoding threads when
`MediaInput.decoder_threads` is absent. Explicit positive limits (including
single-thread image overlays) keep their behavior; audio retains its codec
default. Codec AVOptions still apply last. This uses libavcodec's internal
frame/slice threading, without moving the MediaPlan worker or changing GPU,
encoder, filter or synchronization settings.

The pinned FFmpeg 9.0.2 [frame threading implementation](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavcodec/pthread_frame.c#L834)
chooses the thread count from CPU availability when `thread_count=0`, with its
[automatic thread cap](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavcodec/pthread_internal.h#L22).
Frame threading buffers extra decoded frames, increasing memory use and startup
delay. The native regression covers a short 11-frame H.264/B-frame clip, both
MediaPlan executors, accurate input seek, output trim, copied AAC and EOF drain;
its encoded packets and timestamps must match a single-thread run.

`decoder.py` compares the prior single-thread behavior with the new production
default using **one release binary**. `--decoder-threads 1` sets the existing
structured MediaInput field for the baseline; omitting it tests the default.
No quality, GOP, bitrate, frame rate, render or encode settings change.

- Use the pinned 300 s / 1080p60 / 20 events-per-second scenario.
- Warm up 600 frames in each mode with stage diagnostics, verifying the actual
  decoder count and active thread type rather than trusting the requested value.
- Run three alternating single/automatic pairs **without profiling** for the
  primary process-wall result. Do not include diagnostic runs in these medians.
- After those pairs, run one full **profiled diagnostic pair** for stage
  attribution; it is supplemental, not a three-run stage average.
- Validate every output outside the timer with the existing full frame/PTS,
  AAC-packet and text/PNG sample checks. Only one GPU job runs per host.
- `--discard-verified-media` may remove newly generated files after validation
  and hashing; receipts and sample images remain.

```sh
python3 scripts/export-comparison/decoder.py \
  --current /path/to/current-export-comparison \
  --scenario scripts/export-comparison/assets/scenario.json \
  --warmup scripts/export-comparison/assets/warmup.json \
  --output /tmp/decoder-results --revision "$(git rev-parse HEAD)" \
  --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe --pairs 3
```

See [STAGES.md](STAGES.md) for API-wall versus device-timestamp semantics and
measurement limits. This comparison changes decoding only; it does not measure
an asynchronous VideoToolbox bridge or a revised compositor.
