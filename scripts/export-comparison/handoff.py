#!/usr/bin/env python3
"""Compare blocking/current GPU frame handoff, or a pinned previous binary on Windows."""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess
import tempfile
import time

from run import clean_environment, diagnostics, resources, write_json
import verify
from stages import check_profile


def check_handoff(report, variant, frames):
    result = check_profile(report, frames)
    config = report['metadata']['decoder_0_0']
    match = re.fullmatch(r'thread_count=(\d+), active_thread_type=(\d+)', config)
    if not match or int(match[1]) <= 1 or int(match[2]) == 0:
        raise ValueError('Both variants must use automatic frame-threaded decoding')
    result.update(decoder_threads=int(match[1]), active_thread_type=int(match[2]))
    if platform.system() == 'Darwin':
        mode = 'Blocking' if variant == 'baseline' else 'Asynchronous'
        if report['metadata']['encoder_handoff'] != mode:
            raise ValueError('Wrong encoder handoff mode')
        counters = report['metadata']['vt_bridge_counters']
        match = re.fullmatch(r'submitted=(\d+), completed=(\d+), pending_peak=(\d+), pending_at_drop=(\d+)', counters)
        if (not match or int(match[1]) != frames or int(match[2]) != frames
                or not 1 <= int(match[3]) <= (1 if variant == 'baseline' else 3) or int(match[4])):
            raise ValueError('Incomplete or unbounded VideoToolbox handoff: ' + counters)
        if report['observations']['metal_blit_gpu']['calls'] != frames:
            raise ValueError('Missing Metal GPU completion timestamps')
        result.update(handoff=mode, pending_peak=int(match[3]))
    elif variant == 'current' and report['metadata']['encoder_handoff'] != 'VulkanTimeline':
        raise ValueError('Windows must retain direct Vulkan timeline handoff')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('current', 'scenario', 'warmup', 'output', 'ffmpeg', 'ffprobe'):
        parser.add_argument('--' + key, type=Path, required=True)
    parser.add_argument('--revision', required=True)
    parser.add_argument('--baseline', type=Path, help='Pinned previous binary for Windows regression comparison')
    parser.add_argument('--baseline-revision')
    parser.add_argument('--pairs', type=int, default=3)
    parser.add_argument('--discard-verified-media', action='store_true',
                        help='Remove only media produced by this run, after full validation and hashing')
    args = parser.parse_args()
    if args.pairs < 1:
        parser.error('--pairs must be positive')
    for key in ('current', 'scenario', 'warmup', 'ffmpeg', 'ffprobe'):
        setattr(args, key, getattr(args, key).resolve(strict=True))
    if args.baseline:
        args.baseline = args.baseline.resolve(strict=True)
        if platform.system() != 'Windows' or not args.baseline_revision:
            parser.error('--baseline requires Windows and --baseline-revision')
    elif platform.system() != 'Darwin':
        parser.error('Windows comparisons require a pinned --baseline binary')
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
        'baseline_binary_sha256': verify.sha256(args.baseline) if args.baseline else verify.sha256(args.current),
        'baseline_revision': args.baseline_revision if args.baseline else args.revision,
        'method_sha256': {name: verify.sha256(Path(__file__).with_name(name))
                          for name in ('handoff.py', 'stages.py', 'run.py', 'verify.py')},
        'timing': 'Alternating baseline/current pairs with profiling OFF; separate diagnostic pair with profiling ON.',
        'baseline': 'Pinned previous binary' if args.baseline else 'Same binary with --blocking-handoff; current uses the production asynchronous default',
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
        jobs = [('warmup', 'warmup', variant, True) for variant in ('baseline', 'current')]
        for pair in range(args.pairs):
            order = ('baseline', 'current') if pair % 2 == 0 else ('current', 'baseline')
            jobs += [(f'pair-{pair}', 'full', variant, False) for variant in order]
        jobs += [('diagnostic', 'full', variant, True) for variant in ('baseline', 'current')]
        for label, fixture, variant, enabled in jobs:
            name = f'{label}-{variant}'
            scenario = args.scenario if fixture == 'full' else args.warmup
            output = args.output / (name + '.mp4')
            stdout, stderr, usage = [args.output / (name + suffix) for suffix in ('.stdout', '.stderr', '.time')]
            binary = args.baseline if variant == 'baseline' and args.baseline else args.current
            command = [str(binary), '--scenario', str(scenario), '--output', str(output), '--require-gpu']
            if variant == 'baseline' and not args.baseline:
                command += ['--blocking-handoff']
            if enabled:
                command += ['--measure-stages']
            executed = ['/usr/bin/time', '-l', '-o', str(usage)] + command if platform.system() == 'Darwin' else command
            print(f'begin {name}', flush=True)
            with stdout.open('wb') as out, stderr.open('wb') as err:
                started = time.perf_counter()
                process = subprocess.run(executed, stdout=out, stderr=err, env=environment, timeout=1800)
                wall = time.perf_counter() - started
            result = {'name': name, 'fixture': fixture, 'variant': variant, 'measured': enabled, 'command': command,
                      'process_wall_seconds': wall, 'exit_code': process.returncode,
                      'diagnostics': diagnostics(stdout), 'resources': resources(usage, wall)}
            receipt['jobs'].append(result)
            write_json(receipt_path, receipt)
            if process.returncode:
                raise RuntimeError(f'{name} failed: see {stderr}')
            diag = result['diagnostics']
            if diag['decoder_threads_override'] is not None:
                raise ValueError('Decoder override would confound the handoff comparison')
            if not (variant == 'baseline' and args.baseline):
                if diag['blocking_handoff'] != (variant == 'baseline'):
                    raise ValueError('Incorrect handoff override')
            if not diag['profile']['frame_processor']:
                raise ValueError('GPU processing was not used')
            if enabled:
                result['profile_validation'] = check_handoff(diag['stage_profile'], variant, diag['frames'])
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
        for name in ('baseline', 'current'):
            runs = [r for r in receipt['jobs'] if r['fixture'] == 'full' and not r['measured'] and r['variant'] == name]
            values = [r['process_wall_seconds'] for r in runs]
            summary[name] = {'wall_seconds': values, 'median_wall_seconds': statistics.median(values)}
        summary['speedup'] = summary['baseline']['median_wall_seconds'] / summary['current']['median_wall_seconds']
        summary['time_saved_percent'] = (1 - 1 / summary['speedup']) * 100
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
