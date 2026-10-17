#!/usr/bin/env python3
"""Fail closed unless a fresh full-suite compressed-picture report passed."""
import argparse
import json
from pathlib import Path
from conformance import digest


def failures(report, manifest, manifest_hash, revision):
    errors = []
    if report.get('manifest_sha256') != manifest_hash:
        errors.append('report does not match the pinned manifest')
    expected = {case['id'] for case in manifest['cases']}
    rows = report.get('cases', [])
    if not expected or len(rows) != len(expected) or {row.get('id') for row in rows} != expected:
        errors.append('missing, duplicate, extra or empty acceptance cases')
    by_id = {case['id']: case for case in manifest['cases']}
    for row in rows:
        name = row.get('id')
        if row.get('status') != 'PASS':
            status = row.get('status', 'MISSING')
            reason = row.get('reason', 'picture acceptance has not passed')
            errors.append(f'{name}: {status}: {reason}')
            # No decoded output or subprocess evidence exists for an unrun
            # case. Report the actual blocker, not invented decoder failures.
            continue
        case = by_id.get(name, {})
        if not case.get('sha256') or row.get('input_sha256') != case.get('sha256'):
            errors.append(f'{name}: missing or changed fixture hash')
        outputs = row.get('output_sha256', {})
        if not outputs.get('candidate.yuv') or outputs.get('candidate.yuv') != outputs.get('reference.yuv'):
            errors.append(f'{name}: no matching native pixel digests')
        for stage in ('candidate', 'reference', 'comparison'):
            if row.get(stage, {}).get('returncode') != 0:
                errors.append(f'{name}: {stage} did not succeed')
    commands = report.get('commands', {})
    if not commands.get('candidate') or not commands.get('reference'):
        errors.append('decoder command adapters are not configured; picture comparisons were not run')
    else:
        if commands['candidate'] == commands['reference']:
            errors.append('distinct candidate/reference commands required')
        versions = report.get('revisions', {})
        if not revision or versions.get('candidate') != revision or not versions.get('reference'):
            errors.append('candidate must be this commit and reference build must be identified')
        if not all(report.get('executable_sha256', {}).get(k) for k in ('candidate', 'reference', 'comparator')):
            errors.append('executable hashes required')
    required_features = {f for case in manifest['cases'] for f in case['features']}
    # Derive coverage from cases too; a manually green features map cannot hide a blocked case.
    if not required_features or any(report.get('features', {}).get(f) != 'PASS' for f in required_features):
        errors.append('feature coverage incomplete')
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, default=Path(__file__).with_name('conformance.json'))
    parser.add_argument('--revision', required=True)
    args = parser.parse_args()
    errors = failures(json.loads(args.report.read_text()), json.loads(args.manifest.read_text()),
                      digest(args.manifest), args.revision)
    if errors:
        print('Decoder acceptance NOT READY (no conformance pass recorded)')
    for error in errors:
        print(error)
    return int(bool(errors))


if __name__ == '__main__':
    raise SystemExit(main())
