#!/usr/bin/env python3
"""Extract the pinned, unmodified PR459 exporter into an isolated workspace."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parent
REVISION = "c73c7aaff9b9dd9abb561c90a48f19b2d6374112"
CRATES = ["src-tauri/crates/gpu-export", "src-tauri/crates/native-overlay"]
CARGO = '''[package]
name = "bsr-pr459-worker"
version = "0.1.0"
edition = "2021"
publish = false

[workspace]

[features]
vulkan = ["gpu-export/vulkan"]

[dependencies]
gpu-export = { path = "../source/src-tauri/crates/gpu-export" }
'''
MAIN = '''fn main() {
    gpu_export::worker_entry();
    eprintln!("Usage: bsr-pr459-worker --gpu-export <job.json>");
    std::process::exit(2);
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "pr459")
    args = parser.parse_args()
    repo = ROOT.parents[1]
    if subprocess.run(["git", "cat-file", "-e", REVISION + "^{commit}"], cwd=repo,
                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode:
        subprocess.run(["git", "fetch", "origin", REVISION], cwd=repo, check=True)
    archive = subprocess.check_output(["git", "archive", REVISION, *CRATES], cwd=repo)
    destination = args.output.resolve()
    source = destination / "source"
    files = {}
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        for member in tar:
            if not member.isfile():
                continue
            target = source / member.name
            if ".." in Path(member.name).parts or Path(member.name).is_absolute():
                raise ValueError("Unsafe archive path")
            data = tar.extractfile(member).read()
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            files[member.name] = hashlib.sha256(data).hexdigest()
    runner = destination / "runner"
    (runner / "src").mkdir(parents=True, exist_ok=True)
    (runner / "src/main.rs").write_text(MAIN)
    (runner / "Cargo.toml").write_text(CARGO)
    (runner / "Cargo.lock").write_bytes((ROOT / "pr459.lock").read_bytes())
    manifest = {"pull_request": "https://github.com/Xinrea/bili-shadowreplay/pull/459",
                "verified_head": REVISION, "source_modified": False, "files": files}
    (destination / "source-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(runner / "Cargo.toml")


if __name__ == "__main__":
    main()
