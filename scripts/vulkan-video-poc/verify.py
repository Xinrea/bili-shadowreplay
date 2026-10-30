#!/usr/bin/env python3
"""Verify encoded video and a complete LunarG text API dump independently.

This checks recorded application Vulkan calls, not opaque driver internals.
Mapping buffers is permitted: encoders must read compressed bitstreams back to
the CPU. The trace check specifically rejects image-to-host readback commands;
it does not infer that all mapped memory is image data or claim silicon-level
zero-copy. Keep capture enabled from device creation through device destruction.
"""

import argparse
from collections import Counter
from fractions import Fraction
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys


CALL = re.compile(r"^\s*(?:Thread\b[^\r\n]*?:\s*)?(vk[A-Za-z0-9_]+)\s*\(")
READBACK = re.compile(r"^vk(?:CmdCopyImageToBuffer(?:2)?|CopyImageToMemory)(?:KHR|EXT)?$")
DRAW = re.compile(r"^vkCmdDraw")
SUBMIT = re.compile(r"^vkQueueSubmit(?:2)?(?:KHR)?$")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def positive_int(value):
    number = int(value)
    if number <= 0:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def positive_rate(value):
    try:
        number = Fraction(value)
        if number <= 0:
            raise ValueError()
        return number
    except (ValueError, ZeroDivisionError):
        raise argparse.ArgumentTypeError("must be a positive rate, e.g. 60 or 60000/1001") from None


def verify_trace(path):
    require(path.is_file() and path.stat().st_size > 0, f"Missing/empty Vulkan trace: {path}")
    with path.open("rb") as source:
        prefix = source.read(4)
    encoding = "utf-16" if prefix.startswith((b"\xff\xfe", b"\xfe\xff")) else "utf-8-sig"
    calls = Counter()
    with path.open(encoding=encoding) as source:
        for line in source:
            match = CALL.match(line)
            if match:
                calls[match.group(1)] += 1
    require(calls, "Trace contains no Vulkan call records; capture API dump in text format")
    readbacks = {name: count for name, count in sorted(calls.items()) if READBACK.match(name)}
    require(not readbacks, f"CPU image readback commands recorded: {readbacks}")
    require(calls["vkCreateDevice"] > 0, "Trace lacks device creation; capture may have started late")
    require(calls["vkDestroyDevice"] > 0, "Trace lacks device destruction; capture may be incomplete")
    draws = sum(count for name, count in calls.items() if DRAW.match(name))
    submits = sum(count for name, count in calls.items() if SUBMIT.match(name))
    require(draws > 0, "Trace lacks graphics draw calls; GPU Skia rendering was not demonstrated")
    require(submits > 0, "Trace lacks queue submissions")
    require(calls["vkCmdEncodeVideoKHR"] > 0, "Trace lacks Vulkan Video encode commands")
    interesting = {
        name: count for name, count in sorted(calls.items())
        if DRAW.match(name) or SUBMIT.match(name)
        or name in {"vkCreateDevice", "vkDestroyDevice", "vkCmdEncodeVideoKHR", "vkCmdDispatch",
                    "vkCmdDispatchIndirect", "vkCmdCopyBufferToImage", "vkCmdCopyBufferToImage2",
                    "vkCmdCopyBufferToImage2KHR", "vkCmdCopyImage", "vkCmdCopyImage2",
                    "vkCmdCopyImage2KHR", "vkMapMemory", "vkMapMemory2", "vkMapMemory2KHR"}
    }
    return {
        "path": str(path.resolve()),
        "sha256": sha256(path),
        "bytes": path.stat().st_size,
        "recorded_calls": sum(calls.values()),
        "draw_records": draws,
        "queue_submit_records": submits,
        "image_readback_records": sum(readbacks.values()),
        "command_counts": interesting,
        "scope": "Recorded application Vulkan calls; buffer mapping and opaque driver internals are not classified as image readback",
    }


def verify_video(args):
    path = args.video
    require(path.is_file() and path.stat().st_size > 0, f"Missing/empty encoded video: {path}")
    result = subprocess.run([
        args.ffprobe, "-v", "error", "-select_streams", "v:0", "-count_frames",
        "-show_streams", "-show_frames", "-show_entries",
        "stream=codec_name,width,height,pix_fmt,time_base,avg_frame_rate,start_time,duration,nb_frames,nb_read_frames,color_range,color_space,color_transfer,color_primaries:frame=pts,best_effort_timestamp,duration,pkt_duration",
        "-of", "json", str(path),
    ], capture_output=True, text=True, encoding="utf-8", check=False)
    require(result.returncode == 0, f"ffprobe failed ({result.returncode}): {result.stderr.strip()}")
    value = json.loads(result.stdout)
    streams = value.get("streams", [])
    require(len(streams) == 1, f"Expected one selected video stream, got {len(streams)}")
    stream = streams[0]
    require(stream.get("codec_name") == args.codec, f"Unexpected video codec: {stream}")
    require((stream.get("width"), stream.get("height")) == (args.width, args.height),
            f"Unexpected video dimensions: {stream}")
    require(stream.get("pix_fmt") == args.pix_fmt, f"Unexpected decoded pixel format: {stream}")
    for key, expected in {"color_range": "tv", "color_space": "bt709",
                          "color_transfer": "bt709", "color_primaries": "bt709"}.items():
        require(stream.get(key) == expected, f"Unexpected {key}: {stream}")
    require(Fraction(stream["avg_frame_rate"]) == args.fps, f"Unexpected average frame rate: {stream}")
    frames = value.get("frames", [])
    require(len(frames) == args.frames, f"Decoded {len(frames)} frames; expected {args.frames}")
    require(int(stream["nb_read_frames"]) == args.frames, f"Unexpected decoded frame count: {stream}")
    if "nb_frames" in stream:
        require(int(stream["nb_frames"]) == args.frames, f"Unexpected declared frame count: {stream}")
    time_base = Fraction(stream["time_base"])
    require(time_base > 0, f"Invalid video time base: {stream}")
    tolerance = max(time_base, Fraction(1, 1_000_000))
    for index, frame in enumerate(frames):
        timestamp = frame.get("best_effort_timestamp", frame.get("pts"))
        require(timestamp is not None, f"Decoded frame {index} has no timestamp")
        actual = int(timestamp) * time_base
        expected = Fraction(index, 1) / args.fps
        require(abs(actual - expected) <= time_base,
                f"Frame {index} timestamp {actual} differs from expected {expected} by more than one time-base tick")
    duration = Fraction(stream["duration"])
    expected_duration = Fraction(args.frames, 1) / args.fps
    require(abs(duration - expected_duration) <= tolerance,
            f"Video duration {duration} differs from expected {expected_duration}")
    require(abs(Fraction(stream["start_time"])) <= tolerance, f"Unexpected video start time: {stream}")
    return {
        "path": str(path.resolve()),
        "sha256": sha256(path),
        "bytes": path.stat().st_size,
        "codec": stream["codec_name"],
        "width": stream["width"],
        "height": stream["height"],
        "decoded_pixel_format": stream["pix_fmt"],
        "decoded_frames": len(frames),
        "colour": {key: stream[key] for key in
                   ("color_range", "color_space", "color_transfer", "color_primaries")},
        "frame_rate": str(args.fps),
        "time_base": str(time_base),
        "duration": str(duration),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--video", type=Path, required=True)
    parser.add_argument("--trace", type=Path, required=True, help="complete LunarG API dump text file")
    parser.add_argument("--width", type=positive_int, required=True)
    parser.add_argument("--height", type=positive_int, required=True)
    parser.add_argument("--frames", type=positive_int, required=True)
    parser.add_argument("--fps", type=positive_rate, required=True)
    parser.add_argument("--codec", choices=("h264", "hevc", "av1"), default="h264")
    parser.add_argument("--pix-fmt", default="yuv420p", help="decoded video format reported by ffprobe")
    parser.add_argument("--ffprobe", default="ffprobe")
    parser.add_argument("--json", type=Path, help="optional verification receipt")
    args = parser.parse_args()
    try:
        report = {"trace": verify_trace(args.trace), "video": verify_video(args)}
        output = json.dumps(report, indent=2)
        if args.json:
            args.json.write_text(output + "\n", encoding="utf-8")
        print(output)
    except (OSError, ValueError, KeyError, TypeError, ZeroDivisionError) as error:
        print(f"Verification failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
