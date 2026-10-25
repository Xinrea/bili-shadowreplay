#!/usr/bin/env python3
"""Fetch pinned public fixtures, build the isolated Rust probe, validate and encode.

No Python packages, system font installs, GUI, or application build are required.
Generated files stay under this directory's ignored assets/ and output/ folders.
"""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parent
OUT = ROOT / "output"
ASSETS = ROOT / "assets"
EMOJI_REV = "e20cbc2bbec1926686be9f9bee7d1d2cfa1fea0e"
TEXT_REV = "f8d157532fbfaeda587e826d4cd5b21a49186f7c"
RAW = "https://raw.githubusercontent.com"
FIXTURES = [
    ("Noto-COLRv1.ttf", f"{RAW}/googlefonts/noto-emoji/{EMOJI_REV}/2D/fonts/Noto-COLRv1.ttf",
     "b8e25ea68db82f9e4d0aee921f4420be2be39887bd5c893a2ad98710531f9d0c"),
    ("NotoSansCJKsc-Regular.otf", f"{RAW}/notofonts/noto-cjk/{TEXT_REV}/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf",
     "2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b"),
    ("EMOJI-OFL.txt", f"{RAW}/googlefonts/noto-emoji/{EMOJI_REV}/2D/fonts/LICENSE",
     "6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2"),
    ("TEXT-OFL.txt", f"{RAW}/notofonts/noto-cjk/{TEXT_REV}/Sans/LICENSE",
     "6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2"),
    ("emoji-test.txt", "https://unicode.org/Public/emoji/16.0/emoji-test.txt",
     "24f0c534e86cf142e2496953e8f0e46a3e702392911eddcd29c6cced85139697"),
    ("UNICODE-LICENSE.txt", "https://www.unicode.org/license.txt",
     "e7a93b009565cfce55919a381437ac4db883e9da2126fa28b91d12732bc53d96"),
]


def run(args, **kwargs):
    if kwargs.get("text"):
        # Cargo JSON and Rust/FFmpeg diagnostics use UTF-8, not Windows' locale.
        kwargs.setdefault("encoding", "utf-8")
        kwargs.setdefault("errors", "replace")
    try:
        return subprocess.run([str(a) for a in args], check=True, **kwargs)
    except subprocess.CalledProcessError as error:
        # Cargo's rendered diagnostics travel on JSON stdout; do not hide them
        # merely because stdout is captured to locate the executable artifact.
        if isinstance(error.stdout, str):
            for line in error.stdout.splitlines():
                try:
                    item = json.loads(line)
                except json.JSONDecodeError:
                    print(line)
                    continue
                if item.get("reason") == "compiler-message":
                    print(item["message"].get("rendered", ""))
        raise


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def fetch_assets():
    ASSETS.mkdir(exist_ok=True)
    for name, url, expected in FIXTURES:
        path = ASSETS / name
        if path.exists() and hashlib.sha256(path.read_bytes()).hexdigest() == expected:
            continue
        temporary = path.with_suffix(path.suffix + ".download")
        print(f"Downloading {name}", flush=True)
        run(["curl", "--fail", "--location", "--retry", "2", "--max-time", "120", url, "-o", temporary])
        actual = hashlib.sha256(temporary.read_bytes()).hexdigest()
        require(actual == expected, f"Checksum mismatch for {name}: {actual}")
        temporary.replace(path)


def probe(path):
    return json.loads(run([
        "ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
        "stream=codec_name,width,height,nb_frames,duration,avg_frame_rate", "-of", "json", path,
    ], capture_output=True).stdout)["streams"][0]


def pixels(path):
    stream = probe(path)
    result = run(["ffmpeg", "-v", "error", "-i", path, "-frames:v", "1",
                  "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"], capture_output=True)
    return stream["width"], stream["height"], result.stdout


def validate_alpha():
    source = OUT / "sample-0.png"
    width, height, data = pixels(source)
    raw_path = OUT / "sample-0.rgba"
    raw = raw_path.read_bytes()
    require(len(raw) == len(data), "Raw readback dimensions differ from PNG")
    readback_error = max(abs(a - b) for a, b in zip(raw, data))
    require(readback_error <= 1, f"RGBA readback differs from PNG: {readback_error}")
    opaque = colored = partial = 0
    for i in range(0, len(data), 4):
        r, g, b, a = data[i:i + 4]
        opaque += a == 255
        partial += 0 < a < 255
        colored += a > 200 and max(r, g, b) - min(r, g, b) > 50
    require(opaque > 100 and partial > 100 and colored > 100, "Missing color or alpha coverage")
    edge = [data[(y * width + x) * 4 + 3] for y in range(height) for x in range(width)
            if x in (0, width - 1) or y in (0, height - 1)]
    require(max(edge) == 0, "Sprite outline/shadow clipped at the allocation edge")
    errors = []
    # Feed the actual Rust rgba() readback to FFmpeg. Compare lossless output
    # against the independently encoded PNG, avoiding H.264 quantization.
    for background in (0, 255):
        for opacity in (0.0, 0.8, 1.0):
            name = OUT / f"alpha-{background}-{opacity}.png"
            graph = (f"[0:v]format=rgba[bg];[1:v]format=rgba,colorchannelmixer=aa={opacity}[fg];"
                     "[bg][fg]overlay=format=rgb:alpha=straight:shortest=1,format=rgba[out]")
            run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i",
                 f"color=c=0x{background:02x}{background:02x}{background:02x}:s={width}x{height}:d=1,format=rgba",
                 "-f", "rawvideo", "-pixel_format", "rgba", "-video_size", f"{width}x{height}",
                 "-framerate", "1", "-i", raw_path, "-filter_complex", graph, "-map", "[out]", "-frames:v", "1",
                 "-update", "1", name], timeout=30)
            _, _, actual = pixels(name)
            worst = 0
            for i in range(0, len(data), 4):
                a = data[i + 3] / 255.0 * opacity
                for channel in range(3):
                    expected = round(data[i + channel] * a + background * (1 - a))
                    worst = max(worst, abs(actual[i + channel] - expected))
            require(worst <= 3, f"Alpha blend mismatch: background={background}, opacity={opacity}, error={worst}")
            errors.append({"background": background, "opacity": opacity, "max_channel_error": worst})
    result = {"opaque_pixels": opaque, "partial_alpha_pixels": partial, "colored_pixels": colored,
              "transparent_border": True, "raw_readback_max_channel_error": readback_error,
              "straight_alpha_checks": errors}
    (OUT / "alpha.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    return result


def encode(binary, count):
    video = OUT / f"demo-{count}.mp4"
    graph = ("[0:v]format=rgba[bg];[bg][1:v]overlay=format=auto:alpha=straight:shortest=1,"
             "format=yuv420p[out]")
    command = ["ffmpeg", "-hide_banner", "-y", "-f", "lavfi", "-i",
               "color=c=0x252a34:s=1920x1080:r=60:d=6,drawbox=x=960:y=0:w=960:h=1080:color=0xeeeeee:t=fill",
               "-f", "rawvideo", "-pixel_format", "rgba", "-video_size", "1920x1080",
               "-framerate", "60", "-i", "pipe:0", "-filter_complex", graph, "-map", "[out]",
               "-an", "-c:v", "libx264", "-preset", "veryfast", "-crf", "18",
               "-frames:v", "360", "-movflags", "+faststart", str(video)]
    start = time.perf_counter()
    with (OUT / f"ffmpeg-{count}.log").open("wb") as log:
        encoder = subprocess.Popen(command, stdin=subprocess.PIPE, stderr=log, stdout=subprocess.DEVNULL)
        producer = None
        try:
            producer = subprocess.Popen([str(binary), "stream", str(count)], stdout=encoder.stdin)
            encoder.stdin.close()
            require(producer.wait(timeout=180) == 0, "Skia producer failed")
            require(encoder.wait(timeout=60) == 0, f"FFmpeg failed; see ffmpeg-{count}.log")
        finally:
            for process in (producer, encoder):
                if process is not None and process.poll() is None:
                    process.kill()
                    process.wait()
    elapsed = time.perf_counter() - start
    metadata = probe(video)
    require(metadata["width"] == 1920 and metadata["height"] == 1080, "Unexpected video dimensions")
    require(metadata["avg_frame_rate"] == "60/1" and int(metadata["nb_frames"]) == 360, "Missing video frames")
    require(abs(float(metadata["duration"]) - 6.0) < 0.001, "Incorrect video duration")
    # Actually decode every frame, rather than trusting only the encoder status.
    run(["ffmpeg", "-v", "error", "-xerror", "-i", video, "-f", "null", "-"], timeout=60)
    run(["ffmpeg", "-v", "error", "-y", "-ss", "2", "-i", video,
         "-frames:v", "1", "-update", "1", OUT / f"video-{count}.png"], timeout=30)
    # Check the actual encoded cue boundaries, not just the scheduling branch.
    # Danmaku stays above this crop. Four frames cover each side of [0,2.5)
    # and [3,6), including the empty subtitle interval.
    boundary_frames = (149, 150, 179, 180)
    selected = "+".join(f"eq(n,{frame})" for frame in boundary_frames)
    decoded = run(["ffmpeg", "-v", "error", "-i", video, "-vf",
                   f"select='{selected}',crop=1920:180:0:900", "-fps_mode", "passthrough",
                   "-frames:v", "4", "-pix_fmt", "rgb24", "-f", "rawvideo", "pipe:1"],
                  capture_output=True, timeout=60).stdout
    frame_bytes = 1920 * 180 * 3
    require(len(decoded) == 4 * frame_bytes, "Cue boundary frames missing")
    boundary_ink = []
    for offset, frame in enumerate(boundary_frames):
        data = decoded[offset * frame_bytes:(offset + 1) * frame_bytes]
        backgrounds = (data[100 * 3:100 * 3 + 3], data[1800 * 3:1800 * 3 + 3])
        ink = 0
        for y in range(180):
            # Avoid the encoded dark/light boundary itself.
            for x in (*range(16, 944), *range(976, 1904)):
                i = (y * 1920 + x) * 3
                bg = backgrounds[x >= 960]
                ink += max(abs(data[i + c] - bg[c]) for c in range(3)) > 30
        require((ink > 500) if frame in (149, 180) else (ink == 0),
                f"Subtitle visibility incorrect at frame {frame}: {ink} foreground pixels")
        boundary_ink.append({"frame": frame, "foreground_pixels": ink})
    return {"sprites": count, "end_to_end_seconds": elapsed, "end_to_end_fps": 360 / elapsed,
            "video": metadata, "bytes": video.stat().st_size, "cue_boundaries": boundary_ink}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fetch-only", action="store_true")
    parser.add_argument("--skip-video", action="store_true")
    args = parser.parse_args()
    for tool in ("cargo", "curl", "ffmpeg", "ffprobe"):
        require(shutil.which(tool), f"Required tool not found: {tool}")
    OUT.mkdir(exist_ok=True)
    fetch_assets()
    if args.fetch_only:
        return
    manifest = ROOT / "Cargo.toml"
    build = run(["cargo", "build", "--release", "--locked", "--message-format=json",
                 "--manifest-path", manifest], stdout=subprocess.PIPE, text=True)
    artifacts = [json.loads(line) for line in build.stdout.splitlines()]
    executables = [artifact["executable"] for artifact in artifacts
                   if artifact.get("reason") == "compiler-artifact" and artifact.get("executable")
                   and artifact["target"]["name"] == "color-emoji-prototype"]
    require(len(executables) == 1, "Cargo did not report the prototype executable")
    binary = Path(executables[0])
    run(["cargo", "test", "--release", "--locked", "--manifest-path", manifest])
    run([binary, "fixtures"], timeout=120)
    alpha = validate_alpha()
    videos = [] if args.skip_video else [encode(binary, count) for count in (24, 100)]
    report = {
        "platform": platform.platform(), "machine": platform.machine(),
        "rust": run(["rustc", "--version"], capture_output=True, text=True).stdout.strip(),
        "ffmpeg": run(["ffmpeg", "-version"], capture_output=True, text=True).stdout,
        "assets": [{"file": name, "url": url, "sha256": digest} for name, url, digest in FIXTURES],
        "fixtures": json.loads((OUT / "fixtures.json").read_text(encoding="utf-8")), "alpha": alpha,
        "videos": videos,
        "streams": [json.loads((OUT / f"stream-{v['sprites']}.json").read_text(encoding="utf-8")) for v in videos],
    }
    (OUT / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"fixtures": report["fixtures"], "videos": videos, "streams": report["streams"]}, ensure_ascii=False, indent=2))
    print(f"Results: {OUT}")


if __name__ == "__main__":
    main()
