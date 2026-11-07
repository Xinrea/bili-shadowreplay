`seek.mp4` is an 11-frame generated test pattern: 96×64, 10 fps, SAR 4:3,
BT.709 limited range, H.264 with B-frames. It contains no third-party content.
Tests can exercise decoding, accurate seeking and JPEG encoding without a CLI
or an H.264 encoder installed. Optional oracles use the locally linked SDK's
matching CLI to compare the complete JPEG bytes.

Reproduce with:

```sh
ffmpeg -f lavfi -i testsrc2=size=96x64:rate=10:duration=1.1 \
  -vf setsar=4/3 -c:v libx264 -preset medium -crf 18 -g 10 -bf 3 \
  -color_range tv -colorspace bt709 -color_primaries bt709 -color_trc bt709 \
  -y seek.mp4
```
