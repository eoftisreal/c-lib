#!/usr/bin/env python3
"""Pixel-verified end-to-end decoder process benchmarks (startup and output I/O included)."""
import argparse
import json
import math
from pathlib import Path
import platform
import statistics
import subprocess
import tempfile
import time
import conformance

ROOT = Path(__file__).resolve().parents[1]


def cpu_seconds():
    try:
        import resource
        usage = resource.getrusage(resource.RUSAGE_CHILDREN)
        return usage.ru_utime + usage.ru_stime
    except ImportError:
        return None


def benchmark_case(case, root, config, comparator, timeout, repeats, warmups):
    verified = conformance.run([case], root, config.get('candidate'), config.get('reference'),
                               comparator, timeout)['cases'][0]
    row = {'id': case['id'], 'status': verified['status'], 'verification': verified}
    if verified['status'] != 'PASS':
        return row
    samples = []
    with tempfile.TemporaryDirectory(prefix='c-lib-bench-') as directory:
        source = Path(directory) / ('input' + Path(case['path']).suffix)
        import shutil
        shutil.copyfile(root / case['path'], source)
        if conformance.digest(source) != case['sha256']:
            raise ValueError('benchmark input changed')
        expected = verified['output_sha256']['candidate.yuv']
        for iteration in range(warmups + repeats):
            output = Path(directory) / 'output.yuv'
            output.unlink(missing_ok=True)
            cpu_start = cpu_seconds()
            start = time.perf_counter_ns()
            result = conformance.execute(config['candidate'], {'input': str(source), 'output': str(output)}, timeout)
            elapsed = (time.perf_counter_ns() - start) / 1e9
            cpu_end = cpu_seconds()
            if result['returncode'] or not output.is_file() or conformance.digest(output) != expected:
                raise ValueError('timed decode did not reproduce verified native pixels')
            if iteration >= warmups:
                samples.append({'seconds': elapsed, 'cpu_seconds': None if cpu_start is None else cpu_end - cpu_start})
    durations = sorted(s['seconds'] for s in samples)
    median = statistics.median(durations)
    row.update(status='PASS', samples=samples, frames=case['frames'], median_seconds=median,
               frames_per_second=case['frames']/median, p95_process_seconds=durations[math.ceil(.95*len(durations))-1],
               allocations=None, dropped_frames=None, peak_rss=None)
    return row


def regressions(current, baseline, limit):
    errors = []
    if current.get('context') != baseline.get('context'):
        errors.append('benchmark contexts differ; baseline must use the same runner/build configuration/workloads')
    old = {r['id']: r for r in baseline.get('cases', [])}
    new = current.get('cases', [])
    if not old or len(old) != len(baseline.get('cases', [])) or len(new) != len(old) or {r['id'] for r in new} != set(old):
        errors.append('missing, duplicate or changed benchmark workloads')
    for row in new:
        previous = old.get(row['id'], {})
        if row.get('status') != 'PASS' or previous.get('status') != 'PASS':
            errors.append(row['id'] + ': blocked/failed benchmark cannot satisfy a regression gate')
            continue
        a, b = row.get('median_seconds'), previous.get('median_seconds')
        if not all(isinstance(v, (int, float)) and math.isfinite(v) and v > 0 for v in (a, b)):
            errors.append(row['id'] + ': invalid timing')
        elif a > b * (1 + limit / 100):
            errors.append(f'{row["id"]}: median decode time regressed {(a/b-1)*100:.2f}% (limit {limit}%)')
        for result in (row, previous):
            samples = result.get('samples', [])
            durations = [sample.get('seconds') for sample in samples]
            if len(samples) < 5 or not all(isinstance(v, (int, float)) and math.isfinite(v) and v > 0 for v in durations):
                errors.append(row['id'] + ': at least five positive finite measurements required')
            elif not math.isclose(statistics.median(durations), result['median_seconds'], rel_tol=1e-9):
                errors.append(row['id'] + ': median does not match recorded samples')

    return errors


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--manifest', type=Path, default=ROOT / 'verification/benchmark.json')
    p.add_argument('--tools', type=Path, default=ROOT / 'verification/decoder-tools.json')
    p.add_argument('--report', type=Path, required=True)
    p.add_argument('--comparator', type=Path, required=True)
    p.add_argument('--runner-id', required=True, help='stable dedicated hardware/configuration identifier')
    p.add_argument('--build-id', required=True, help='compiler, target, flags and thread configuration')
    p.add_argument('--revision', required=True)
    p.add_argument('--repeats', type=int, default=7)
    p.add_argument('--warmups', type=int, default=1)
    p.add_argument('--timeout', type=float, default=120)
    p.add_argument('--baseline', type=Path)
    p.add_argument('--max-regression-percent', type=float, default=10)
    args = p.parse_args()
    if args.repeats < 5 or args.warmups < 1 or not math.isfinite(args.timeout) or args.timeout <= 0:
        p.error('at least five repeats, one warmup and positive finite timeout required')
    if not math.isfinite(args.max_regression_percent) or args.max_regression_percent < 0:
        p.error('regression threshold must be finite and nonnegative')
    config = json.loads(args.tools.read_text())
    if config.get('candidate') == config.get('reference') and config.get('candidate'):
        p.error('independent candidate/reference required')
    manifest = json.loads(args.manifest.read_text())
    report = {'schema': 1, 'measurement': 'end-to-end-process', 'revision': args.revision,
              'reference_revision': config.get('reference_revision'),
              'context': {'runner': args.runner_id, 'build': args.build_id, 'platform': platform.platform(),
                          'machine': platform.machine(), 'manifest_sha256': conformance.digest(args.manifest),
                          'commands': config, 'warmups': args.warmups, 'repeats': args.repeats}, 'cases': []}
    for case in manifest['cases']:
        try:
            row = benchmark_case(case, ROOT, config, args.comparator.resolve(), args.timeout, args.repeats, args.warmups)
        except (OSError, subprocess.TimeoutExpired, ValueError, KeyError) as error:
            row = {'id': case['id'], 'status': 'FAIL', 'reason': str(error)}
        report['cases'].append(row)
    report['regressions'] = regressions(report, json.loads(args.baseline.read_text()), args.max_regression_percent) if args.baseline else []
    args.report.write_text(json.dumps(report, indent=2, allow_nan=False) + '\n')
    if report['regressions'] or any(r['status'] == 'FAIL' for r in report['cases']):
        return 1
    return 0 if report['cases'] and all(r['status'] == 'PASS' for r in report['cases']) else 2


if __name__ == '__main__':
    raise SystemExit(main())
