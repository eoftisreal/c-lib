#!/usr/bin/env python3
"""Check the ABI snapshot and enforce major/minor changes relative to a base ref."""
import argparse
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SNAPSHOT = ROOT / 'verification/api-contract.json'


def declarations(header):
    text = re.sub(r'/\*.*?\*/|//[^\n]*', '', header, flags=re.S)
    macros = re.findall(r'^#define (CODEC_(?!H\b)\w+[^\n]*)', text, re.M)
    text = re.sub(r'^\s*#.*$', '', text, flags=re.M).replace('extern "C" {', '')
    pattern = (r'typedef\s+(?:struct|enum)\s+\w+\s*\{[^}]*\}\s*\w+\s*;'
               r'|typedef\s+[^;{}]+;'
               r'|[^;{}\n]*\bcodec_\w+\([^;{}]*\);')
    return sorted(' '.join(m.split()) for m in re.findall(pattern, text, re.S) + macros)



def compatibility(old, new):
    a = (old['major'], old['minor'])
    b = (new['major'], new['minor'])
    if b < a:
        return ['ABI version decreased']
    removed = set(old['declarations']) - set(new['declarations'])
    added = set(new['declarations']) - set(old['declarations'])
    if removed and b[0] <= a[0]:
        return ['changed/removed ABI declaration requires a major increment']
    if added and b <= a:
        return ['additive ABI declarations require a minor or major increment']
    return []


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--write', action='store_true')
    p.add_argument('--base', help='git ref containing the previous contract')
    args = p.parse_args()
    source = (ROOT / 'crates/codec-ffi/src/lib.rs').read_text()
    number = int(re.search(r'fn codec_get_abi_version\(\) -> u32\s*\{\s*(0x[\da-fA-F_]+)', source)[1].replace('_', ''), 16)
    current = dict(schema=1, major=number >> 16, minor=number & 65535,
                   declarations=declarations((ROOT / 'include/codec.h').read_text()))
    if args.write:
        SNAPSHOT.write_text(json.dumps(current, indent=2) + '\n')
    errors = [] if SNAPSHOT.is_file() and json.loads(SNAPSHOT.read_text()) == current else ['C ABI snapshot drift']
    if args.base:
        previous = subprocess.run(['git', 'show', args.base + ':verification/api-contract.json'],
                                  cwd=ROOT, capture_output=True, text=True, check=False)
        if previous.returncode == 0:
            errors += compatibility(json.loads(previous.stdout), current)
        else:
            # Initial introduction is allowed only when the base commit exists
            # and does not yet contain a contract. Missing/fetch failures are errors.
            exists = subprocess.run(['git', 'cat-file', '-e', args.base + '^{commit}'], cwd=ROOT, check=False)
            if exists.returncode:
                errors.append('comparison base unavailable')
    for error in errors:
        print(error)
    return int(bool(errors))


if __name__ == '__main__':
    raise SystemExit(main())
