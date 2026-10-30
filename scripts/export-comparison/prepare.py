#!/usr/bin/env python3
"""Prepare one continuous native-1080p60 movie and an identical dense scene."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parent
MOVIE_URL = "https://download.blender.org/demo/movies/BBB/bbb_sunflower_1080p_60fps_normal.mp4.zip"
MOVIE_FILE = "bbb_sunflower_1080p_60fps_normal.mp4"
TWEMOJI = "https://raw.githubusercontent.com/jdecked/twemoji/v15.1.0"
EMOJI = ["1f600", "1f602", "1f60d", "1f914", "1f973", "1f62d", "1f44d", "1f525", "1f389", "2764", "1f680", "1f431"]


def sha256(path):
    value = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def run(args):
    subprocess.run([str(arg) for arg in args], check=True)


def download(url, path):
    if not path.exists():
        temporary = path.with_suffix(path.suffix + ".download")
        run(["curl", "-fL", "--retry", "2", "--max-time", "600", url, "-o", temporary])
        temporary.replace(path)
    return {"url": url, "sha256": sha256(path), "bytes": path.stat().st_size}


def ass_time(seconds):
    centiseconds = round(seconds * 100)
    return f"{centiseconds // 360000}:{centiseconds // 6000 % 60:02}:{centiseconds // 100 % 60:02}.{centiseconds % 100:02}"


def scene(directory, seconds):
    # Use one pre-laid-out scene for both implementations: layout/download is
    # outside timing, while parsing, render, real decode/encode/mux remain timed.
    header = """[Script Info]
Title: Five-minute dense text and image danmaku
ScriptType: v4.00+
PlayResX: 1280
PlayResY: 720
[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Arial,36,&H33FFFFFF,&H33FFFFFF,&H33000000,&H33000000,-1,0,0,0,100,100,0,0,1,1,0,7,0,0,0,1
[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
"""
    messages = [
        "这波操作太精彩了", "前方高能注意看", "今天的直播很开心", "哈哈哈哈笑出声了",
        "这个画面可以反复看", "一路看到这里的朋友", "Welcome to the stream", "What a wonderful moment",
        "弹幕护体继续加油", "来了来了大家一起看", "太可爱了 😀👍", "好耶我们继续出发 🚀",
    ]
    lines, images, arrivals = [], [], []
    for second in range(seconds):
        for slot in range(20):
            index = second * 20 + slot
            start, end = index / 20.0, index / 20.0 + 8.0
            lane = index % 24
            y = 8.0 + lane * 29.0
            if slot % 5 == 4:
                image_index = (second * 4 + slot // 5) % len(EMOJI)
                images.append({
                    "path": f"emoji-{image_index:02}.png", "start": start, "end": end,
                    "from": {"x": 1970.0, "y": y * 1.5},
                    "to": {"x": -420.0, "y": y * 1.5},
                    "width": 54.0, "height": 54.0, "opacity": 0.8,
                })
                kind = "image"
            else:
                text = f"用户{index % 997:03}：{messages[index % len(messages)]} #{index:04}"
                lines.append(f"Dialogue: 0,{ass_time(start)},{ass_time(end)},Default,,0,0,0,,{{\\move(1313.333,{y:.3f},-280,{y:.3f})}}{text}\n")
                kind = "text"
            arrivals.append({"index": index, "time": start, "kind": kind})
    ass_path = directory / ("danmaku.ass" if seconds == 300 else "warmup.ass")
    ass_path.write_text(header + "".join(lines), encoding="utf-8")
    counts = {"text": len(lines), "image": len(images), "total": len(arrivals)}
    assert counts == {"text": seconds * 16, "image": seconds * 4, "total": seconds * 20}
    value = {
        "source": f"source-{seconds}s.mp4", "duration_seconds": seconds,
        "width": 1920, "height": 1080, "fps": 60, "start": 0,
        "codec": "hevc", "bitrate_bps": 10_000_000,
        "documents": [{"path": ass_path.name, "kind": "danmaku"}],
        "overlays": images, "counts": counts,
        "event_rate_per_second": 20, "rolling_seconds": 8,
        "font_policy": "Arial, with the same system CJK fallback on each platform; embedded renderer Noto color emoji unchanged",
        "text_font_size_in_720p_ass_units": 36,
        "preparation_excluded_from_timing": ["download", "video fixture generation", "danmaku scheduling"],
    }
    name = "scenario.json" if seconds == 300 else "warmup.json"
    (directory / name).write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ffmpeg", default="ffmpeg")
    parser.add_argument("--ffprobe", default="ffprobe")
    parser.add_argument("--assets", type=Path, default=ROOT / "assets")
    args = parser.parse_args()
    directory = args.assets.resolve()
    directory.mkdir(parents=True, exist_ok=True)
    archive = directory / (MOVIE_FILE + ".zip")
    sources = {"movie_archive": download(MOVIE_URL, archive), "emoji": {}}
    pinned = json.loads((ROOT / "fixtures.json").read_text(encoding="utf-8"))
    if sources["movie_archive"]["sha256"] != pinned["movie_archive"]["sha256"]:
        raise ValueError("Official source archive differs from the pinned fixture")
    full_movie = directory / MOVIE_FILE
    if not full_movie.exists():
        with zipfile.ZipFile(archive) as zipped:
            full_movie.write_bytes(zipped.read(MOVIE_FILE))

    def fetch_emoji(item):
        index, codepoint = item
        name = f"emoji-{index:02}.png"
        path = directory / name
        metadata = download(f"{TWEMOJI}/assets/72x72/{codepoint}.png", path)
        if metadata["sha256"] != pinned["emoji"][name]["sha256"]:
            raise ValueError(f"Emoji source differs from the pinned fixture: {name}")
        assert path.read_bytes().startswith(b"\x89PNG\r\n\x1a\n"), name
        return name, metadata

    with ThreadPoolExecutor(max_workers=4) as pool:
        sources["emoji"] = dict(pool.map(fetch_emoji, enumerate(EMOJI)))
    sources["emoji_license"] = download(f"{TWEMOJI}/LICENSE-GRAPHICS", directory / "TWEMOJI-LICENSE-GRAPHICS")
    movie = directory / "source-300s.mp4"
    if not movie.exists():
        run([args.ffmpeg, "-hide_banner", "-loglevel", "warning", "-y", "-ss", "60", "-i", full_movie,
             "-t", "300", "-map", "0:v:0", "-map", "0:a:0", "-vf", "setsar=1,setpts=PTS-STARTPTS,format=yuv420p",
             "-r", "60", "-fps_mode", "cfr", "-frames:v", "18000", "-c:v", "libx264", "-preset", "veryfast", "-crf", "18",
             "-g", "120", "-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv",
             "-af", "asetpts=PTS-STARTPTS", "-c:a", "aac", "-b:a", "192k", "-ac", "2", "-ar", "48000",
             "-movflags", "+faststart", movie])
    warmup = directory / "source-10s.mp4"
    warm_frames = None
    if warmup.exists():
        warm_frames = json.loads(subprocess.check_output([
            args.ffprobe, "-v", "error", "-count_frames", "-select_streams", "v:0",
            "-show_entries", "stream=nb_read_frames", "-of", "json", str(warmup),
        ]))["streams"][0]["nb_read_frames"]
    if warm_frames != "600":
        # Stream-copy -t can retain extra B-frames at the boundary. Encode this
        # untimed warmup separately so both runners receive exactly 600 frames.
        run([args.ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-i", movie, "-t", "10",
             "-map", "0", "-frames:v", "600", "-c:v", "libx264", "-preset", "veryfast",
             "-crf", "18", "-g", "120", "-c:a", "copy", warmup])
    scene(directory, 300)
    scene(directory, 10)
    streams = json.loads(subprocess.check_output([
        args.ffprobe, "-v", "error", "-count_frames", "-show_streams", "-show_format", "-of", "json", str(movie),
    ]))
    video = next(s for s in streams["streams"] if s["codec_type"] == "video")
    assert (video["width"], video["height"], video["avg_frame_rate"], int(video["nb_read_frames"])) == (1920, 1080, "60/1", 18000)
    assert abs(float(video["duration"]) - 300.0) < 1 / 60
    sources["prepared_movie"] = {"sha256": sha256(movie), "bytes": movie.stat().st_size, "clip_in_original_seconds": [60, 360], "streams": streams}
    sources["warmup_movie"] = {"sha256": sha256(warmup), "bytes": warmup.stat().st_size, "frames": 600, "seconds": 10}
    sources["files"] = {p.name: sha256(p) for p in directory.iterdir() if p.suffix in {"png", ".png", ".ass", ".json"} and p.name != "fixture-manifest.json"}
    (directory / "fixture-manifest.json").write_text(json.dumps(sources, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"source_sha256": sha256(movie), "frames": 18000, "seconds": 300, "text": 4800, "images": 1200}))


if __name__ == "__main__":
    main()
