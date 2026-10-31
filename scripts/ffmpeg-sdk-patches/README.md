# FFmpeg Vulkan encoder cleanup

FFmpeg 9.0.2 can leave the last queued H.264/HEVC Vulkan picture's input and DPB
image views alive at device destruction. Draining the encoder does not release
every retained reference picture. This can report `VUID-vkDestroyDevice-device-05137` independently of Skia.

The included patch is the unchanged upstream fix
[`ddfa842088cd86bd48bc4533f10a4b5badcbd02d`](https://github.com/FFmpeg/FFmpeg/commit/ddfa842088cd86bd48bc4533f10a4b5badcbd02d).
It releases queued picture views before the base context discards their records,
and releases the packet buffer pool before the base context. The application
continues to use public FFmpeg APIs; it does not inspect private picture layouts.
This backport targets the unpatched `n9.0.2` source; verify whether a newer SDK
already contains the upstream fix before applying it.

## Backport to a source build

Extract an unmodified FFmpeg 9.0.2 source tree with LF line endings, then run:

```sh
python scripts/ffmpeg-sdk-patches/apply.py /path/to/ffmpeg-9.0.2 --check
python scripts/ffmpeg-sdk-patches/apply.py /path/to/ffmpeg-9.0.2
```

The offline helper verifies the upstream patch, source and patched-file SHA-256
values from `manifest.json`. It refuses unknown source contents and is idempotent
after a successful application. It patches source only: building a replacement
SDK still requires the intended compiler, complete FFmpeg configuration and all
third-party development dependencies.

Rebuild and validate the complete SDK before using it for exports. Replacing one
official `avcodec` DLL with a differently configured custom DLL has not been
validated and can remove decoders/encoders or mismatch dependencies. Keep a full
matching set of headers, import libraries and runtime DLLs in a separate prefix.
