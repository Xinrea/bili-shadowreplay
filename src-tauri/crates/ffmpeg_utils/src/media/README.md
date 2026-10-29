Structured media plans select input streams, copy or encode them, and configure
output containers. They contain no process arguments. The initial executor
supports one demuxed input and one atomic file output; a plan requiring more
inputs or outputs fails before output publication.

Input-side seeking and output-side trimming have separate policies. Copied
streams retain keyframe preroll; encoded streams pass through trim filters and
the native decoder/output clocks. Audio uses the same packet/sample clock and
frame queue as the fixed audio profiles. Video handles automatic rotation,
pixel-format negotiation, format changes, frame synchronization, encoder delay
and complete draining. Muxing retains stream metadata, chapters, dispositions,
codec tags and side data. Missing copied DTS uses the demux-side estimate from
fftools; coarse audio packet clocks use av_rescale_delta.

Encoder AVOptions are validated against the linked codec and unused options
are rejected. Hardware readiness opens the actual configured encoder, feeds
three synthetic frames and drains it. It is not inferred from registry
membership; the application retains candidate ordering and software fallback.
VAAPI devices and filter hardware frame pools are attached through libavutil.

The worker owns the input interrupt, codec/filter contexts and temporary output.
Dropping the future signals cooperative cancellation. Output trailer, buffered
write and close errors are propagated. Success publishes the file atomically
before the final progress callback. Progress comes from media timestamps rather
than parsing subprocess stderr.

The single-input tests compare stream headers, metadata and every packet with
the original commands on FFmpeg 6 and 9. The frame-clock tests and their exact
CLI oracle are documented in video_clock/README.md. Remaining graph and overlay
call sites migrate separately; this module does not parse arbitrary FFmpeg CLI
commands or fall back to a process.
