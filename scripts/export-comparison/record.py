#!/usr/bin/env python3
"""Make a portable measurement receipt without machine-local paths."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import statistics


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def receipt(path, platform):
    raw = json.loads(path.read_text(encoding="utf-8"))
    if raw["status"] != "complete" or not raw["summary"]["complete"]:
        raise ValueError("Only a complete, validated comparison may be published")
    rows = [job for job in raw["jobs"] if job["label"].startswith("pair-")]
    assert len(rows) == 6
    runs = []
    for job in rows:
        validation = job["validation"]
        assert job["exit_code"] == 0 and validation["ok"]
        assert validation["video"]["frames"] == 18000
        assert len(validation["samples"]) == 8
        stream = validation["video"]["stream"]
        resources = job.get("resources") or {}
        usage = {key: resources[key] for key in (
            "user_seconds", "system_seconds", "mean_cpu_cores", "maximum_resident_set_size_bytes"
        ) if key in resources}
        physical = re.search(r"(\d+)\s+peak memory footprint", resources.get("raw", ""))
        if physical:
            usage["peak_physical_memory_footprint_bytes"] = int(physical[1])
        diagnostic = job.get("diagnostics") or {}
        internal = {key: diagnostic[key] for key in (
            "export_ms", "wall_ms", "encoder_probe_ms", "total_seconds", "profile", "linked_ffmpeg_version"
        ) if key in diagnostic}
        runs.append({
            "pair": job["label"], "implementation": job["implementation"],
            "process_wall_seconds": job["process_wall_seconds"],
            "encoded_fps": 18000 / job["process_wall_seconds"], "resources": usage,
            "internal_diagnostics": internal, "output_bytes": validation["bytes"],
            "output_sha256": validation["sha256"],
            "video": {
                "stream": {key: stream[key] for key in (
                    "codec_name", "profile", "pix_fmt", "width", "height", "avg_frame_rate",
                    "time_base", "bit_rate", "has_b_frames"
                ) if key in stream},
                **{key: validation["video"][key] for key in (
                    "frames", "first_pts", "last_pts", "pts_grid_max_error", "pts_sha256",
                    "picture_types", "keyframes", "gop_gaps", "maximum_gop_gap"
                )},
            },
            "audio": validation["audio"], "audio_packets_identical": validation["audio_packets_identical"],
            "pcm_fallback": validation["pcm_fallback"],
            "samples": [{key: sample[key] for key in (
                "time", "sha256", "source_mae", "reference_mae", "components"
            )} for sample in validation["samples"]],
        })
    summaries = {}
    for name in ("current", "pr459"):
        jobs = [row for row in runs if row["implementation"] == name]
        assert len(jobs) == 3
        seconds = [job["process_wall_seconds"] for job in jobs]
        summaries[name] = {"runs": seconds, "median_seconds": statistics.median(seconds),
                           "minimum_seconds": min(seconds), "maximum_seconds": max(seconds),
                           "median_fps": 18000 / statistics.median(seconds)}
        for key in ("mean_cpu_cores", "maximum_resident_set_size_bytes", "peak_physical_memory_footprint_bytes"):
            values = [job["resources"][key] for job in jobs if key in job["resources"]]
            if values:
                summaries[name]["median_" + key] = statistics.median(values)
        cpu = [job["resources"]["user_seconds"] + job["resources"]["system_seconds"]
               for job in jobs if "user_seconds" in job["resources"]]
        if cpu:
            summaries[name]["median_cpu_seconds"] = statistics.median(cpu)
    current, previous = summaries["current"]["median_seconds"], summaries["pr459"]["median_seconds"]
    summaries["pr459_throughput_ratio"] = current / previous
    summaries["pr459_duration_reduction_percent"] = (1 - previous / current) * 100
    meta = raw["metadata"]
    host = dict(meta["host"])
    if "hardware" in host:
        # Keep benchmark-relevant fields, not display names, security status or
        # machine identifiers from system_profiler's broader inventory.
        host["hardware"] = "\n".join(line.strip() for line in host["hardware"].splitlines()
                                      if re.match(r"\s*(Chip:|Chipset Model:|Total Number of Cores:|Memory:|Metal Support:)", line))
    windows_environment = path.parent / "windows-environment-and-original-check.txt"
    if platform == "windows" and windows_environment.exists():
        for line in windows_environment.read_text(encoding="utf-8-sig").splitlines():
            if line.startswith('{"cpu"'):
                values = json.loads(line)
                host["cpu"] = values["cpu"].strip()
                host["physical_cores"] = values["cores"]
                host["ram_bytes"] = values["ram_bytes"]
                break
    return {
        "recorded_on": "2026-10-31", "platform": platform,
        "current_revision": "730ebd23d57ae01d7ad30e49d944288c07dfbef4",
        "pr459_revision": "c73c7aaff9b9dd9abb561c90a48f19b2d6374112",
        "method_commit": "358d402e", "raw_receipt_sha256": sha256(path),
        "scope": "Release, complete single-video danmaku export; process start through successful close/exit. Prepared input and scene, UI, download and verification are outside timing.",
        "host": host,
        "fixture": {"source_sha256": raw["references"]["full"]["source_sha256"],
                    "scenario_sha256": meta["scenarios"]["full"]["sha256"],
                    "seconds": 300, "frames": 18000, "width": 1920, "height": 1080,
                    "fps": 60, "codec": "hevc", "target_bitrate": 10000000,
                    "text_events": 4800, "image_events": 1200, "arrivals_per_second": 20},
        "warmups": [{"implementation": job["implementation"], "frames": job["validation"]["video"]["frames"],
                     "validation_ok": job["validation"]["ok"]}
                    for job in raw["jobs"] if job["label"] == "warmup"],
        "producer_sha256": {name: value["sha256"] for name, value in meta["binaries"].items()},
        "ffmpeg_version": meta["ffmpeg"]["version"].splitlines()[0],
        "validation": "All 18000 decoded frames and PTS, copied audio, and text/image reference evidence at all eight sampled times including 299.9s passed on every output.",
        "summary": summaries, "runs_in_execution_order": runs,
        "pair_sample_differences": raw["pair_samples"],
        "limitations": [
            "Default encoder GOP/B-frame/async-depth policies differ; these results are not an equal-encoding-quality comparison.",
            "macOS current uses CPU Skia; PR459 uses Metal. Windows uses Vulkan on both sides.",
            "Hosts have different hardware; compare versions within each host.",
            "No API capture or Vulkan validation during timing; prior runtime diagnostics are separate.",
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--platform", choices=("macos", "windows"), required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = receipt(args.source, args.platform)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result["summary"], indent=2))


if __name__ == "__main__":
    main()
