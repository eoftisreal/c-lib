#!/usr/bin/env python3
"""Separate component implementation evidence from complete-picture acceptance."""
import argparse
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def render(catalog, report):
    intro = """The native Rust decoder reconstructs a verified, constrained subset of AV1 key
pictures from compressed OBU bytes. **No complete AV1 profile is supported.**

The original lossless path covers 8/10-bit 4:2:0, 64x64 superblocks, rectangular
4..64 coding blocks and 4x4 WHT transforms, with real DC/AC coefficients. DC,
zero-delta V/H, smooth, Paeth, CfL, palette and filter-intra are connected.
Independent tiles and multiple tile groups are supported in this subset.
The seven original oracle pictures match all 209,664 native reference samples.

A second verified path decodes **lossy 4x4 coding blocks** with real entropy-coded
transform selection, all seven 4x4 intra transform types (DCT/ADST combinations,
identity, vertical/horizontal DCT), their scans and coefficient contexts, all
four quantizer CDF banks, per-plane quantizer deltas and integer inverse
transforms. The 4x4 path supports 8/10/12-bit native 4:2:0, 4:2:2 and 4:4:4.

The additional [27-picture corpus](../verification/intra-4x4-conformance.json)
compares every sample with independent libaom: 113,664 additional samples,
including lossy multi-tile output, reduced/full intra transform sets, and
lossless/lossy depth/chroma combinations. The depth/chroma encodings permit CfL,
smooth and Paeth; they do not establish exhaustive syntax coverage of those modes.
[84 independent coefficient vectors](../tests/oracles/lossy_coeff_vectors.c)
check every 4x4 intra transform type and quantizer bank, level/sign decoding and
the entropy cursor. Cargo does not link libaom at runtime.

## Entry-point limits

- Only complete key pictures, with 64x64 superblocks, are accepted.
- Lossy, 12-bit, 4:2:2 and 4:4:4 pictures require **4x4 coding blocks**.
  Larger lossy transforms and entropy-coded transform-depth selection remain
  unsupported; existing 4..64 inverse-transform component tests do not enable them.
- Deblocking levels, CDEF strengths/index bits and restoration must be zero/off.
  The decoder rejects active filtering before publishing pixels; filtering is not
  silently omitted. Quantization matrices, segmentation and delta-Q/LF are rejected.
- Inter pictures, 128x128 superblocks, oblique prediction, angle deltas, IntraBC,
  super-resolution and film grain remain unsupported.
- Dimensions must be multiples of eight, at most 4096x4096; packets at most 16 MiB.
- Failed or incomplete decoding leaves the caller's state unchanged. This API
  does not automatically refresh decoded picture slots; callers must finalize
  supported pixels through the existing picture lifecycle API.

The original `intra_lossless.obu` is unchanged and matches all reference pixels.
The separate [lossless manifest](../verification/core-conformance.json),
[4x4 manifest](../verification/intra-4x4-conformance.json), and
[pixel comparison CI](../.github/workflows/core-pixel-conformance.yml) report
these narrow gates. The broad fixture `intra_10bit.obu` remains unsupported:
a passing constrained 10-bit encoding does not pass that existing lossy fixture.
The same distinction applies to `intra_tiles.obu`, inter video, and MP4 playback.

The following table concerns **full feature/profile acceptance**, not the
passing constrained picture gates above."""
    lines = ['# AV1 support', '', intro, '',
             '| Profile | Name | Complete picture decoding |', '| --- | --- | --- |']
    for p in catalog['profiles']:
        lines.append(f'| {p["id"]} | {p["name"]} | {"Supported" if p["decoded_picture_support"] else "Unsupported"} |')
    lines += ['', '| Feature | Implementation | Picture acceptance | Evidence |', '| --- | --- | --- | --- |']
    for f in catalog['features']:
        states = [report.get('features', {}).get(k, 'BLOCKED') for k in f['acceptance_features']]
        status = 'FAIL' if 'FAIL' in states else 'PASS' if states and all(s == 'PASS' for s in states) else 'BLOCKED'
        lines.append(f'| {f["id"]} | {f["implementation"]} | {status} | [{f["evidence"]}](../{f["evidence"]}) |')
    return '\n'.join(lines) + '\n'


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--report', type=Path, default=ROOT / 'verification/conformance-report.json')
    p.add_argument('--output', type=Path, default=ROOT / 'docs/AV1_SUPPORT.md')
    p.add_argument('--check', action='store_true')
    args = p.parse_args()
    catalog = json.loads((ROOT / 'verification/features.json').read_text())
    for feature in catalog['features']:
        if not (ROOT / feature['evidence']).is_file():
            raise ValueError('missing feature evidence: ' + feature['evidence'])
    content = render(catalog, json.loads(args.report.read_text()))
    if args.check:
        return int(not args.output.is_file() or args.output.read_text() != content)
    args.output.write_text(content)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
