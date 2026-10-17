#!/usr/bin/env python3
"""Run compressed-picture acceptance tests, never parser/component substitutes.

Commands are JSON arrays (no shell); {input} and {output} are replaced in argv.
The caller is responsible for using a genuinely independent c-lib candidate.
"""
import argparse
import hashlib
import math
import json
from pathlib import Path
import subprocess
import shutil
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(65536), b''):
            h.update(chunk)
    return h.hexdigest()


def execute(argv, substitutions, timeout):
    command = list(argv)
    for key, value in substitutions.items():
        command = [part.replace('{' + key + '}', value) for part in command]
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        result = subprocess.run(command, stdout=stdout, stderr=stderr,
                                timeout=timeout, check=False)
        def tail(stream):
            stream.seek(0, 2)
            stream.seek(max(0, stream.tell() - 4096))
            return stream.read(4096).decode('utf-8', 'replace')
        return {'argv': command, 'returncode': result.returncode,
                'stdout': tail(stdout), 'stderr': tail(stderr)}


def run_case(case, root, candidate, reference, comparator, timeout):
    row = {'id': case['id'], 'features': case['features'], 'status': 'BLOCKED'}
    source = root / case['path'] if case.get('path') else None
    if source is None or not source.is_file():
        row['reason'] = 'fixture not installed; acceptance coverage missing'
        return row
    row['input_sha256'] = digest(source)
    if row['input_sha256'] != case['sha256']:
        row.update(status='FAIL', reason='fixture SHA-256 mismatch')
        return row
    if not candidate:
        row['reason'] = 'c-lib compressed-picture decoder unavailable'
        return row
    if case.get('layout') not in ('420', '422', '444'):
        row['reason'] = 'native output layout unsupported by YUV comparator (no color conversion permitted)'
        return row
    if not reference or not comparator:
        row['reason'] = 'independent reference decoder and yuv-compare required'
        return row
    # New directory per case prevents stale or shared output from passing.
    with tempfile.TemporaryDirectory(prefix='c-lib-conformance-') as directory:
        snapshot = Path(directory) / ('input' + source.suffix)
        shutil.copyfile(source, snapshot)
        if digest(snapshot) != row['input_sha256']:
            row.update(status='FAIL', reason='fixture changed while preparing decode')
            return row
        outputs = [Path(directory) / name for name in ('candidate.yuv', 'reference.yuv')]
        for label, argv, output in zip(('candidate', 'reference'), (candidate, reference), outputs):
            result = execute(argv, {'input': str(snapshot), 'output': str(output)}, timeout)
            row[label] = result
            if result['returncode'] != 0 or not output.is_file():
                row.update(status='FAIL', reason=f'{label} failed to decode')
                return row
        # Exact expected count rejects decoders which both omit the same frames.
        w, h, depth = case['width'], case['height'], case['bit_depth']
        sx, sy = {'420': (2, 2), '422': (2, 1), '444': (1, 1)}[case['layout']]
        count = (w*h + 2*((w+sx-1)//sx)*((h+sy-1)//sy)) * (1 if depth == 8 else 2) * case['frames']
        for output in outputs:
            if output.stat().st_size != count:
                row.update(status='FAIL', reason=f'{output.name}: expected exactly {count} bytes')
                return row
        row['output_sha256'] = {p.name: digest(p) for p in outputs}
        comparison = execute([str(comparator), '--expected', str(outputs[1]), '--actual', str(outputs[0]),
                              '--width', str(w), '--height', str(h), '--bit-depth', str(depth),
                              '--subsampling', case['layout']], {}, timeout)
        row['comparison'] = comparison
        row.update(status='PASS' if comparison['returncode'] == 0 else 'FAIL',
                   reason='pixel comparison' if comparison['returncode'] == 0 else comparison['stderr'])
    return row


def run(cases, root, candidate=None, reference=None, comparator=None, timeout=60):
    rows = []
    for case in cases:
        try:
            row = run_case(case, root, candidate, reference, comparator, timeout)
        except (OSError, subprocess.TimeoutExpired, ValueError, KeyError) as error:
            row = {'id': case.get('id'), 'features': case.get('features', []),
                   'status': 'FAIL', 'reason': str(error)}
        rows.append(row)
    features = sorted({feature for row in rows for feature in row['features']})
    matrix = {}
    for feature in features:
        statuses = [row['status'] for row in rows if feature in row['features']]
        matrix[feature] = 'FAIL' if 'FAIL' in statuses else 'BLOCKED' if 'BLOCKED' in statuses else 'PASS'
    return {'schema': 1, 'scope': 'compressed-picture acceptance only',
            'candidate_identity': 'caller-supplied; independence requires source review',
            'cases': rows, 'features': matrix}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, default=ROOT / 'verification/conformance.json')
    parser.add_argument('--root', type=Path, default=ROOT)
    parser.add_argument('--candidate', type=json.loads, help='JSON argv for independent c-lib decoder')
    parser.add_argument('--reference', type=json.loads, help='JSON argv for libaom/dav1d raw output')
    parser.add_argument('--candidate-revision', help='c-lib commit/build identity (required with candidate)')
    parser.add_argument('--reference-revision', help='libaom/dav1d version and build identity')
    parser.add_argument('--comparator', type=Path)
    parser.add_argument('--timeout', type=float, default=60)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    for argv in (args.candidate, args.reference):
        if argv is not None and (not isinstance(argv, list) or not argv or
                                 not all(isinstance(part, str) for part in argv)):
            parser.error('decoder commands must be nonempty JSON arrays of strings')
    if args.candidate and (not args.candidate_revision or not args.reference_revision):
        parser.error('candidate and reference build revisions are required for a decode run')
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error('timeout must be positive')
    if args.candidate and args.candidate == args.reference:
        parser.error('candidate and reference commands must differ')
    manifest = json.loads(args.manifest.read_text())
    report = run(manifest['cases'], args.root, args.candidate, args.reference,
                 args.comparator.resolve() if args.comparator else None, args.timeout)
    report['manifest_sha256'] = digest(args.manifest)
    report['commands'] = {'candidate': args.candidate, 'reference': args.reference}
    report['revisions'] = {'candidate': args.candidate_revision, 'reference': args.reference_revision}
    report['executable_sha256'] = {}
    for label, argv in [('candidate', args.candidate), ('reference', args.reference),
                        ('comparator', [str(args.comparator)] if args.comparator else None)]:
        if argv:
            executable = shutil.which(argv[0])
            if executable:
                report['executable_sha256'][label] = digest(Path(executable))
    args.report.write_text(json.dumps(report, indent=2) + '\n')
    statuses = [row['status'] for row in report['cases']]
    # Blocked is deliberately not green CI; empty suites cannot pass.
    return 1 if 'FAIL' in statuses else 2 if not statuses or 'BLOCKED' in statuses else 0


if __name__ == '__main__':
    raise SystemExit(main())
