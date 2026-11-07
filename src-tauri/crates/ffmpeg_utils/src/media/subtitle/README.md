The native subtitle stage follows FFmpeg 6.1/9's `do_subtitle_out` in
[ffmpeg_enc.c](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/fftools/ffmpeg_enc.c)
and the decoder's
[subtitle path](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/fftools/ffmpeg_dec.c).
The actual library encoding API is `avcodec_encode_subtitle`.

Decoded subtitle rectangles have an explicit RAII owner because ffmpeg-next's
`Subtitle` type has no destructor. The encoder receives a NUL-terminated copy
of the decoder's ASS header. ASS rectangles are encoded individually; DVB uses
the CLI's draw/clear packet pair. Encoding uses a 1 MiB scratch buffer, then
allocates an output packet for the exact payload size before passing it to the
muxer. Queued small captions therefore do not retain 1 MiB allocations each.

Subtitle timing intentionally differs from audio/video filter trimming:

- Output `-ss` drops events starting before the target. Output `-t` checks the
  event start minus that target, but emitted packet PTS retain the original
  normalized event PTS. Display spans crossing an endpoint are not shortened.
- Input `-t` does not trim encoded subtitle events. FFmpeg signals its input
  duration to stream copy and audio/video trim filters; subtitles bypass those
  filters. Input seek still controls demuxing and common timestamp offsets.
- Events with no PTS are skipped, as in the CLI's default non-`-xerror` path.
  Decode, encode, allocation and downstream mux errors propagate normally.

Tests cover UTF-8, inline styling, original ASS headers and styles, precise
PTS/DTS/duration/payload comparisons against both SDKs' CLI, input/output seek
and duration, the packet-size limit, cancellation and downstream errors.
