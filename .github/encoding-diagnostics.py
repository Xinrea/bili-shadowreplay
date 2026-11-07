import hashlib
import json
import os
from pathlib import Path
import subprocess

root = Path(os.environ["BSR_ENCODING_DIAGNOSTICS"])
args = json.loads((root / "case.txt").read_text().splitlines()[1])
args[args.index("-i") + 1] = str(root / "source.mp4")

def video_hash(path):
    raw = subprocess.check_output(["ffmpeg", "-v", "error", "-i", str(path), "-map", "0:v", "-c", "copy", "-f", "h264", "-"])
    return hashlib.sha256(raw).hexdigest()

native_hash = video_hash(root / "native.mp4")
oracle_hash = video_hash(root / "oracle.mp4")
results = [f"native={native_hash} oracle={oracle_hash}"]
for iteration in range(30):
    target = root / "cli-repeat.mp4"
    with (root / "cli-debug.log").open("wb") as log:
        subprocess.run(["ffmpeg", "-v", "verbose", "-debug_ts", "-nostdin", "-y", "-filter_threads", "1", "-filter_complex_threads", "1", *args, str(target)], stderr=log, check=True)
    results.append(f"CLI repeat {iteration}: {video_hash(target)}")
print("\n".join(results))
(root / "cli-results.txt").write_text("\n".join(results))
