#!/usr/bin/env python3
"""Measure production GPU export stages and observer overhead on the same binary."""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import tempfile
import time

from run import clean_environment, diagnostics, resources, write_json
import verify


def check_profile(report, frames):
    spans = report['spans']
    total = spans['worker']['inclusive_ms']
    exclusive = sum(stage['exclusive_ms'] for stage in spans.values())
    if abs(total - exclusive) > 0.001:
        raise ValueError(f'Worker partition differs: {total} vs {exclusive} ms')
    if spans['process_frame']['calls'] != frames or spans['skia_render']['calls'] != frames:
        raise ValueError('Profiler did not observe every video frame')
    for name in ('decode_send', 'decode_receive', 'cpu_filter', 'nv12_upload',
                 'libplacebo_pull', 'encode_send', 'encode_receive', 'mux_write'):
        if spans[name]['calls'] < 1:
            raise ValueError(f'Missing stage {name}')
    return {'worker_partition_error_ms': abs(total - exclusive), 'profiled_frames': frames}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('current', 'scenario', 'warmup', 'output', 'ffmpeg', 'ffprobe'):
        parser.add_argument('--' + key, type=Path, required=True)
    parser.add_argument('--revision', required=True)
    parser.add_argument('--pairs', type=int, default=3)
    parser.add_argument('--discard-verified-media', action='store_true',
                        help='Remove only media produced by this run, after full validation and hashing')
    args = parser.parse_args()
    if args.pairs < 1:
        parser.error('--pairs must be positive')
    for key in ('current', 'scenario', 'warmup', 'ffmpeg', 'ffprobe'):
        setattr(args, key, getattr(args, key).resolve(strict=True))
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    receipt_path = args.output / 'results.json'
    if receipt_path.exists():
        parser.error('Use a fresh output directory')
    lock = Path(tempfile.gettempdir()) / 'bsr-export-comparison-gpu.lock'
    descriptor = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
    os.write(descriptor, json.dumps({'pid': os.getpid(), 'output': str(args.output)}).encode())
    os.close(descriptor)
    environment, removed = clean_environment()
    receipt = {'status': 'running', 'jobs': [], 'metadata': {
        'revision': args.revision, 'platform': platform.platform(), 'pairs': args.pairs,
        'started_at': datetime.now(timezone.utc).isoformat(), 'removed_environment_names': removed,
        'binary_sha256': verify.sha256(args.current),
        'method_sha256': {name: verify.sha256(Path(__file__).with_name(name))
                          for name in ('stages.py', 'run.py', 'verify.py')},
        'timing': 'External process wall; validation excluded. Same binary, alternating profiling off/on.',
        'span_semantics': 'Worker API wall including blocking; exclusive spans partition worker wall.',
        'observation_semantics': 'Overlapping supplemental intervals; only metal_blit_gpu is a device timestamp.',
        'unavailable_device_timings': ['Skia Vulkan', 'libplacebo', 'NV12 upload', 'hardware encode'],
    }}
    try:
        references = {}
        for label, scenario, duration, expected_counts in [
            ('full', args.scenario, 300, (4800, 1200)), ('warmup', args.warmup, 10, (160, 40))
        ]:
            scene = json.loads(scenario.read_text(encoding='utf-8'))
            texts = sum(sum(line.startswith('Dialogue:') for line in
                            (scenario.parent / doc['path']).read_text(encoding='utf-8').splitlines())
                        for doc in scene['documents'])
            if ((scene['duration_seconds'], scene['width'], scene['height'], scene['fps'])
                    != (duration, 1920, 1080, 60) or (texts, len(scene['overlays'])) != expected_counts):
                raise ValueError('Use the complete 300s/10s 1080p60 20-event/s fixtures')
            references[label] = verify.prepare_reference(scenario, args.current, args.ffmpeg,
                                                        args.ffprobe, args.output / ('reference-' + label), environment)
        receipt['references'] = references
        jobs = [('warmup', 'warmup', False), ('warmup', 'warmup', True)]
        for pair in range(args.pairs):
            jobs += [(f'pair-{pair}', 'full', enabled) for enabled in ((False, True) if pair % 2 == 0 else (True, False))]
        for label, fixture, enabled in jobs:
            name = f'{label}-{"measured" if enabled else "control"}'
            scenario = args.scenario if fixture == 'full' else args.warmup
            output = args.output / (name + '.mp4')
            stdout, stderr, usage = [args.output / (name + suffix) for suffix in ('.stdout', '.stderr', '.time')]
            command = [str(args.current), '--scenario', str(scenario), '--output', str(output), '--require-gpu']
            if enabled:
                command += ['--measure-stages']
            executed = ['/usr/bin/time', '-l', '-o', str(usage)] + command if platform.system() == 'Darwin' else command
            print(f'begin {name}', flush=True)
            with stdout.open('wb') as out, stderr.open('wb') as err:
                started = time.perf_counter()
                process = subprocess.run(executed, stdout=out, stderr=err, env=environment, timeout=1800)
                wall = time.perf_counter() - started
            result = {'name': name, 'fixture': fixture, 'measured': enabled, 'command': command,
                      'process_wall_seconds': wall, 'exit_code': process.returncode,
                      'diagnostics': diagnostics(stdout), 'resources': resources(usage, wall)}
            receipt['jobs'].append(result)
            write_json(receipt_path, receipt)
            if process.returncode:
                raise RuntimeError(f'{name} failed: see {stderr}')
            diag = result['diagnostics']
            if not diag['profile']['frame_processor']:
                raise ValueError('GPU processing was not used')
            if enabled:
                result['profile_validation'] = check_profile(diag['stage_profile'], diag['frames'])
            elif diag['stage_profile'] is not None:
                raise ValueError('Profiling was enabled for the control')
            print(f'end {name} {wall:.3f}s; validating', flush=True)
            result['validation'] = verify.verify_output(output, scenario, references[fixture], args.ffmpeg,
                                                        args.ffprobe, args.output / ('verify-' + name), environment)
            result['output_sha256'] = verify.sha256(output)
            if args.discard_verified_media:
                output.unlink()
                result['verified_media_removed'] = True
            write_json(receipt_path, receipt)
        summary = {}
        for name, enabled in [('control', False), ('measured', True)]:
            runs = [r for r in receipt['jobs'] if r['fixture'] == 'full' and r['measured'] == enabled]
            values = [r['process_wall_seconds'] for r in runs]
            summary[name] = {'wall_seconds': values, 'median_wall_seconds': statistics.median(values)}
        summary['observer_overhead_percent'] = (summary['measured']['median_wall_seconds'] /
                                                summary['control']['median_wall_seconds'] - 1) * 100
        receipt.update(status='complete', summary=summary)
        print(json.dumps(summary), flush=True)
    except BaseException as error:
        receipt.update(status='failed', error=repr(error))
        raise
    finally:
        receipt['finished_at'] = datetime.now(timezone.utc).isoformat()
        write_json(receipt_path, receipt)
        lock.unlink(missing_ok=True)


if __name__ == '__main__':
    main()
