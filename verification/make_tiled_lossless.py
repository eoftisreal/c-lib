#!/usr/bin/env python3
"""Reproduce tiled lossless fixtures with independent pinned libaom tools.

Build aomenc and aomdec at the revision documented in tests/oracles/README.md.
No codec implementation from this repository creates reference pixels.
"""
import argparse
from pathlib import Path
import subprocess
import tempfile


def generate(aom_build: Path, output: Path) -> None:
    for depth, groups, palette in [(8, 1, 0), (10, 4, 0), (8, 4, 1)]:
        name = f"intra_lossless_tiles{depth}" + ("_palette" if palette else "")
        source = bytearray()
        for plane in range(3):
            width = height = 192 if plane == 0 else 96
            for y in range(height):
                for x in range(width):
                    value = (32 + ((x // 4) * 11 + (y // 4) * 17) % 192
                             if plane == 0 else 83 + plane * 29)
                    if depth > 8:
                        value = (value << 2) + ((x // 4 + y // 4 + plane) % 4)
                    source.extend(value.to_bytes(1 if depth == 8 else 2, "little"))
        obu = output / "tests" / f"{name}.obu"
        reference = output / "tests" / "oracles" / f"{name}.yuv"
        reference.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="c-lib-tile-source-") as temporary:
            raw = Path(temporary) / "source.yuv"
            raw.write_bytes(source)
            subprocess.run([
                str(aom_build / "aomenc"), "--obu", "--width=192", "--height=192",
                "--fps=1/1", "--limit=1", "--cpu-used=0", "--lossless=1",
                "--sb-size=64", "--tile-columns=1", "--tile-rows=1",
                f"--num-tile-groups={groups}", f"--bit-depth={depth}",
                f"--input-bit-depth={depth}", "--enable-directional-intra=0",
                f"--enable-palette={palette}", "--enable-filter-intra=0",
                "--enable-cfl-intra=0", "--enable-smooth-intra=0",
                "--enable-paeth-intra=0", "-o", str(obu), str(raw),
            ], check=True)
            subprocess.run([
                str(aom_build / "aomdec"), "--rawvideo", "--i420",
                f"--output-bit-depth={depth}", "-o", str(reference), str(obu),
            ], check=True)
        if reference.read_bytes() != source:
            raise RuntimeError(f"{name}: independent lossless round trip changed pixels")
        print(f"{name}: independent lossless round trip verified")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--aom-build", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    args = parser.parse_args()
    generate(args.aom_build.resolve(), args.output_dir.resolve())
