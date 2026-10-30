#!/usr/bin/env python3
"""Build an official PR459 worker job from the common benchmark scenario."""
import argparse
import json
import math
import pathlib
import sys


def build_job(scenario_file, output, platform, source_override=None):
    scenario_file = pathlib.Path(scenario_file).resolve(strict=True)
    scenario = json.loads(scenario_file.read_text(encoding="utf-8"))
    base = scenario_file.parent

    def asset(value):
        path = pathlib.Path(value)
        if not path.is_absolute():
            path = base / path
        path = path.resolve(strict=True)
        if not path.is_file():
            raise ValueError(f"Asset must be a file: {path}")
        return str(path)

    fps = float(scenario["fps"])
    start = float(scenario.get("start", 0.0))
    if not math.isfinite(fps) or not 1.0 <= fps <= 240.0 or start != 0.0:
        raise ValueError("Benchmark requires a finite valid frame rate and start=0")
    if (scenario["width"], scenario["height"]) != (1920, 1080):
        raise ValueError("The official PR459 worker normalizes to 1920x1080")
    codec = scenario.get("codec", "hevc")
    if codec not in ("hevc", "h264"):
        raise ValueError(f"Unsupported benchmark codec: {codec}")
    bitrate = int(scenario["bitrate_bps"])
    if bitrate <= 0:
        raise ValueError("Bitrate must be positive")
    documents = []
    for document in scenario["documents"]:
        kind = document["kind"]
        if kind not in ("danmaku", "subtitle"):
            raise ValueError(f"Invalid document kind: {kind}")
        documents.append({"path": asset(document["path"]), "kind": kind})
    overlays = []
    for event in scenario["overlays"]:
        event = dict(event)
        event["path"] = asset(event["path"])
        overlays.append(event)
    output = pathlib.Path(output).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    return {
        "source": asset(source_override or scenario["source"]),
        "prepared": False,
        "output": str(output),
        "fps": fps,
        "start": start,
        "encoder": codec + ("_videotoolbox" if platform == "macos" else "_vulkan"),
        "bitrate_bps": bitrate,
        "options": {"prio_speed": "1"} if platform == "macos" else {"quality": "0"},
        "documents": documents,
        "overlays": overlays,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scenario", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--job", type=pathlib.Path, required=True)
    parser.add_argument("--platform", choices=("macos", "windows"),
                        default="macos" if sys.platform == "darwin" else "windows")
    parser.add_argument("--source", help="Optional source override, relative to scenario directory")
    args = parser.parse_args()
    job = build_job(args.scenario, args.output, args.platform, args.source)
    args.job.parent.mkdir(parents=True, exist_ok=True)
    args.job.write_text(json.dumps(job, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(args.job.resolve())


if __name__ == "__main__":
    main()
