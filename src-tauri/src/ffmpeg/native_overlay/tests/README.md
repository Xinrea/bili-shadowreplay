`translucent.png` is a deterministic 16×16, 8-bit RGBA PNG. Every pixel is
`[224, 48, 96, 128]`; no external image tool or media decoder is needed to make
it. The file contains only IHDR, one zlib-compressed IDAT, and IEND chunks.

The native overlay tests combine this half-transparent image with text fill,
outline and shadow over black, white and coloured backgrounds. They retain the
preview comparison's existing two-level rounding tolerance and separately
compare all output BGRA bytes with a real, matching FFmpeg 6/9 CLI oracle.
