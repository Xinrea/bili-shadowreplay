The decoder clock and output frame scheduler implement the behavior described
by the official FFmpeg [6.1 decoder](https://github.com/FFmpeg/FFmpeg/blob/n6.1.3/fftools/ffmpeg_dec.c),
[9.0 decoder](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/fftools/ffmpeg_dec.c),
[6.1 filter scheduler](https://github.com/FFmpeg/FFmpeg/blob/n6.1.3/fftools/ffmpeg_filter.c),
and [9.0 filter scheduler](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/fftools/ffmpeg_filter.c).
Auto mode also follows the output policy selection in
[ffmpeg_mux_init.c](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/fftools/ffmpeg_mux_init.c).

The independent CLI oracle generates six uniquely colored frames, encodes PNG
packets to `framehash`, and compares each source frame's identity, PTS and
duration against the Rust scheduler. Its 25 combinations cover 10/25/30 fps
CFR, VFR, passthrough, missing time intervals, initial offsets, EOF median tails,
and output-side `-ss .15 -t .35` half-tick rounding. Thresholds are exact; there
is no duration or pixel tolerance. It requires the CLI's `lavfi` input, `png`
encoder and `framehash` muxer. The remaining five tests run with only libraries.

The scheduler uses C `llrint`/`llrintf` nearest-even rounding and FFmpeg's added
fractional-position precision. It keeps estimated decoder durations separate
from real frame durations. Repeated output frames hold references to pixels,
and batches are lazy so a long gap is cancellable without preallocating a large
frame list. The CLI's default excessive-duplication threshold is retained.
