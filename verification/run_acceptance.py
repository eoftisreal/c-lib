#!/usr/bin/env python3
"""Run the reviewed decoder command configuration, then the strict acceptance gate."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--revision', required=True)
    p.add_argument('--report', type=Path, required=True)
    p.add_argument('--tools', type=Path, default=ROOT / 'verification/decoder-tools.json')
    p.add_argument('--comparator', type=Path, required=True)
    args = p.parse_args()
    config = json.loads(args.tools.read_text())
    command = [sys.executable, str(ROOT / 'verification/conformance.py'), '--report', str(args.report)]
    if config.get('candidate'):
        command += ['--candidate', json.dumps(config['candidate']), '--reference', json.dumps(config['reference']),
                    '--candidate-revision', args.revision, '--reference-revision', config['reference_revision'],
                    '--comparator', str(args.comparator)]
    result = subprocess.run(command, check=False)
    if result.returncode not in (0, 1, 2):
        return result.returncode
    if not args.report.is_file():
        return 1
    return subprocess.run([sys.executable, str(ROOT / 'verification/acceptance_gate.py'),
                           '--report', str(args.report), '--revision', args.revision], check=False).returncode


if __name__ == '__main__':
    raise SystemExit(main())
