`subtitle-black.mp4` contains generated black video (160×90, 10 fps, 1.1 s,
H.264 with B-frames) and a generated 440 Hz mono AAC tone. It contains no
third-party media. It lets native libass rendering tests verify color, style
overrides, cue boundaries and unchanged audio without a CLI installed.

Reproduce with:

```sh
ffmpeg -f lavfi -i color=black:size=160x90:rate=10:duration=1.1 \
  -f lavfi -i sine=frequency=440:sample_rate=48000:duration=1.1 \
  -c:v libx264 -crf 18 -g 10 -bf 2 -c:a aac -b:a 64k -y subtitle-black.mp4
```
