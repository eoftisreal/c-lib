#!/usr/bin/env python3
"""Build/test/package Linux x86_64 experimental library artifacts reproducibly.

Two clean offline builds must produce identical binaries. Source revision must be
clean. Does not publish a GitHub Release or assert decoder production readiness.
"""
import argparse
import gzip
import io
import json
import os
from pathlib import Path
import platform
import subprocess
import shutil
import tarfile
import tempfile
import tomllib
import ctypes
from conformance import digest

ROOT = Path(__file__).resolve().parents[1]
TARGET = 'x86_64-unknown-linux-gnu'
BINARIES = ('codec-cli', 'yuv-compare', 'libcodec_ffi.so')


def command(argv, **kwargs):
    return subprocess.check_output(argv, cwd=ROOT, **kwargs)


def archive(files, output, epoch):
    with output.open('wb') as raw, gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=0) as zipped:
        with tarfile.open(fileobj=zipped, mode='w', format=tarfile.USTAR_FORMAT) as tar:
            for name, content in sorted(files.items()):
                entry = tarfile.TarInfo(name)
                entry.size = len(content)
                entry.mtime = epoch
                entry.mode = 0o755 if name.startswith('bin/') else 0o644
                entry.uid = entry.gid = 0
                tar.addfile(entry, io.BytesIO(content))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--out', type=Path, required=True)
    args = p.parse_args()
    if platform.system() != 'Linux' or platform.machine() != 'x86_64':
        p.error('reproducibility is currently verified only for Linux x86_64')
    if command(['git', 'status', '--porcelain', '--untracked-files=normal']).strip():
        p.error('release builds require a clean committed source tree')
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    revision = command(['git', 'rev-parse', 'HEAD']).decode().strip()
    epoch = int(command(['git', 'show', '-s', '--format=%ct', 'HEAD']))
    package_version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
    tag = os.environ.get('GITHUB_REF', '')
    if tag.startswith('refs/tags/') and tag != 'refs/tags/v' + package_version:
        p.error('release tag must match the Cargo workspace version')
    version = command(['rustc', '-Vv']).decode()
    if not version.startswith('rustc 1.99.1 '):
        p.error('release requires pinned Rust 1.99.1')
    # Fetch checksum-verified lockfile dependencies once, then forbid network.
    subprocess.run(['cargo', 'fetch', '--locked', '--target', TARGET], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix='c-lib-release-') as directory:
        builds = []
        for index in range(2):
            target = Path(directory) / str(index)
            env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_INCREMENTAL='0',
                       SOURCE_DATE_EPOCH=str(epoch), RUSTFLAGS=f'--remap-path-prefix={ROOT}=/source --remap-path-prefix={target}=/target')
            # Avoid inherited encoded flags silently overriding the pinned flags.
            env.pop('CARGO_ENCODED_RUSTFLAGS', None)
            subprocess.run(['cargo', 'build', '--offline', '--locked', '--release', '--workspace', '--target', TARGET], cwd=ROOT, env=env, check=True)
            # cargo test can unify dev-dependency features and replace normal
            # release binaries. Freeze the ordinary build before running tests.
            snapshot = Path(directory) / f'binaries-{index}'
            snapshot.mkdir()
            for name in BINARIES:
                shutil.copyfile(target / TARGET / 'release' / name, snapshot / name)
            builds.append(snapshot)
            if index == 0:
                subprocess.run(['cargo', 'test', '--offline', '--locked', '--release', '--workspace', '--target', TARGET], cwd=ROOT, env=env, check=True)
                subprocess.run(['cargo', 'doc', '--offline', '--locked', '--workspace', '--no-deps', '--target', TARGET], cwd=ROOT, env=dict(env, RUSTDOCFLAGS='-D warnings'), check=True)
                docdir = target / TARGET / 'doc'
        hashes = {}
        for name in BINARIES:
            if digest(builds[0] / name) != digest(builds[1] / name):
                raise RuntimeError('non-reproducible binary: ' + name)
            hashes[name] = digest(builds[0] / name)
        library = ctypes.CDLL(str(builds[0] / 'libcodec_ffi.so'))
        library.codec_get_capabilities.restype = ctypes.c_uint64
        library.codec_get_abi_version.restype = ctypes.c_uint32
        abi = library.codec_get_abi_version()
        metadata = {'schema': 1, 'version': package_version, 'abi': f'{abi >> 16}.{abi & 65535}', 'capabilities': library.codec_get_capabilities(),
                    'scope': 'experimental infrastructure; complete AV1 decoding unavailable',
                    'revision': revision, 'source_tree': command(['git', 'rev-parse', 'HEAD^{tree}']).decode().strip(),
                    'os': platform.platform(), 'target': TARGET, 'source_date_epoch': epoch,
                    'rustc': version, 'cargo': command(['cargo', '-V']).decode().strip(),
                    'linker': command(['cc', '--version']).decode().splitlines()[0],
                    'cargo_lock_sha256': digest(ROOT / 'Cargo.lock'), 'binaries': hashes,
                    'reproduction': 'two clean offline builds, byte-identical binaries in one toolchain/OS environment'}
        files = {'bin/' + name: (builds[0] / name).read_bytes() for name in BINARIES}
        files.update({'include/codec.h': (ROOT / 'include/codec.h').read_bytes(),
                      'build-manifest.json': (json.dumps(metadata, indent=2) + '\n').encode(),
                      'source.tar': command(['git', 'archive', '--format=tar', 'HEAD'])})
        for directory_name in ('docs', 'examples'):
            files.update({str(path.relative_to(ROOT)): path.read_bytes() for path in (ROOT / directory_name).rglob('*') if path.is_file()})
        # Rustdoc is generated and tested, but not part of the binary reproducibility comparison.
        for path in docdir.rglob('*'):
            if path.is_file():
                files['rustdoc/' + str(path.relative_to(docdir))] = path.read_bytes()
        bundle = out / f'c-lib-{package_version}-{TARGET}.tar.gz'
        archive(files, bundle, epoch)
        checksums = out / 'SHA256SUMS'
        checksums.write_text(digest(bundle) + '  ' + bundle.name + '\n')
        print(str(bundle))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
