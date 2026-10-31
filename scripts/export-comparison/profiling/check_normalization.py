#!/usr/bin/env python3
"""Independent FFmpeg CLI oracle for the production GPU input normalization.

Run outside timing jobs. Common YUV cases must remain byte-identical; RGB is
allowed to differ because one-pass chroma conversion changes edge resampling.
This is a validation tool, not part of the application export path.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ffmpeg', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[3]
    code = (root / 'src-tauri/src/ffmpeg/hwaccel.rs').read_text()

    def constant(name):
        return re.search(r'pub const ' + name + r': &str =\s*"([^"]+)";', code)[1]

    old = constant('H264_SCALE_PAD_FILTER') + ',fps=60,scale=out_color_matrix=bt709:out_range=tv,format=nv12'
    new = constant('GPU_SCALE_PAD_FILTER') + ',fps=60'
    cases = [('real-1080p', ['-ss', '10', '-i', str(root / 'scripts/export-comparison/assets/source-300s.mp4')])]
    for name, size in [('landscape', '1280x720'), ('portrait', '720x1280')]:
        cases.append((name, ['-f', 'lavfi', '-i', f'testsrc2=size={size}:rate=60,setparams=colorspace=bt709:range=tv']))
    cases.append(('rgb', ['-f', 'lavfi', '-i', 'testsrc=size=640x360:rate=60,setparams=colorspace=bt709:range=pc']))
    results = []
    for name, source in cases:
        arrays = []
        for vf in (old, new):
            run = subprocess.run([str(args.ffmpeg), '-v', 'error', *source, '-an', '-frames:v', '1',
                                  '-vf', vf, '-pix_fmt', 'nv12', '-f', 'rawvideo', '-'],
                                 capture_output=True, check=True)
            assert len(run.stdout) == 1920 * 1080 * 3 // 2
            arrays.append(run.stdout)
        delta = [abs(a - b) for a, b in zip(*arrays)]
        result = {'case': name, 'source': source,
                  'old_sha256': hashlib.sha256(arrays[0]).hexdigest(),
                  'new_sha256': hashlib.sha256(arrays[1]).hexdigest(),
                  'mean_abs_difference_8bit': sum(delta) / len(delta),
                  'maximum_difference_8bit': max(delta),
                  'different_samples': sum(d != 0 for d in delta)}
        if name != 'rgb':
            assert arrays[0] == arrays[1], result
        if name == 'portrait':
            for data in arrays:
                assert all(data[y * 1920] == data[y * 1920 + 1919] == 16 for y in range(1080))
                assert all(data[1920 * 1080 + y * 1920 + x] == 128
                           for y in range(540) for x in (0, 1, 1918, 1919))
            result['black_borders_checked'] = True
        results.append(result)
    args.output.write_text(json.dumps({'old_filter': old, 'new_filter': new, 'results': results}, indent=2) + '\n')


if __name__ == '__main__':
    main()
