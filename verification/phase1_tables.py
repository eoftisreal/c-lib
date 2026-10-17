#!/usr/bin/env python3
"""Extract/check normative numeric tables from a pinned AV1 specification checkout.
Usage: python3 verification/phase1_tables.py /path/to/av1-spec [--write]
No network access or runtime decoder dependency is needed.
"""
import argparse
import ast
from pathlib import Path
import re
import subprocess

PIN = '5e04f3f75e73a5898d7616c47c52f032144b8f80'
ROOT = Path(__file__).resolve().parents[1]


def extract(text, name):
    match = re.search(r'\b' + name + r'\s*\[[^=]+?=\s*(\{)', text, re.S)
    if not match:
        raise ValueError('Missing table: ' + name)
    start = match.start(1)
    depth = 0
    for end in range(start, len(text)):
        depth += (text[end] == '{') - (text[end] == '}')
        if depth == 0:
            body = text[start:end + 1].replace('{', '[').replace('}', ']')
            body = re.sub(r'\b(\d+)\s*\*\s*(\d+)\b', lambda m: str(int(m[1]) * int(m[2])), body)
            return ast.literal_eval(body)
    raise ValueError('Unclosed table: ' + name)


def flatten(value):
    return [n for row in value for n in flatten(row)] if isinstance(value, list) else [value]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('spec', type=Path)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    sha = subprocess.check_output(['git', '-C', str(args.spec), 'rev-parse', 'HEAD'], text=True).strip()
    if sha != PIN:
        raise ValueError('Expected specification commit ' + PIN + ', got ' + sha)
    tables = (args.spec / '10.additional.tables.md').read_text()
    decode = (args.spec / '08.decoding.process.md').read_text()
    scan_names = re.findall(r'^(?:Default|Mrow|Mcol)_Scan_\d+x\d+(?=\[)', tables, re.M)
    groups = {
        'lossy': [
            ('INTRA_TX_SET1', extract(tables, 'Default_Intra_Tx_Type_Set1_Cdf')),
            ('INTRA_TX_SET2', extract(tables, 'Default_Intra_Tx_Type_Set2_Cdf')),
        ] + [
            (rust, extract(tables, spec))
            for rust, spec in [
                ('TXB_SKIP_BY_Q', 'Default_Txb_Skip_Cdf'),
                ('EOB_MULTI16_BY_Q', 'Default_Eob_Pt_16_Cdf'),
                ('EOB_EXTRA_BY_Q', 'Default_Eob_Extra_Cdf'),
                ('BASE_EOB_BY_Q', 'Default_Coeff_Base_Eob_Cdf'),
                ('BASE_BY_Q', 'Default_Coeff_Base_Cdf'),
                ('BR_BY_Q', 'Default_Coeff_Br_Cdf'),
                ('DC_SIGN_BY_Q', 'Default_Dc_Sign_Cdf'),
            ]
        ],
        'default_cdfs': [
            (rust, extract(tables, spec)[0] if quantized else extract(tables, spec))
            for rust, spec, quantized in [
                ('DEFAULT_KF_Y_MODE_CDF', 'Default_Intra_Frame_Y_Mode_Cdf', False),
                ('DEFAULT_UV_MODE_CDF', 'Default_Uv_Mode_Cfl_Not_Allowed_Cdf', False),
                ('DEFAULT_UV_MODE_CFL_CDF', 'Default_Uv_Mode_Cfl_Allowed_Cdf', False),
                ('DEFAULT_ANGLE_DELTA_CDF', 'Default_Angle_Delta_Cdf', False),
                ('DEFAULT_CFL_SIGN_CDF', 'Default_Cfl_Sign_Cdf', False),
                ('DEFAULT_CFL_ALPHA_CDF', 'Default_Cfl_Alpha_Cdf', False),
                ('DEFAULT_SKIP_CDFS', 'Default_Skip_Cdf', False),
                ('DEFAULT_TXB_SKIP_CDFS', 'Default_Txb_Skip_Cdf', True),
                ('DEFAULT_EOB_MULTI16_CDFS', 'Default_Eob_Pt_16_Cdf', True),
                ('DEFAULT_EOB_EXTRA_CDFS', 'Default_Eob_Extra_Cdf', True),
                ('DEFAULT_COEFF_BASE_EOB_CDFS', 'Default_Coeff_Base_Eob_Cdf', True),
                ('DEFAULT_COEFF_BASE_CDFS', 'Default_Coeff_Base_Cdf', True),
                ('DEFAULT_COEFF_BR_CDFS', 'Default_Coeff_Br_Cdf', True),
                ('DEFAULT_DC_SIGN_CDFS', 'Default_Dc_Sign_Cdf', True),
            ]
        ],
        'block_syntax': [('PALETTE_Y', extract(tables,'Default_Palette_Y_Mode_Cdf')), ('PALETTE_UV', extract(tables,'Default_Palette_Uv_Mode_Cdf')), ('FILTER_INTRA', extract(tables,'Default_Filter_Intra_Cdf'))],
        'palette': [
            ('PALETTE_Y_SIZE', extract(tables, 'Default_Palette_Y_Size_Cdf')),
            ('PALETTE_UV_SIZE', extract(tables, 'Default_Palette_Uv_Size_Cdf')),
        ] + [
            (f'PALETTE_{plane}_COLOR_{size}', extract(tables, f'Default_Palette_Size_{size}_{spec_plane}_Color_Cdf'))
            for plane, spec_plane in [('Y', 'Y'), ('UV', 'Uv')]
            for size in range(2, 9)
        ],
        'superres': [('UPSCALE_FILTER', extract(decode, 'Upscale_Filter'))],
        'filter_intra': [('FILTER_INTRA_MODE', extract(tables, 'Default_Filter_Intra_Mode_Cdf')),
                         ('INTRA_FILTER_TAPS', extract(tables, 'Intra_Filter_Taps'))],
        'scan': [(name.upper(), extract(tables, name)) for name in scan_names],
        'partition': [(f'PARTITION_W{w}', extract(tables, f'Default_Partition_W{w}_Cdf')) for w in (8, 16, 32, 64, 128)],
        'smooth': [(f'SMOOTH_{w}', extract(tables, f'Sm_Weights_Tx_{w}x{w}')) for w in (4, 8, 16, 32, 64)],
        'quant': [('DC_QLOOKUP', extract(decode, 'Dc_Qlookup')), ('AC_QLOOKUP', extract(decode, 'Ac_Qlookup'))],
    }
    for group, entries in groups.items():
        path = ROOT / f'crates/codec-av1/src/frame/{group + "_tables" if group != "default_cdfs" else group}.rs'
        if args.write:
            lines = [f'//! Normative AV1 numeric tables; generated by verification/phase1_tables.py.',
                     f'//! Specification commit: {PIN}.',
                     '//! <https://aomediacodec.github.io/av1-spec/>', '']
            for name, value in entries:
                def rust_type(v):
                    return f'[{rust_type(v[0])}; {len(v)}]' if isinstance(v,list) else 'u16'
                ty=rust_type(value)
                if group == 'superres' or name == 'INTRA_FILTER_TAPS':
                    ty = ty.replace('u16', 'i16')
                visibility = ('pub static' if group == 'default_cdfs' else
                              'pub(super) static' if group == 'lossy' else 'pub(super) const')
                lines.append(f'{visibility} {name}: {ty} = {value!r};')
            path.write_text('\n'.join(lines) + '\n')
        source = path.read_text()
        for name, value in entries:
            match = re.search(r'\b' + name + r':[^=]+=\s*(\[.*?\]);', source, re.S)
            if not match or ast.literal_eval(match[1]) != value:
                raise ValueError('Table mismatch: ' + name)
        print(f'{path.relative_to(ROOT)}: verified {sum(len(flatten(v)) for _, v in entries)} entries')


if __name__ == '__main__':
    main()
