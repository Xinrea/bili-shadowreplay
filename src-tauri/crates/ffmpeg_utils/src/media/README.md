Structured media plans select input streams, copy or encode them, and configure
output containers. They contain no process arguments. File jobs and shared
complex graphs support explicit stream routes, generated color/silence, bounded
raw video and multiple outputs. Each filter input binds one selected stream;
direct AllAudio routes expand in input order without another probe.

Input-side seeking and output-side trimming have separate policies. Copied
streams retain keyframe preroll; encoded streams pass through trim filters and
the native decoder/output clocks. Audio uses the same packet/sample clock and
frame queue as the fixed audio profiles. Video handles automatic rotation,
pixel-format negotiation, format changes, frame synchronization, encoder delay
and complete draining. Muxing retains stream metadata, chapters, dispositions,
codec tags and side data. Missing copied DTS uses the demux-side estimate from
fftools; coarse audio packet clocks use av_rescale_delta.

Audio encoding owns format negotiation, sample framing, duration cutoff and
encoder draining independently of file decoding. File and graph callers can
feed the same encoder stage directly after their respective filter sinks.

The shared scheduler follows buffer-source demand. Streams awaiting their turn
retain compressed packets rather than eagerly decoding whole inputs. Compressed
queues are bounded by 64 MiB/16384 packets and decoded queues by 64 MiB/1024
frames; these queue limits are not a hard budget for codec/filter allocations.
Input, graph and encoder EOF are handled separately. All outputs close before
publication; each rename is atomic, with preflight checks for predictable errors.
Multiple files do not form a filesystem transaction. Segment callers retain
their partial-file guards, and resource leases outlive worker completion through
publication so scratch directories cannot disappear too early.

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

Tests compare stream headers, metadata, dispositions and every packet with the
original single-input and complex-graph commands on FFmpeg 6 and 9. The frame
clock's exact CLI oracle is documented in video_clock/README.md. This module
does not parse arbitrary FFmpeg CLI commands or fall back to a process.
