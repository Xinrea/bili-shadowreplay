# HLS probe fixtures

These small MPEG-TS files contain 0.4 seconds of generated black video at 25 FPS.
They exercise the recorder's actual download, probe, archive and stream-change
paths without requiring FFmpeg CLI executables while the tests run.

Generated using FFmpeg 9.0.2:

```sh
ffmpeg -f lavfi -i color=black:s=64x48:r=25:d=0.4 -c:v libx264 -preset ultrafast -g 10 -f mpegts probe-h264.ts
ffmpeg -f lavfi -i color=black:s=96x64:r=25:d=0.4 -c:v libx264 -preset ultrafast -g 10 -f mpegts probe-h264-resized.ts
ffmpeg -f lavfi -i color=black:s=64x48:r=25:d=0.4 -c:v mpeg2video -g 10 -f mpegts probe-mpeg2.ts
```

`probe-h264-no-parameters.ts` is a copy of `probe-h264.ts` whose two Annex B
SPS/PPS NAL headers were replaced with filler NAL headers. Keeping the packet
layout and timestamps models a live fragment that has a stream and duration
but no decodable video parameters:

```python
import pathlib
import re

source = pathlib.Path("probe-h264.ts").read_bytes()
fragment, count = re.subn(b"\x00\x00\x01[\x67\x68]", b"\x00\x00\x01\x0c", source)
assert count == 2
pathlib.Path("probe-h264-no-parameters.ts").write_bytes(fragment)
```

The paired fMP4 fixtures used by the same tests are the existing
`src-tauri/tests/video/init.m4s` and `segment.m4s`. Their absolute media
timestamps must not replace the HLS playlist's `EXTINF` duration.
