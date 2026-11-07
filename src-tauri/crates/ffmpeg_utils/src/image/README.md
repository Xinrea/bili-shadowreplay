The emoji normalizer accepts one local regular file, at most 8 MiB, identified
by PNG/APNG, JPEG, GIF or WebP magic. It feeds a bounded memory buffer to FFmpeg
through custom AVIO without an input URL. The former format whitelist and file
protocol whitelist remain in force; playlists, URLs and filename sequences
cannot supply additional inputs. Stream discovery does not run an implicit
unrestricted decoder before the image decoder's limits are installed.

The source canvas and every decoded frame must contain at most 4,194,304 pixels.
Decoder, filter graph and PNG encoder each use one thread. The first frame is
scaled to fit 512×512 without enlargement and encoded as RGBA PNG. A temporary
file is atomically published after success. Errors, timeout and cancellation
retain the prior output. Animated WebP requiring a demuxer outside the legacy
whitelist remains unsupported, allowing the application to fall back to text.

PNG chunk lengths are checked against the encoded input. Compressed ICC, text
and international-text metadata are validated with a bounded streaming zlib
decoder before FFmpeg can expand them with its unlimited metadata buffer. Their
combined expansion is limited to 8 MiB; this is a stricter normalization policy
than the former command. Normal ICC data and original input chunks are retained.

This runs inside the application process. FFmpeg's `av_max_alloc` controls a
process-global allocation limit, so the normalizer does not change it. These
input, dimension, metadata and thread limits constrain work; they are **not a
hard per-job or process memory limit**. The ten-second timeout cancels the
future, interrupts cooperative input work and prevents output publication.
An executing native codec/filter call is not forcibly terminated, and a native
crash or allocation failure no longer has the isolation of a separate process.
