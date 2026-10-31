#!/usr/bin/env python3
"""Probe a supplied KosmicKrisp SDK; no installation or system configuration.

FFmpeg CLI is an independent diagnostic oracle only. The native producer and
render probe exercise the project's linked libraries. A successful script run
means the evidence was collected, not that Vulkan Video encoding is supported.
"""

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time

from verify import CALL, READBACK


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", type=Path, required=True, help="SDK macOS directory")
    parser.add_argument("--ffmpeg", type=Path, required=True)
    parser.add_argument("--producer", type=Path, required=True)
    parser.add_argument("--render-probe", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    sdk, output = args.sdk.resolve(), args.output.resolve()
    if output.exists() and any(output.iterdir()):
        raise FileExistsError("Use an empty output directory to avoid stale evidence")
    output.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    # Avoid inherited driver/layer overrides changing the subject of the probe.
    for key in list(env):
        if key.startswith(("VK_", "MESA_KK_", "MESA_VK_", "DYLD_")):
            del env[key]
    env.update(
        DYLD_LIBRARY_PATH=str(sdk / "lib"),
        VK_DRIVER_FILES=str(sdk / "share/vulkan/icd.d/libkosmickrisp_icd.json"),
        VK_LOADER_LAYERS_DISABLE="~implicit~",
    )
    runs = {}

    def run(name, command, extra_env=None):
        started = time.monotonic()
        with (output / (name + ".txt")).open("w") as log:
            result = subprocess.run(
                [str(value) for value in command], cwd=output,
                env=env | (extra_env or {}), stdout=log,
                stderr=subprocess.STDOUT, timeout=120, check=False,
            )
        runs[name] = {
            "command": [str(value) for value in command],
            "exit_code": result.returncode,
            "seconds": round(time.monotonic() - started, 3),
            "extra_environment": extra_env or {},
        }
        print(name, result.returncode, flush=True)
        return (output / (name + ".txt")).read_text()

    vulkaninfo = sdk / "bin/vulkaninfo"
    summary = run("vulkan-summary", [vulkaninfo, "--summary"])
    if "DRIVER_ID_MESA_KOSMICKRISP" not in summary:
        raise RuntimeError("KosmicKrisp was not enumerated; see vulkan-summary.txt")
    run("vulkan-full", [vulkaninfo, "--show-video-props"])
    run("vulkan-json", [vulkaninfo, "--json", "-o", output / "capabilities.json"])
    if runs["vulkan-json"]["exit_code"]:
        raise RuntimeError("Failed to obtain device capabilities")
    device = json.loads((output / "capabilities.json").read_text())["capabilities"]["device"]

    ffmpeg = args.ffmpeg.resolve()
    version = run("ffmpeg-version", [ffmpeg, "-version"]).splitlines()[0]
    base = [ffmpeg, "-hide_banner", "-loglevel", "verbose", "-init_hw_device", "vulkan=vk:0"]
    source = ["-f", "lavfi", "-i", "color=size=640x360:rate=30"]
    run("ffmpeg-device", base + source + ["-frames:v", "1", "-f", "null", "-"])
    for encoder in ("h264_vulkan", "hevc_vulkan"):
        run("ffmpeg-" + encoder, base + ["-filter_hw_device", "vk"] + source + [
            "-vf", "format=nv12,hwupload", "-frames:v", "3", "-c:v", encoder,
            "-f", "null", "-",
        ])
        run("native-" + encoder, [args.producer.resolve(), "--width", "640",
            "--height", "360", "--frames", "3", "--encoder", encoder,
            "--output", output / (encoder + ".mp4")])

    capture_env = {
        "VK_LAYER_PATH": str(sdk / "share/vulkan/explicit_layer.d"),
        "VK_INSTANCE_LAYERS": "VK_LAYER_LUNARG_api_dump:VK_LAYER_KHRONOS_validation",
        "VK_LOADER_DEBUG": "layer",
        "VK_APIDUMP_LOG_FILENAME": str(output / "render-api.txt"),
        "VK_APIDUMP_LOG_TO_FILE": "true",
        "VK_APIDUMP_OUTPUT_FORMAT": "text",
        "VK_APIDUMP_DETAILED": "false",
        "VK_APIDUMP_FLUSH": "true",
        "VK_APIDUMP_TIMESTAMP": "false",
    }
    render_log = run("render-probe", [args.render_probe.resolve()], capture_env)
    calls = Counter()
    trace = output / "render-api.txt"
    if trace.exists():
        for line in trace.read_text().splitlines():
            match = CALL.match(line)
            if match:
                calls[match[1]] += 1
    render_result = next((json.loads(line) for line in render_log.splitlines()
                          if line.startswith('{"compositor":')), None)
    receipt = {
        "environment": {key: env[key] for key in (
            "DYLD_LIBRARY_PATH", "VK_DRIVER_FILES", "VK_LOADER_LAYERS_DISABLE")},
        "ffmpeg_version": version,
        "device_summary": summary,
        "device_extensions": device["extensions"],
        "video_extensions": {k: v for k, v in device["extensions"].items() if "video" in k.lower()},
        "queue_families": device["queueFamiliesProperties"],
        "runs": runs,
        "render_result": render_result,
        "render_trace_complete": (
            runs["render-probe"]["exit_code"] == 0
            and calls["vkCreateInstance"] == calls["vkDestroyInstance"] == 1
            and calls["vkCreateDevice"] == calls["vkDestroyDevice"] == 1
            and calls["vkCmdDraw"] + calls["vkCmdDrawIndexed"] > 0
        ),
        "loaded_layers": [name for name in (
            "VK_LAYER_LUNARG_api_dump", "VK_LAYER_KHRONOS_validation")
            if f'Insert instance layer "{name}"' in render_log],
        "render_api_calls": dict(sorted(calls.items())),
        "image_to_host_calls": {k: v for k, v in calls.items() if READBACK.match(k)},
        "validation_vuids": sorted(set(re.findall(r"VUID-[\w-]+", render_log))),
        "scope": "Runtime capability/encoder initialization and GPU render smoke test. No decoded visual or performance verification; inspect individual encoder exit codes. API trace does not inspect opaque driver internals.",
    }
    receipt["artifacts"] = {
        path.name: hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(output.iterdir())
        if path.is_file() and path.name != "receipt.json"
    }
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
