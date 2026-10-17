#!/usr/bin/env python3
"""Independently encode/decode constrained real AV1 key pictures for pixel gates.

Use pinned libaom aomenc/aomdec from tests/oracles/README.md. FFmpeg/libaom is
used for 12-bit 422/444 because the pinned aomenc CLI rejects those raw inputs;
its version and executable hash are recorded. Reference decoding always uses
the pinned unmodified aomdec. No c-lib code produces reference pixels.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def cases():
    for reduced in (True, False):
        for q in (4, 12, 24, 48):
            yield (f'lossy_4x4_{"" if reduced else "full_"}q{q:02}',
                   8, '420', 64, False, q, reduced, False)
    for depth in (8, 10, 12):
        for layout in ('420', '422', '444'):
            for lossless in (True, False):
                yield (f'intra_4x4_{depth}_{layout}_{"lossless" if lossless else "lossy"}',
                       depth, layout, 32, lossless, 24, True, False)
    yield ('lossy_4x4_tiles', 8, '420', 128, False, 24, False, True)


def generate(aom_build, output, ffmpeg):
    entries = []
    reference_dir = output / 'tests' / 'oracles'
    reference_dir.mkdir(parents=True, exist_ok=True)
    (output / 'verification').mkdir(parents=True, exist_ok=True)
    for name, depth, layout, size, lossless, q, reduced, tiled in cases():
        sx, sy = {'420': (1, 1), '422': (1, 0), '444': (0, 0)}[layout]
        source = bytearray()
        for plane in range(3):
            width = size if plane == 0 else size >> sx
            height = size if plane == 0 else size >> sy
            for y in range(height):
                for x in range(width):
                    value = 20 + (x * 7 + y * 11 + (x * y) % 17 + plane * 41) % 216
                    if depth > 8:
                        value = (value << (depth - 8)) + ((x + 3*y + plane) % (1 << (depth - 8)))
                    source.extend(value.to_bytes(1 if depth == 8 else 2, 'little'))
        encoded = output / 'tests' / (name + '.obu')
        reference = reference_dir / (name + '.yuv')
        params = [
            'sb-size=64', 'min-partition-size=4', 'max-partition-size=4',
            f'reduced-tx-type-set={int(reduced)}', 'enable-directional-intra=0',
            'enable-palette=0', 'enable-filter-intra=0', 'enable-cfl-intra=0',
            'enable-smooth-intra=0', 'enable-paeth-intra=0', 'enable-cdef=0',
            'enable-restoration=0', 'loopfilter-control=0', 'deltaq-mode=0', 'enable-qm=0',
        ]
        if name.startswith('intra_4x4_'):
            params = [p.replace('enable-cfl-intra=0', 'enable-cfl-intra=1')
                      .replace('enable-smooth-intra=0', 'enable-smooth-intra=1')
                      .replace('enable-paeth-intra=0', 'enable-paeth-intra=1') for p in params]
        if tiled:
            params += ['tile-columns=1', 'tile-rows=1', 'num-tile-groups=4']
        use_ffmpeg = depth == 12 and layout != '420'
        with tempfile.TemporaryDirectory(prefix='c-lib-intra-source-') as temporary:
            raw = Path(temporary) / 'source.yuv'
            raw.write_bytes(source)
            if use_ffmpeg:
                command = [str(ffmpeg), '-nostdin', '-hide_banner', '-loglevel', 'error',
                           '-f', 'rawvideo', '-pix_fmt', f'yuv{layout}p{depth}le', '-s', f'{size}x{size}',
                           '-i', str(raw), '-frames:v', '1', '-c:v', 'libaom-av1', '-cpu-used', '0',
                           '-crf', str(0 if lossless else q), '-b:v', '0', '-aom-params', ':'.join(params),
                           '-f', 'obu', '-y', str(encoded)]
            else:
                profile = 2 if depth == 12 or layout == '422' else 1 if layout == '444' else 0
                command = [str(aom_build / 'aomenc'), '--obu', f'--i{layout}', f'--profile={profile}',
                           f'--bit-depth={depth}', f'--input-bit-depth={depth}', f'--width={size}',
                           f'--height={size}', '--fps=1/1', '--limit=1', '--cpu-used=0']
                command += ['--' + param for param in params]
                command += ['--lossless=1'] if lossless else ['--end-usage=q', f'--cq-level={q}']
                command += ['-o', str(encoded), str(raw)]
            subprocess.run(command, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            subprocess.run([str(aom_build / 'aomdec'), '--rawvideo', '-o', str(reference), str(encoded)],
                           check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        expected = reference.read_bytes()
        if len(expected) != len(source) or (expected == source) != lossless:
            raise RuntimeError(f'{name}: incorrect reference length or lossless/lossy classification')
        entries.append(dict(id=encoded.name, path=f'tests/{encoded.name}', sha256=digest(encoded),
                            width=size, height=size, bit_depth=depth, layout=layout, frames=1,
                            features=['4x4-coding-blocks', 'lossless' if lossless else 'lossy',
                                      'four-tiles' if tiled else 'single-tile',
                                      'reduced-tx-set' if reduced else 'full-intra-tx-set', 'no-loop-filters'],
                            oracle=f'tests/oracles/{reference.name}', oracle_sha256=digest(reference),
                            source_sha256=hashlib.sha256(source).hexdigest(),
                            encoder='ffmpeg-libaom' if use_ffmpeg else 'pinned-aomenc'))
        print(name)
    metadata = dict(schema=1, scope='constrained 4x4 intra pictures; not full AV1 acceptance',
                    libaom_revision='394f9f7df624e4268fd8c894ead9f059d279e480',
                    aomenc_sha256=digest(aom_build / 'aomenc'), aomdec_sha256=digest(aom_build / 'aomdec'),
                    ffmpeg_sha256=digest(ffmpeg),
                    ffmpeg_version=subprocess.check_output([str(ffmpeg), '-version'], text=True), cases=entries)
    (output / 'verification' / 'intra-4x4-conformance.json').write_text(json.dumps(metadata, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--aom-build', required=True, type=Path)
    parser.add_argument('--output-dir', required=True, type=Path)
    parser.add_argument('--ffmpeg', default=shutil.which('ffmpeg'), type=Path)
    args = parser.parse_args()
    generate(args.aom_build.resolve(), args.output_dir.resolve(), args.ffmpeg.resolve())
