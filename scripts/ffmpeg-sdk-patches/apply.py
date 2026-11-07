#!/usr/bin/env python3
"""Apply one pinned upstream Vulkan cleanup fix to an extracted FFmpeg 9.0.2 tree."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, help="extracted FFmpeg source root")
    parser.add_argument("--check", action="store_true", help="verify without modifying source")
    args = parser.parse_args()
    folder = Path(__file__).resolve().parent
    try:
        manifest = json.loads((folder / "manifest.json").read_text(encoding="utf-8"))
        patch = folder / manifest["patch_file"]
        if digest(patch) != manifest["patch_sha256"]:
            raise ValueError("Upstream patch SHA-256 mismatch")
        target = args.source.resolve() / manifest["source_file"]
        current = digest(target)
        if current == manifest["patched_sha256"]:
            status = "already_applied"
        elif current == manifest["source_sha256"]:
            command = ["git", "apply", "--check", "--whitespace=error-all", str(patch)]
            subprocess.run(command, cwd=args.source, check=True, capture_output=True, text=True)
            status = "can_apply"
            if not args.check:
                subprocess.run(
                    ["git", "apply", str(patch)], cwd=args.source,
                    check=True, capture_output=True, text=True,
                )
                if digest(target) != manifest["patched_sha256"]:
                    raise ValueError("Patched source SHA-256 mismatch")
                status = "applied"
        else:
            raise ValueError(
                "Expected unmodified FFmpeg n9.0.2 Vulkan encoder source; "
                "refusing to patch unknown or locally modified source"
            )
        print(json.dumps({
            "status": status,
            "upstream_commit": manifest["upstream_commit"],
            "source_file": manifest["source_file"],
            "patched_sha256": manifest["patched_sha256"],
        }, indent=2))
        return 0
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        detail = getattr(error, "stderr", None)
        print(f"FFmpeg source patch failed: {detail or error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
