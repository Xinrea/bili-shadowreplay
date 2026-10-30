#!/usr/bin/env python3
"""Time matched project-default render/encode subpipelines without API capture.

The CPU producer retains the production three-buffer/one-frame-queue overlap.
There is no source-video decoder or audio work in this Vulkan PoC. Different
encoder APIs retain their own speed policies; this is not an equal-quality test.
"""
import argparse
from fractions import Fraction
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import time


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def inspect_video(path, ffprobe, frames):
    result = subprocess.run([
        ffprobe, "-v", "error", "-select_streams", "v:0", "-count_frames",
        "-show_streams", "-show_frames", "-show_entries",
        "stream=codec_name,codec_tag_string,width,height,pix_fmt,has_b_frames,avg_frame_rate,time_base,duration,nb_read_frames,bit_rate:frame=key_frame,best_effort_timestamp",
        "-of", "json", str(path),
    ], capture_output=True, text=True, encoding="utf-8", errors="replace", check=True)
    value = json.loads(result.stdout)
    stream = value["streams"][0]
    require(stream["codec_name"] == "hevc", f"Wrong codec: {stream}")
    require((stream["width"], stream["height"]) == (1920, 1080), f"Wrong size: {stream}")
    require(stream["pix_fmt"] == "yuv420p", f"Wrong pixel format: {stream}")
    require(Fraction(stream["avg_frame_rate"]) == 60, f"Wrong FPS: {stream}")
    decoded = value["frames"]
    require(len(decoded) == frames and int(stream["nb_read_frames"]) == frames,
            f"Wrong frame count: {stream}")
    tick = Fraction(stream["time_base"])
    require(abs(Fraction(stream["duration"]) - Fraction(frames, 60)) <= tick,
            f"Wrong video duration: {stream}")
    for index, frame in enumerate(decoded):
        require(abs(int(frame["best_effort_timestamp"]) * tick - Fraction(index, 60)) <= tick,
                f"Wrong timestamp at frame {index}")
    keyframes = [index for index, frame in enumerate(decoded) if frame["key_frame"]]
    require(keyframes and keyframes[0] == 0, "Video does not begin with a keyframe")
    max_gap = max(b - a for a, b in zip(keyframes, keyframes[1:] + [frames]))
    require(max_gap <= 120, f"Output GOP exceeds project default: {keyframes}")
    return {**stream, "decoded_frames": len(decoded), "keyframe_indices": keyframes,
            "maximum_keyframe_gap": max_gap, "bytes": path.stat().st_size,
            "sha256": sha256(path)}


def frame_rgb(path, index, ffmpeg):
    result = subprocess.run([
        ffmpeg, "-v", "error", "-nostdin", "-i", str(path),
        "-vf", f"select=eq(n\\,{index})", "-fps_mode", "passthrough",
        "-frames:v", "1", "-pix_fmt", "rgb24", "-f", "rawvideo", "-",
    ], capture_output=True, check=True)
    require(len(result.stdout) == 1920 * 1080 * 3, "Invalid decoded sample size")
    return result.stdout


def compare_scene(cpu_path, gpu_path, frames, ffmpeg):
    samples = []
    for index in [0, frames // 2, frames - 2]:
        cpu, gpu = frame_rgb(cpu_path, index, ffmpeg), frame_rgb(gpu_path, index, ffmpeg)
        mean_error = sum(abs(a - b) for a, b in zip(cpu, gpu)) / len(cpu)
        cpu_bright = sum(min(cpu[n:n + 3]) > 180 for n in range(0, len(cpu), 3))
        gpu_bright = sum(min(gpu[n:n + 3]) > 180 for n in range(0, len(gpu), 3))
        require(mean_error < 8, f"Scene outputs differ too much at frame {index}: {mean_error}")
        if index >= 120:
            require(min(cpu_bright, gpu_bright) > 100, f"Missing visible scene text at {index}")
            require(abs(cpu_bright - gpu_bright) / max(cpu_bright, gpu_bright) < 0.3,
                    f"Text content differs at frame {index}: {cpu_bright}, {gpu_bright}")
        samples.append({"frame": index, "cpu_bright_pixels": cpu_bright,
                        "gpu_bright_pixels": gpu_bright, "mean_absolute_rgb_difference": mean_error})
    return {"scope": "post-encode scene sanity check, not an equal-quality objective metric",
            "samples": samples}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--ffprobe", required=True)
    parser.add_argument("--ffmpeg", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--frames", type=int, default=1800)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--scenes", nargs="+", choices=("normal", "dense"), default=["normal", "dense"])
    args = parser.parse_args()
    require(120 <= args.frames <= 3600 and 1 <= args.repeats <= 10, "Invalid benchmark length/repeats")
    require(args.executable.is_file(), "Benchmark executable missing")
    args.output.mkdir(parents=True, exist_ok=True)
    receipt = args.output / "benchmark.json"
    require(not receipt.exists(), "Choose a fresh output directory; a benchmark receipt already exists")
    env = os.environ.copy()
    for key in list(env):
        if key.startswith(("VK_APIDUMP_", "VK_KHRONOS_VALIDATION_")) or key in {
            "VK_INSTANCE_LAYERS", "VK_LAYER_SETTINGS_PATH", "VK_LAYER_PATH", "VK_LAYER_ENABLES",
        }:
            env.pop(key, None)
    env["VK_LOADER_LAYERS_DISABLE"] = "~implicit~"
    report = {"scope": "render/composite/convert/encode only; no input video decoding, audio or project caches",
              "frames": args.frames, "repeats": args.repeats,
              "capture_and_validation_layers": "disabled; run from an ordinary non-elevated user process",
              "executable_sha256": sha256(args.executable), "warmups": [], "runs": [], "summary": {}}

    def save():
        receipt.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    def run(scene, mode, frames, label):
        video = args.output / f"{label}.mp4"
        started = time.perf_counter()
        process = subprocess.run([
            str(args.executable), "--mode", mode, "--scene", scene,
            "--frames", str(frames), "--output", str(video),
        ], capture_output=True, text=True, encoding="utf-8", errors="replace", env=env, timeout=180)
        elapsed = (time.perf_counter() - started) * 1000
        (args.output / f"{label}.stdout.log").write_text(process.stdout, encoding="utf-8")
        (args.output / f"{label}.stderr.log").write_text(process.stderr, encoding="utf-8")
        require(process.returncode == 0, f"{label} failed ({process.returncode}): {process.stderr[-2000:]}")
        measured = json.loads(process.stdout)
        require(measured["mode"] == mode and measured["frames"] == frames, f"Wrong producer receipt: {label}")
        return {"label": label, "scene": scene, "mode": mode, "process_wall_ms": elapsed,
                "measurement": measured, "video": inspect_video(video, args.ffprobe, frames)}

    save()
    for mode in ["cpu", "gpu"]:
        row = run(args.scenes[0], mode, 120, f"warmup-{mode}")
        report["warmups"].append(row)
        save()
        print(f"Warmup {mode}: {row['measurement']['stats']['total_ms']:.2f} ms", flush=True)
    for scene_index, scene in enumerate(args.scenes):
        for trial in range(args.repeats):
            # Alternate pairs so neither path always runs first on a warming GPU.
            order = ["cpu", "gpu"] if (trial + scene_index) % 2 == 0 else ["gpu", "cpu"]
            for mode in order:
                row = run(scene, mode, args.frames, f"{scene}-{trial + 1}-{mode}")
                report["runs"].append(row)
                save()
                print(f"{scene} {trial + 1}/{args.repeats} {mode}: "
                      f"{row['measurement']['stats']['total_ms']:.2f} ms", flush=True)
        rows = [row for row in report["runs"] if row["scene"] == scene]
        times = {mode: [row["measurement"]["stats"]["total_ms"] for row in rows if row["mode"] == mode]
                 for mode in ["cpu", "gpu"]}
        cpu, gpu = statistics.median(times["cpu"]), statistics.median(times["gpu"])
        last = {mode: args.output / f"{scene}-{args.repeats}-{mode}.mp4" for mode in ["cpu", "gpu"]}
        report["summary"][scene] = {
            "cpu_median_total_ms": cpu, "gpu_median_total_ms": gpu,
            "speedup": cpu / gpu, "elapsed_time_reduction_percent": (1 - gpu / cpu) * 100,
            "cpu_fps": args.frames / (cpu / 1000), "gpu_fps": args.frames / (gpu / 1000),
            "cpu_total_ms": times["cpu"], "gpu_total_ms": times["gpu"],
            "scene_check": compare_scene(last["cpu"], last["gpu"], args.frames, args.ffmpeg),
        }
        save()
    report["status"] = "complete"
    save()
    print(json.dumps(report["summary"], indent=2), flush=True)


if __name__ == "__main__":
    main()
