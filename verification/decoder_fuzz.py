#!/usr/bin/env python3
"""Gate full decoder fuzzing on an actual pixel backend and dedicated target.

Primitive/header targets cannot satisfy this gate. The reviewed target must submit
compressed packets, drain real frames, exercise reset/flush/reference rollback and
assert budgets. Source review is still needed to establish this semantic contract.
"""
import argparse
import ctypes
import json
from pathlib import Path
import subprocess
import tomllib
from conformance import digest

ROOT = Path(__file__).resolve().parents[1]
PRIMITIVE_TARGETS = {'mp4', 'av1', 'av1_settings', 'av1_cdf', 'av1_blocks', 'av1_filters'}


def target_source(target, manifest):
    if not target or target in PRIMITIVE_TARGETS:
        return None
    return next((b['path'] for b in manifest['bin'] if b['name'] == target), None)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--library', type=Path, required=True)
    p.add_argument('--report', type=Path, required=True)
    p.add_argument('--seconds', type=int, default=60)
    args = p.parse_args()
    if not 1 <= args.seconds <= 3600:
        p.error('fuzz duration must be 1..3600 seconds')
    config = json.loads((ROOT / 'verification/decoder-tools.json').read_text())
    report = {'schema': 1, 'status': 'BLOCKED', 'reason': 'full compressed-picture decoder/target unavailable'}
    library = ctypes.CDLL(str(args.library.resolve()))
    library.codec_get_capabilities.restype = ctypes.c_uint64
    report['capabilities'] = library.codec_get_capabilities()
    report['library_sha256'] = digest(args.library)
    manifest = tomllib.loads((ROOT / 'fuzz/Cargo.toml').read_text())
    target = config.get('full_decoder_fuzz_target')
    source = target_source(target, manifest)
    if report['capabilities'] & 1 and source and (ROOT / 'fuzz' / source).is_file():
        command = ['cargo', '+nightly', 'fuzz', 'run', target, '--',
                   f'-max_total_time={args.seconds}', '-max_len=1048576', '-rss_limit_mb=2048', '-timeout=5']
        report.update(target=target, target_sha256=digest(ROOT / 'fuzz' / source), argv=command)
        try:
            result = subprocess.run(command, cwd=ROOT, timeout=args.seconds + 600, check=False)
            report.update(status='PASS' if result.returncode == 0 else 'FAIL', returncode=result.returncode,
                          reason='dedicated decoder fuzz process completed')
        except subprocess.TimeoutExpired:
            report.update(status='FAIL', reason='decoder fuzz watchdog expired')
    args.report.write_text(json.dumps(report, indent=2) + '\n')
    print(report['status'] + ': ' + report['reason'])
    return 0 if report['status'] == 'PASS' else 2 if report['status'] == 'BLOCKED' else 1


if __name__ == '__main__':
    raise SystemExit(main())
