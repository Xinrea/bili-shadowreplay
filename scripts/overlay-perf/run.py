#!/usr/bin/env python3
"""Reproducible native-overlay CPU benchmark; Python 3.10+, no pip packages."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import struct
import subprocess
import tarfile
import time
import zlib

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
OUT = HERE / "output"
BASELINE = "c9b516a67f1b4155a5fe930cc00cd62c1cb5927e"
SCENES = ("repeated", "unique", "mixed")


def command(args, **kwargs):
    if kwargs.get("text"):
        kwargs.setdefault("encoding", "utf-8")
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def text(args):
    return command(args, cwd=ROOT, stdout=subprocess.PIPE, text=True).stdout.strip()


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def fixtures():
    """100 deterministic 64x64 straight-RGBA PNGs with transparent edge/detail."""
    assets = OUT / "assets"
    assets.mkdir(parents=True, exist_ok=True)
    for image in range(100):
        pixels = bytearray()
        for y in range(64):
            pixels.append(0)  # PNG scanline filter: None
            for x in range(64):
                alpha = 0 if (x - 32) ** 2 + (y - 32) ** 2 > 30 ** 2 else 96 + ((x + y) % 160)
                pixels.extend(((image * 31 + x * 3) % 256, (image * 67 + y * 4) % 256,
                               (image * 13 + x + y) % 256, alpha))
        png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 64, 64, 8, 6, 0, 0, 0))
               + chunk(b"IDAT", zlib.compress(pixels, 9)) + chunk(b"IEND", b""))
        (assets / f"image-{image:03}.png").write_bytes(png)
    return assets


def prepare_baseline():
    """Extract only the historical core; never checkout/reset/edit live sources."""
    revision = text(["git", "rev-parse", f"{BASELINE}^{{commit}}"])
    dest = OUT / "baseline"
    dest.mkdir(parents=True, exist_ok=True)
    archive = command(["git", "archive", revision, "src-tauri/crates/native-overlay"],
                      cwd=ROOT, stdout=subprocess.PIPE).stdout
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        for member in tar:
            target = (dest / member.name).resolve()
            if not target.is_relative_to(dest.resolve()):
                raise RuntimeError("Archive entry escapes baseline directory")
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            elif member.isfile():
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(tar.extractfile(member).read())
            else:
                raise RuntimeError(f"Unexpected non-regular archive entry: {member.name}")
    manifest = (HERE / "Cargo.toml").read_text()
    manifest = manifest.replace('"../../src-tauri/crates/native-overlay"',
                                json.dumps(str(dest / "src-tauri/crates/native-overlay")))
    manifest = manifest.replace('"../../src-tauri/crates/ffmpeg_utils"',
                                json.dumps(str(ROOT / "src-tauri/crates/ffmpeg_utils")))
    manifest = manifest.replace('path = "src/main.rs"',
                                "path = " + json.dumps(str(HERE / "src/main.rs")))
    (dest / "Cargo.toml").write_text(manifest)
    return revision, dest / "Cargo.toml"


def build(manifest, name, online):
    args = ["cargo", "build", "--release", "--message-format=json", "--manifest-path", manifest,
            "--target-dir", OUT / "target"]
    if not online:
        args.append("--offline")
    lock = manifest.with_name("Cargo.lock")
    if lock.exists() and 'name = "overlay-perf"' in lock.read_text():
        args.append("--locked")
    if name == "baseline":
        args.append("--no-default-features")
    result = command(args, cwd=ROOT, stdout=subprocess.PIPE, text=True)
    messages = [json.loads(line) for line in result.stdout.splitlines()]
    executables = [item["executable"] for item in messages
                   if item.get("reason") == "compiler-artifact" and item.get("executable")
                   and item["target"]["name"] == "overlay-perf"]
    if len(executables) != 1:
        raise RuntimeError("Cargo did not report a unique benchmark executable")
    suffix = ".exe" if os.name == "nt" else ""
    dest = OUT / "bin" / (name + suffix)
    dest.parent.mkdir(exist_ok=True)
    shutil.copy2(executables[0], dest)
    return dest


def bench(binary, mode, assets, scene, *extra):
    result = command([binary, mode, assets, scene, *extra], cwd=ROOT,
                     stdout=subprocess.PIPE, text=True, timeout=90)
    return json.loads(result.stdout)


def probe(ffprobe, path):
    value = json.loads(text([ffprobe, "-v", "error", "-select_streams", "v:0", "-count_frames",
                            "-show_entries", "stream=codec_name,width,height,avg_frame_rate,nb_read_frames,duration,pix_fmt",
                            "-of", "json", path]))["streams"][0]
    assert value["codec_name"] == "h264", value
    assert (value["width"], value["height"]) == (1920, 1080), value
    assert value["avg_frame_rate"] == "60/1", value
    assert int(value["nb_read_frames"]) == 360, value
    assert abs(float(value["duration"]) - 6.0) < 1 / 60, value
    assert value["pix_fmt"] == "yuv420p", value
    return value


def numeric_medians(rows):
    result = {}
    for key, value in rows[0].items():
        if isinstance(value, dict):
            result[key] = numeric_medians([row[key] for row in rows])
        elif isinstance(value, (int, float)) and not isinstance(value, bool):
            result[key] = statistics.median(row[key] for row in rows)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=int, default=3, help="timed repetitions per case (default: 3)")
    parser.add_argument("--prepare-only", action="store_true", help="generate fixtures and baseline source only")
    parser.add_argument("--baseline-only", action="store_true", help="build/run baseline while current APIs are unavailable")
    parser.add_argument("--online", action="store_true", help="allow Cargo dependency resolution/downloads (default offline)")
    parser.add_argument("--ffprobe", default="ffprobe")
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")
    OUT.mkdir(exist_ok=True)
    assets = fixtures()
    revision, baseline_manifest = prepare_baseline()
    if args.prepare_only:
        print(f"Prepared deterministic fixtures and {revision} under {OUT}")
        return
    # Reuse an existing lock so both binaries resolve the same dependency graph.
    current_lock = HERE / "Cargo.lock"
    baseline_lock = baseline_manifest.with_name("Cargo.lock")
    if current_lock.exists():
        shutil.copy2(current_lock, baseline_lock)
    elif not baseline_lock.exists():
        # sidecar 0.0.3 depends on yanked krata-tokio-tar 0.4.2. Preserve the
        # app's already-locked version rather than resolving it from scratch.
        shutil.copy2(ROOT / "src-tauri/Cargo.lock", baseline_lock)
    baseline = build(baseline_manifest, "baseline", args.online)
    shutil.copy2(baseline_lock, current_lock)
    if args.baseline_only:
        results = [bench(baseline, "core", assets, scene) for scene in SCENES for _ in range(args.runs)]
        (OUT / "baseline-only.json").write_text(json.dumps(results, indent=2) + "\n")
        print(json.dumps(results, indent=2))
        return
    current = build(HERE / "Cargo.toml", "current", args.online)
    ffprobe = shutil.which(args.ffprobe)
    if not ffprobe:
        raise RuntimeError("ffprobe must already be installed for independent output validation; no downloads are performed")
    metadata = {
        "baseline_revision": revision, "current_revision": text(["git", "rev-parse", "HEAD"]),
        "git_status": text(["git", "status", "--short"]), "platform": platform.platform(),
        "machine": platform.machine(), "processor": platform.processor(), "logical_cpus": os.cpu_count(),
        "rustc": text(["rustc", "-Vv"]),
        "ffprobe": text([ffprobe, "-version"]), "runs": args.runs,
        "cargo_lock_sha256": hashlib.sha256(current_lock.read_bytes()).hexdigest(),
        "fixture_set_sha256": hashlib.sha256(b"".join(p.read_bytes() for p in sorted(assets.glob("*.png")))).hexdigest(),
        "source_sha256": {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in (
            list((ROOT / "src-tauri/crates/native-overlay/src").rglob("*.rs"))
            + list((ROOT / "src-tauri/crates/ffmpeg_utils/src").rglob("*.rs"))
            + [ROOT / "src-tauri/crates/ffmpeg_utils/build.rs", ROOT / "src-tauri/crates/ffmpeg_utils/Cargo.toml"]
            + [ROOT / "src-tauri/src/ffmpeg/runner.rs", ROOT / "src-tauri/src/ffmpeg/native_overlay.rs",
               HERE / "src/main.rs", HERE / "src/stubs.rs", HERE / "run.py"])},
        "scene": "1920x1080, 60 fps, 6 seconds; 100 events in two 50-event waves; 4 repeated / 100 unique PNG paths; mixed adds 24 text/emoji events",
        "notes": ["CPU Skia only; no GPU", "Core A/B is unprofiled; current stream is profiled",
                  "Stages overlap: do not sum them; frame copy/drain/wall are not pure encoder CPU time",
                  "Source/build/fixtures/process-launch (core only)/pixel-hash validation excluded from render_loop_ms"],
    }
    report = {"metadata": metadata, "cases": {}}
    started = time.monotonic()
    for scene in SCENES:
        before = bench(baseline, "check", assets, scene)
        after = bench(current, "check", assets, scene)
        if before["samples"] != after["samples"]:
            raise RuntimeError(f"Baseline/current RGBA sample mismatch for {scene}")
        # The equality pass warms filesystem/font caches; each subsequent process
        # still constructs a cold Renderer/LRU. Alternate order to limit drift.
        rows = {"baseline_core": [], "current_core": [], "current_stream": []}
        for repetition in range(args.runs):
            order = (("baseline_core", baseline), ("current_core", current))
            for label, binary in order if repetition % 2 == 0 else reversed(order):
                rows[label].append(bench(binary, "core", assets, scene))
            output = OUT / f"{scene}.mp4"
            streamed = bench(current, "stream", assets, scene, output)
            streamed["ffprobe"] = probe(ffprobe, output)
            rows["current_stream"].append(streamed)
        medians = {key: numeric_medians(values) for key, values in rows.items()}
        ratio = medians["baseline_core"]["render_loop_ms"] / medians["current_core"]["render_loop_ms"]
        report["cases"][scene] = {"pixel_equality": {"sample_count": len(before["samples"]),
                                                    "samples": before["samples"]},
                                   "runs": rows, "medians": medians, "core_speedup": ratio}
        print(f"{scene}: core {medians['baseline_core']['render_loop_ms']:.1f} -> "
              f"{medians['current_core']['render_loop_ms']:.1f} ms ({ratio:.2f}x); "
              f"production stream {medians['current_stream']['wall_ms']:.1f} ms")
        # Preserve completed scenes if a later scene fails.
        (OUT / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    report["metadata"]["validation_and_measurement_wall_s"] = time.monotonic() - started
    (OUT / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"Results, native plans, source hashes and sampled RGBA checks: {OUT / 'results.json'}")


if __name__ == "__main__":
    main()
