#!/usr/bin/env python3
"""Generate a deterministic 64x64 10-bit 4:2:0 lossless AV1 test picture.

The image contains nonzero, non-8-bit-equivalent luma/chroma samples.
Input is native little-endian planar YUV, with no container or colorspace conversion.
"""
import argparse
from pathlib import Path
import struct

WIDTH = HEIGHT = 64


def generate(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("wb") as output:
        for _y in range(HEIGHT):
            for x in range(WIDTH):
                output.write(struct.pack("<H", 190 + ((x // 4) % 16) * 39))
        for value in (301, 765):
            for _ in range((WIDTH // 2) * (HEIGHT // 2)):
                output.write(struct.pack("<H", value))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("output", type=Path)
    generate(parser.parse_args().output)
