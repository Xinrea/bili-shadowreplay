#!/usr/bin/env python3
"""Untimed full-stream and sampled-scene checks for export comparisons."""
from collections import Counter
from fractions import Fraction
import hashlib
import json
import math
from pathlib import Path
import subprocess

SAMPLE_WIDTH, SAMPLE_HEIGHT = 640, 360
FULL_SAMPLE_TIMES = [2.0, 30.0, 60.0, 120.0, 180.0, 240.0, 298.0, 299.9]


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def command_json(command, env):
    result = subprocess.run([str(arg) for arg in command], env=env, check=True,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return json.loads(result.stdout)


def video_grid(path, scenario, ffprobe, env):
    data = command_json([
        ffprobe, "-v", "error", "-select_streams", "v:0", "-show_frames",
        "-show_streams", "-show_format", "-show_entries",
        "stream=codec_name,profile,pix_fmt,width,height,avg_frame_rate,time_base,bit_rate,has_b_frames:"
        "frame=pts_time,best_effort_timestamp_time,key_frame,pict_type:format=duration,size,bit_rate",
        "-of", "json", path,
    ], env)
    frames, stream = data["frames"], data["streams"][0]
    fps = float(scenario["fps"])
    duration = float(scenario["duration_seconds"])
    expected = round(duration * fps)
    if len(frames) != expected:
        raise ValueError(f"{path}: decoded {len(frames)} frames, expected {expected}")
    if (stream["width"], stream["height"]) != (scenario["width"], scenario["height"]):
        raise ValueError(f"{path}: wrong output dimensions")
    if abs(float(Fraction(stream["avg_frame_rate"])) - fps) > 1e-8:
        raise ValueError(f"{path}: wrong output frame rate")
    pts = [float(frame.get("best_effort_timestamp_time", frame.get("pts_time", "nan")))
           for frame in frames]
    if not pts or not all(math.isfinite(value) for value in pts):
        raise ValueError(f"{path}: missing decoded timestamps")
    error = max(abs(value - (pts[0] + index / fps)) for index, value in enumerate(pts))
    if abs(pts[0]) > 0.5 / fps or error > 0.000002:
        raise ValueError(f"{path}: incomplete/shifted CFR timeline (start={pts[0]}, error={error})")
    if abs(float(data["format"]["duration"]) - duration) > 0.1:
        raise ValueError(f"{path}: wrong container duration")
    keys = [index for index, frame in enumerate(frames) if int(frame.get("key_frame", 0))]
    gaps = [right - left for left, right in zip(keys, keys[1:])]
    return {
        "frames": len(frames), "first_pts": pts[0], "last_pts": pts[-1],
        "pts_grid_max_error": error,
        "pts_sha256": hashlib.sha256("\n".join(f"{value:.6f}" for value in pts).encode()).hexdigest(),
        "stream": stream, "format": data["format"],
        "picture_types": dict(Counter(frame.get("pict_type", "?") for frame in frames)),
        "keyframes": len(keys), "keyframe_indices": keys,
        "gop_gaps": dict(sorted(Counter(gaps).items())), "maximum_gop_gap": max(gaps, default=0),
    }


def audio_packets(path, ffprobe, env):
    data = command_json([
        ffprobe, "-v", "error", "-select_streams", "a:0", "-show_streams",
        "-show_packets", "-show_data_hash", "sha256", "-show_entries",
        "stream=codec_name,sample_rate,channels,channel_layout:packet=size,data_hash",
        "-of", "json", path,
    ], env)
    packets = data.get("packets", [])
    if not data.get("streams") or not packets:
        raise ValueError(f"{path}: missing source/output audio")
    digest = hashlib.sha256()
    for packet in packets:
        digest.update(bytes.fromhex(packet["data_hash"].split(":", 1)[1]))
    stream = {name: data["streams"][0].get(name) for name in
              ("codec_name", "sample_rate", "channels", "channel_layout")}
    return {"stream": stream, "packets": len(packets),
            "payload_bytes": sum(int(packet["size"]) for packet in packets),
            "packet_hashes_sha256": digest.hexdigest()}


def pcm_hash(path, ffmpeg, env):
    result = subprocess.run([
        str(ffmpeg), "-v", "error", "-nostdin", "-i", str(path), "-map", "0:a:0",
        "-vn", "-c:a", "pcm_s16le", "-f", "hash", "-hash", "sha256", "-",
    ], env=env, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return result.stdout.decode().strip()


def pixels(path, ffmpeg, env, time=None, alpha=False):
    command = [str(ffmpeg), "-v", "error", "-nostdin"]
    if time is not None:
        command += ["-ss", f"{time:.6f}"]
    command += ["-i", str(path), "-map", "0:v:0", "-frames:v", "1", "-vf",
                f"scale={SAMPLE_WIDTH}:{SAMPLE_HEIGHT}:flags=bilinear", "-pix_fmt",
                "rgba" if alpha else "rgb24", "-f", "rawvideo", "-"]
    result = subprocess.run(command, env=env, check=True,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout
    if len(result) != SAMPLE_WIDTH * SAMPLE_HEIGHT * (4 if alpha else 3):
        raise ValueError(f"{path}: sample {time} is missing or has the wrong size")
    return result


def save_ppm(path, rgb):
    Path(path).write_bytes(f"P6\n{SAMPLE_WIDTH} {SAMPLE_HEIGHT}\n255\n".encode() + rgb)


def read_ppm(path):
    return Path(path).read_bytes().split(b"\n", 3)[3]


def sample_times(duration):
    return FULL_SAMPLE_TIMES if duration >= 300 else [2.0, duration / 2, duration - 0.1]


def image_boxes(scenario, time):
    boxes = []
    for event in scenario["overlays"]:
        if not event["start"] <= time < event["end"]:
            continue
        part = (time - event["start"]) / (event["end"] - event["start"])
        x = event["from"]["x"] + (event["to"]["x"] - event["from"]["x"]) * part
        y = event["from"]["y"] + (event["to"]["y"] - event["from"]["y"]) * part
        scale = SAMPLE_WIDTH / 1920
        boxes.append((x * scale, y * scale, (x + event["width"]) * scale,
                      (y + event["height"]) * scale))
    return boxes


def prepare_reference(scenario_path, current, ffmpeg, ffprobe, directory, env):
    scenario_path, directory = Path(scenario_path).resolve(), Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    scenario = json.loads(scenario_path.read_text(encoding="utf-8"))
    source = (scenario_path.parent / scenario["source"]).resolve()
    result = {"source": str(source), "source_sha256": sha256(source),
              "video": video_grid(source, scenario, ffprobe, env),
              "audio": audio_packets(source, ffprobe, env), "samples": []}
    for index, time in enumerate(sample_times(float(scenario["duration_seconds"]))):
        overlay = directory / f"overlay-{index}.png"
        report = command_json([current, "--render-reference", "--scenario", scenario_path,
                               "--time", str(time), "--output", overlay], env)
        missing = report.get("missing_glyphs", report.get("missingGlyphs", 0))
        if missing:
            raise ValueError(f"CPU scene reference has {missing} missing glyphs")
        base = pixels(source, ffmpeg, env, time=time)
        layer = pixels(overlay, ffmpeg, env, alpha=True)
        expected = bytes((base[index // 4 * 3 + index % 4] * (255 - layer[index // 4 * 4 + 3])
                          + value * layer[index // 4 * 4 + 3] + 127) // 255
                         for index, value in enumerate(layer) if index % 4 != 3)
        base_path, expected_path = directory / f"source-{index}.ppm", directory / f"expected-{index}.ppm"
        save_ppm(base_path, base)
        save_ppm(expected_path, expected)
        masks = {"text": [], "image": []}
        image_pixels = set()
        for left, top, right, bottom in image_boxes(scenario, time):
            for y in range(max(0, math.ceil(top)), min(SAMPLE_HEIGHT, math.ceil(bottom))):
                image_pixels.update(y * SAMPLE_WIDTH + x for x in
                                    range(max(0, math.ceil(left)), min(SAMPLE_WIDTH, math.ceil(right))))
        for pixel in range(SAMPLE_WIDTH * SAMPLE_HEIGHT):
            if layer[pixel * 4 + 3] < 64:
                continue
            masks["image" if pixel in image_pixels else "text"].append(pixel)
        mask_path = directory / f"mask-{index}.json"
        mask_path.write_text(json.dumps(masks), encoding="utf-8")
        if not masks["text"] or not masks["image"]:
            raise ValueError(f"Reference at {time}s must contain both text and image danmaku")
        result["samples"].append({"time": time, "source": str(base_path),
                                  "expected": str(expected_path), "mask": str(mask_path),
                                  "overlay": str(overlay), "renderer": report})
    return result


def mae(left, right, mask=None):
    if len(left) != len(right):
        raise ValueError("Sample dimensions differ")
    if mask is None:
        return sum(abs(a - b) for a, b in zip(left, right)) / len(left)
    return sum(abs(left[pixel * 3 + channel] - right[pixel * 3 + channel])
               for pixel in mask for channel in range(3)) / (len(mask) * 3)


def verify_output(path, scenario_path, reference, ffmpeg, ffprobe, directory, env):
    path, directory = Path(path), Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    scenario = json.loads(Path(scenario_path).read_text(encoding="utf-8"))
    video = video_grid(path, scenario, ffprobe, env)
    if video["stream"]["codec_name"] != scenario["codec"]:
        raise ValueError(f"{path}: wrong output codec {video['stream']['codec_name']}")
    audio = audio_packets(path, ffprobe, env)
    packet_match = audio == reference["audio"]
    pcm = None
    if not packet_match:
        expected = pcm_hash(reference["source"], ffmpeg, env)
        pcm = {"source": expected, "output": pcm_hash(path, ffmpeg, env)}
        if pcm["source"] != pcm["output"]:
            raise ValueError(f"{path}: audio packet payload and decoded PCM differ")
    samples = []
    for index, sample in enumerate(reference["samples"]):
        actual = pixels(path, ffmpeg, env, time=sample["time"])
        destination = directory / f"frame-{index}.ppm"
        save_ppm(destination, actual)
        base, expected = read_ppm(sample["source"]), read_ppm(sample["expected"])
        masks = json.loads(Path(sample["mask"]).read_text())
        components = {}
        for kind, mask in masks.items():
            signal, error = mae(base, expected, mask), mae(actual, expected, mask)
            components[kind] = {"pixels": len(mask), "source_to_reference_mae": signal,
                                "output_to_reference_mae": error,
                                "output_from_source_mae": mae(actual, base, mask),
                                "closer_than_missing_overlay": error < signal}
        samples.append({"time": sample["time"], "path": str(destination),
                        "sha256": sha256(destination), "source_mae": mae(actual, base),
                        "reference_mae": mae(actual, expected), "components": components})
    # Require positive scene evidence for each component, without turning
    # compressed cross-backend pixel differences into a tight quality gate.
    failures = [{"time": sample["time"], "component": kind, **sample["components"][kind]}
                for sample in samples for kind in ("text", "image")
                if not sample["components"][kind]["closer_than_missing_overlay"]]
    if failures:
        raise ValueError(f"{path}: missing sampled overlay evidence: {json.dumps(failures)}")
    return {"ok": True, "video": video, "audio": audio, "audio_packets_identical": packet_match,
            "pcm_fallback": pcm, "bytes": path.stat().st_size, "sha256": sha256(path),
            "samples": samples}


def compare_samples(current, previous):
    return [{"time": left["time"], "rgb_mae": mae(read_ppm(left["path"]), read_ppm(right["path"]))}
            for left, right in zip(current["samples"], previous["samples"])]
