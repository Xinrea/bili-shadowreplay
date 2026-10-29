# Native live recording fixture

`live.flv` is a six-second generated test pattern with 64×48 H.264 video,
B-frames, one-second keyframe intervals, and 48 kHz mono AAC sine audio. No
third-party recording is included. Generated with FFmpeg 9.0.2:

```sh
ffmpeg -f lavfi -i testsrc2=size=64x48:rate=25:duration=6 \
  -f lavfi -i sine=frequency=523:sample_rate=48000:duration=6 \
  -c:v libx264 -preset ultrafast -g 25 -bf 2 -c:a aac -b:a 64k \
  -shortest live.flv
```

Normal tests read this fixture directly without requiring the CLI.
