#!/usr/bin/env python3
"""macOS stage A/B: pinned instrumented exporters, serial runs, external telemetry.

PR459's worker threads overlap: validate their partitions individually, never
sum them into elapsed export time. Output validation is outside timed exports.
"""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import time

from pr459_job import build_job
from run import clean_environment, diagnostics, resources, write_json
from stages import check_profile
import verify


def check_pr(profile, frames):
    names = {t['name'] for t in profile['threads']}
    assert names >= {'decode', 'render', 'encode'}, names
    for thread in profile['threads']:
        spans = thread['spans']
        for inclusive, exclusive in [('inclusive_ms', 'exclusive_ms'), ('inclusive_cpu_ms', 'exclusive_cpu_ms')]:
            assert abs(sum(s[exclusive] for s in spans.values()) - spans['worker'][inclusive]) < 0.01, thread['name']
        if thread['name'] == 'render':
            assert spans['process_frame']['calls'] == spans['skia_render']['calls'] == frames
            assert thread['observations']['composite_gpu']['calls'] == frames
    return {'partition': 'each thread checked independently', 'profiled_frames': frames}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('current', 'pr459', 'output', 'telemetry'):
        parser.add_argument('--' + key, type=Path, required=True)
    parser.add_argument('--pairs', type=int, default=3)
    parser.add_argument('--controls', type=int, default=1)
    parser.add_argument('--only', choices=['current', 'pr459'])
    parser.add_argument('--revision', required=True)
    parser.add_argument('--cases', type=Path, help='Explicit benchmark cases: name, kind (current/pr459), binary, optional args')
    parser.add_argument('--references', type=Path, help='Reuse verified source/reference assets from a previous results.json')
    parser.add_argument('--defer-validation', action='store_true', help='Run all serial timing jobs before validating their outputs')
    parser.add_argument('--current-extra', action='append', default=[])
    args = parser.parse_args()
    assert platform.system() == 'Darwin' and args.pairs >= 0 and args.controls >= 0
    for key in ('current', 'pr459', 'telemetry'):
        setattr(args, key, getattr(args, key).resolve(strict=True))
    args.output = args.output.resolve(); args.output.mkdir(parents=True, exist_ok=True)
    receipt_path = args.output / 'results.json'
    if receipt_path.exists():
        parser.error('Use a fresh output directory')
    lock = Path(tempfile.gettempdir()) / 'bsr-export-comparison-gpu.lock'
    environment, removed = clean_environment()
    environment.update(BSR_FFMPEG_LIBRARY_DIR='/opt/homebrew/opt/ffmpeg-full/lib', BSR_FFMPEG_PREFIXES='/opt/homebrew/opt/ffmpeg-full')
    root = Path(__file__).resolve().parent
    ffmpeg, ffprobe = [Path('/opt/homebrew/opt/ffmpeg-full/bin') / name for name in ('ffmpeg', 'ffprobe')]
    scenarios = {'full': root / 'assets/scenario.json', 'warmup': root / 'assets/warmup.json'}
    assets = set(scenarios.values())
    for scene in scenarios.values():
        value = json.loads(scene.read_text())
        assets.add(scene.parent / value['source'])
        assets.update(scene.parent / v['path'] for k in ('documents', 'overlays') for v in value[k])
    fixture_hashes = {str(p): verify.sha256(p) for p in sorted(assets)}
    cases = json.loads(args.cases.read_text()) if args.cases else [
        {'name': k, 'kind': k, 'binary': str(getattr(args, k)), 'args': args.current_extra if k == 'current' else []}
        for k in ([args.only] if args.only else ['current', 'pr459'])]
    assert cases and len({c['name'] for c in cases}) == len(cases)
    for c in cases:
        assert c['kind'] in ('current', 'pr459') and c['name'].replace('-', '').replace('_', '').isalnum()
        c['binary'] = str(Path(c['binary']).resolve(strict=True)); c['binary_sha256'] = verify.sha256(c['binary'])
    receipt = {'status': 'running', 'jobs': [], 'metadata': {
        'revision': args.revision, 'cases': cases, 'fixture_hashes': fixture_hashes,
        'started_at': datetime.now(timezone.utc).isoformat(),
        'platform': platform.platform(), 'removed_environment_names': removed,
        'binary_sha256': {k: verify.sha256(getattr(args, k)) for k in ('current', 'pr459', 'telemetry')},
        'method_sha256': {k: verify.sha256(root / k) for k in ('compare_stages.py', 'verify.py', 'run.py', 'profiling/instrument_pr459.py', 'profiling/pr459_profile.hpp', 'profiling/telemetry.mm')},
        'arguments': {k: str(v) for k, v in vars(args).items()},
        'timing': 'External process wall; telemetry once/s; complete output validation excluded.',
        'cpu_semantics': 'Calling-thread CPU excludes other codec/driver threads; process CPU includes them.',
    }}
    fd = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
    os.write(fd, json.dumps({'pid': os.getpid(), 'output': str(args.output)}).encode()); os.close(fd)
    try:
        if args.references:
            references = json.loads(args.references.read_text())['references']
            for label, scene in scenarios.items():
                ref = references[label]
                assert verify.sha256(Path(ref['source'])) == ref['source_sha256']
                assert all(p['renderer']['scenario_sha256'] == verify.sha256(scene) for p in ref['samples'])
                assert all(Path(p[k]).is_file() for p in ref['samples'] for k in ('source', 'expected', 'mask', 'overlay'))
        else:
            references = {label: verify.prepare_reference(scene, args.current, ffmpeg, ffprobe, args.output / ('reference-' + label), environment) for label, scene in scenarios.items()}
        receipt['references'] = references
        variants = [c['name'] for c in cases]
        by_name = {c['name']: c for c in cases}
        pending_validation = []

        def validate(result, output, scene, fixture, name):
            result['validation'] = verify.verify_output(output, scene, references[fixture], ffmpeg, ffprobe, args.output / ('verify-' + name), environment)
            result['output_sha256'] = verify.sha256(output)
            output.unlink(); result['verified_media_removed'] = True
            write_json(receipt_path, receipt)
        jobs = [('warmup', 'warmup', v, True) for v in variants]
        for i in range(max(args.pairs, args.controls)):
            for enabled, number in [(False, args.controls), (True, args.pairs)]:
                if i < number:
                    jobs += [(f'{"profile" if enabled else "control"}-{i}', 'full', v, enabled) for v in (variants if i % 2 == 0 else variants[::-1])]
        for label, fixture, variant, measured in jobs:
            name = label + '-' + variant; scene = scenarios[fixture]
            output = args.output / (name + '.mp4')
            out, err, usage, profile_path = [args.output / (name + s) for s in ('.stdout', '.stderr', '.time', '.profile.json')]
            env = environment.copy(); case = by_name[variant]; kind = case['kind']
            measured = measured and case.get('profiling', True)
            if kind == 'current':
                command = [case['binary'], '--scenario', str(scene), '--output', str(output), '--require-gpu', *case.get('args', [])]
                if measured: command += ['--measure-stages']
            else:
                job = args.output / (name + '.job.json'); write_json(job, build_job(scene, output, 'macos'))
                command = [case['binary'], '--gpu-export', str(job), *case.get('args', [])]
                if measured: env['BSR_STAGE_PROFILE'] = str(profile_path)
            print('begin ' + name, flush=True)
            with (args.output / (name + '.telemetry.jsonl')).open('wb') as telem, out.open('wb') as stdout, err.open('wb') as stderr:
                monitor = subprocess.Popen([str(args.telemetry)], stdout=telem)
                try:
                    started = time.perf_counter()
                    process = subprocess.run(['/usr/bin/time', '-l', '-o', str(usage), *command], stdout=stdout, stderr=stderr, env=env, timeout=1800)
                    wall = time.perf_counter() - started
                finally:
                    monitor.terminate(); monitor.wait(timeout=10)
            result = {'name': name, 'fixture': fixture, 'variant': variant, 'measured': measured, 'command': command,
                      'process_wall_seconds': wall, 'exit_code': process.returncode, 'resources': resources(usage, wall)}
            receipt['jobs'].append(result); write_json(receipt_path, receipt)
            if process.returncode: raise RuntimeError(f'{name} failed; see {err}')
            if kind == 'current':
                diag = diagnostics(out); assert diag['profile']['frame_processor']
                result['diagnostics'] = diag
                if measured:
                    profile = diag['stage_profile']; write_json(profile_path, profile)
                    result['profile_validation'] = check_profile(profile, diag['frames'])
            elif measured:
                profile = json.loads(profile_path.read_text())
                result['profile_validation'] = check_pr(profile, 18000 if fixture == 'full' else 600)
            if measured: result['stage_profile'] = profile
            print(f'end {name} {wall:.3f}s', flush=True)
            result['telemetry'] = [json.loads(s) for s in (args.output / (name + '.telemetry.jsonl')).read_text().splitlines()]
            write_json(receipt_path, receipt)
            item = (result, output, scene, fixture, name)
            if args.defer_validation: pending_validation.append(item)
            else: validate(*item)
        for item in pending_validation:
            print('validate ' + item[-1], flush=True)
            validate(*item)
        assert all(verify.sha256(Path(p)) == digest for p, digest in fixture_hashes.items())
        assert all(verify.sha256(Path(c['binary'])) == c['binary_sha256'] for c in cases)
        receipt['status'] = 'complete'
    except BaseException as error:
        receipt.update(status='failed', error=repr(error)); raise
    finally:
        receipt['finished_at'] = datetime.now(timezone.utc).isoformat(); write_json(receipt_path, receipt); lock.unlink(missing_ok=True)


if __name__ == '__main__':
    main()
