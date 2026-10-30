#!/usr/bin/env python3
"""Run release production exporters serially against the complete shared movie."""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

from pr459_job import build_job
import verify


def write_json(path, value):
    path = Path(path)
    temporary = path.with_suffix(path.suffix + ".pending")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def clean_environment():
    environment, removed = os.environ.copy(), []
    for name in list(environment):
        upper = name.upper()
        if (upper.startswith(("VK_", "MTL_", "METAL_", "RENDERDOC", "PIX_"))
                or re.search(r"PROFILE|PROFILING|APIDUMP|API_DUMP|VALIDATION|NSIGHT", upper)
                or upper in {"RUST_LOG", "SKIA_DEBUG", "FFREPORT", "DYLD_INSERT_LIBRARIES"}):
            removed.append(name)
            del environment[name]
    return environment, sorted(removed)


def diagnostics(path):
    values = []
    for line in Path(path).read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and (value.get("ok") or value.get("implementation")):
            values.append(value)
    return values[-1] if values else None


def resources(path, wall):
    if not Path(path).exists():
        return None
    text = Path(path).read_text(encoding="utf-8", errors="replace")
    user = re.search(r"([\d.]+)\s+user", text)
    system = re.search(r"([\d.]+)\s+sys", text)
    resident = re.search(r"(\d+)\s+maximum resident set size", text)
    result = {"path": str(path), "raw": text}
    if user and system:
        result.update(user_seconds=float(user[1]), system_seconds=float(system[1]))
        result["mean_cpu_cores"] = (float(user[1]) + float(system[1])) / wall
    if resident:
        result["maximum_resident_set_size_bytes"] = int(resident[1])
    return result


def invoke(args, implementation, scenario, label, directory, environment):
    output = directory / f"{label}-{implementation}.mp4"
    stdout = directory / f"{label}-{implementation}.stdout"
    stderr = directory / f"{label}-{implementation}.stderr"
    resource = directory / f"{label}-{implementation}.time"
    if implementation == "current":
        command = [str(args.current), "--scenario", str(scenario), "--output", str(output)]
    else:
        job_path = directory / f"{label}-pr459-job.json"
        write_json(job_path, build_job(scenario, output, args.platform))
        command = [str(args.pr459), "--gpu-export", str(job_path)]
    executed = (["/usr/bin/time", "-l", "-o", str(resource)] + command
                if args.platform == "macos" else command)
    print(f"begin {label} {implementation}", flush=True)
    with stdout.open("wb") as out, stderr.open("wb") as error:
        started = time.perf_counter()
        process = subprocess.run(executed, env=environment, stdout=out, stderr=error,
                                 timeout=1800, check=False)
        wall = time.perf_counter() - started
    result = {
        "label": label, "implementation": implementation, "command": command,
        "executed_command": executed, "output": str(output), "stdout": str(stdout),
        "stderr": str(stderr), "exit_code": process.returncode,
        "process_wall_seconds": wall, "diagnostics": diagnostics(stdout),
        "resources": resources(resource, wall),
    }
    print(f"end {label} {implementation} wall={wall:.3f}s exit={process.returncode}", flush=True)
    return result


def metadata(args, environment, removed):
    result = {
        "started_at": datetime.now(timezone.utc).isoformat(), "platform": args.platform,
        "host": {"system": platform.platform(), "machine": platform.machine(),
                 "logical_cpus": os.cpu_count(), "python": sys.version},
        "primary_timing": "External process wall through successful exit, including startup, decode, render, encode, mux and close",
        "serial_gpu_jobs": True, "pairs": args.pairs, "removed_environment_names": removed,
        "binaries": {name: {"path": str(path), "sha256": verify.sha256(path)}
                     for name, path in [("current", args.current), ("pr459", args.pr459)]},
        "scenarios": {name: {"path": str(path), "sha256": verify.sha256(path)}
                      for name, path in [("full", args.scenario), ("warmup", args.warmup)]},
        "sdk_environment": {name: environment[name] for name in
                            ["BSR_FFMPEG_LIBRARY_DIR", "BSR_FFMPEG_PREFIXES", "FFMPEG_DIR"]
                            if name in environment},
    }
    for name in ("ffmpeg", "ffprobe"):
        executable = getattr(args, name)
        data = subprocess.check_output([str(executable), "-version"], env=environment,
                                       stderr=subprocess.STDOUT).decode(errors="replace")
        result[name] = {"path": str(executable), "version": data}
    if args.platform == "macos":
        hardware = subprocess.check_output(
            ["system_profiler", "SPHardwareDataType", "SPDisplaysDataType"], env=environment,
        ).decode(errors="replace")
        result["host"]["hardware"] = "\n".join(line for line in hardware.splitlines()
                                                   if not any(name in line for name in
                                                              ("Serial Number", "Hardware UUID", "Provisioning UDID")))
    return result


def summary(jobs, pairs):
    timed = [job for job in jobs if job["label"].startswith("pair-")]
    result = {}
    for implementation in ("current", "pr459"):
        values = [job for job in timed if job["implementation"] == implementation]
        walls = [job["process_wall_seconds"] for job in values]
        if values:
            frames = values[0]["validation"]["video"]["frames"]
            result[implementation] = {"runs": len(walls), "process_wall_seconds": walls,
                                      "median_process_wall_seconds": statistics.median(walls),
                                      "median_encoded_fps": frames / statistics.median(walls),
                                      "mean_cpu_cores": [job["resources"].get("mean_cpu_cores")
                                                         if job["resources"] else None for job in values]}
    if all(name in result for name in ("current", "pr459")):
        result["pr459_speedup_over_current"] = (
            result["current"]["median_process_wall_seconds"]
            / result["pr459"]["median_process_wall_seconds"])
    result["complete"] = len(timed) == pairs * 2
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("current", "pr459", "scenario", "warmup", "output", "ffmpeg", "ffprobe"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    parser.add_argument("--platform", choices=("macos", "windows"), required=True)
    parser.add_argument("--pairs", type=int, default=3)
    args = parser.parse_args()
    if args.pairs < 1:
        parser.error("--pairs must be positive")
    for name in ("current", "pr459", "scenario", "warmup", "ffmpeg", "ffprobe"):
        path = getattr(args, name)
        if name in ("current", "pr459", "ffmpeg", "ffprobe") and not path.exists():
            path = Path(shutil.which(str(path)) or path)
        setattr(args, name, path.resolve(strict=True))
    for scenario_path, duration, text_count, image_count in [
        (args.scenario, 300, 4800, 1200), (args.warmup, 10, 160, 40),
    ]:
        scene = json.loads(scenario_path.read_text(encoding="utf-8"))
        if (scene["duration_seconds"], scene["fps"], scene["width"], scene["height"]) != (duration, 60, 1920, 1080):
            parser.error(f"{scenario_path}: use the complete {duration}s/1080p60 scenario")
        texts = sum(sum(line.startswith("Dialogue:") for line in
                        (scenario_path.parent / document["path"]).read_text(encoding="utf-8").splitlines())
                    for document in scene["documents"])
        if texts != text_count or len(scene["overlays"]) != image_count:
            parser.error(f"{scenario_path}: wrong exact text/image event counts")
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    receipt_path = args.output / "results.json"
    if receipt_path.exists():
        parser.error("--output already contains results.json; use a new directory")
    lock = Path(tempfile.gettempdir()) / "bsr-export-comparison-gpu.lock"
    try:
        descriptor = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
    except FileExistsError:
        parser.error(f"Another benchmark owns {lock}; no concurrent GPU measurements allowed")
    os.write(descriptor, json.dumps({"pid": os.getpid(), "output": str(args.output)}).encode())
    os.close(descriptor)
    environment, removed = clean_environment()
    receipt = {"jobs": [], "status": "running"}
    try:
        receipt["metadata"] = metadata(args, environment, removed)
        write_json(receipt_path, receipt)
        references = {}
        for name, scenario in [("full", args.scenario), ("warmup", args.warmup)]:
            references[name] = verify.prepare_reference(
                scenario, args.current, args.ffmpeg, args.ffprobe,
                args.output / f"reference-{name}", environment,
            )
            receipt.setdefault("references", {})[name] = references[name]
            write_json(receipt_path, receipt)
        jobs = [("warmup", "warmup", implementation) for implementation in ("current", "pr459")]
        for pair in range(args.pairs):
            order = ("current", "pr459") if pair % 2 == 0 else ("pr459", "current")
            jobs.extend((f"pair-{pair}", "full", implementation) for implementation in order)
        for label, name, implementation in jobs:
            scenario = args.warmup if name == "warmup" else args.scenario
            result = invoke(args, implementation, scenario, label, args.output, environment)
            receipt["jobs"].append(result)
            write_json(receipt_path, receipt)
            if result["exit_code"]:
                raise RuntimeError(f"{label}/{implementation} failed; see {result['stderr']}")
            result["validation"] = verify.verify_output(
                result["output"], scenario, references[name], args.ffmpeg, args.ffprobe,
                args.output / f"verify-{label}-{implementation}", environment,
            )
            result["process_encoded_fps"] = result["validation"]["video"]["frames"] / result["process_wall_seconds"]
            if label.startswith("pair-"):
                paired = [item for item in receipt["jobs"] if item["label"] == label]
                if len(paired) == 2:
                    receipt.setdefault("pair_samples", {})[label] = verify.compare_samples(
                        paired[0]["validation"], paired[1]["validation"],
                    )
            receipt["summary"] = summary(receipt["jobs"], args.pairs)
            write_json(receipt_path, receipt)
        receipt["status"] = "complete"
    except BaseException as error:
        receipt["status"], receipt["error"] = "failed", repr(error)
        raise
    finally:
        receipt["finished_at"] = datetime.now(timezone.utc).isoformat()
        write_json(receipt_path, receipt)
        lock.unlink(missing_ok=True)
    print(json.dumps(receipt["summary"], indent=2))


if __name__ == "__main__":
    main()
