# Independent oracle provenance

These are test inputs and external verification tools, not runtime dependencies.
No FFmpeg, dav1d or libaom library is linked by the Rust workspace.

## Entropy vectors

`entropy_vectors.txt` was generated and decoded independently using libaom
commit `b57fd4438bf9db9c48b6c9743e7bcdc53ad9aac8`, fetched from
https://aomedia.googlesource.com/aom on 2026-10-13.

The checked-in `entropy_vectors.c` calls libaom's real range encoder, range
decoder and `update_cdf`. It asserts that the independent decoder reproduces
every input symbol and the encoder's final CDF. The Rust test compares each
symbol and final CDF, then verifies normative AV1 tile termination. There are
30 vectors, alphabets 2 through 16, both adaptation modes, 96 symbols each.
The count crosses both adaptation thresholds and saturates at 32.

Each line contains: alphabet size, adaptation enabled (0/1), tile bytes in hex,
comma-separated symbols, final ascending CDF including count. Libaom uses inverse
CDFs; the generator explicitly converts them to ascending CDFs for the fixture.

Reproduce with a configured libaom checkout (`AOM_SRC`, `AOM_BUILD`):

```sh
cc -O2 -Wall -Wextra -Werror -I "$AOM_BUILD" -I "$AOM_SRC" \
  tests/oracles/entropy_vectors.c \
  "$AOM_SRC/aom_dsp/entenc.c" "$AOM_SRC/aom_dsp/entdec.c" \
  "$AOM_SRC/aom_dsp/entcode.c" -o /tmp/entropy-oracle
/tmp/entropy-oracle > /tmp/entropy_vectors.txt
cmp tests/oracles/entropy_vectors.txt /tmp/entropy_vectors.txt
```

For this run only these portable source files were compiled, using a minimal
`config/aom_config.h` with the following definitions (no source modifications):
`CONFIG_ACCOUNTING=0`, `CONFIG_BITSTREAM_DEBUG=0`,
`CONFIG_BITSTREAM_DEBUG_WRITE=0`, `CONFIG_REALTIME_ONLY=0`,
`CONFIG_AV1_HIGHBITDEPTH=1`, `INLINE=inline`, `HAVE_UNISTD_H=1`,
`ARCH_X86_64=1`. The generator was rebuilt with warnings treated as errors and
produced byte-identical output twice.

## Header traces and encoded fixtures

Header expectations were checked using FFmpeg's `trace_headers` bitstream filter:

```sh
ffmpeg -i tests/multi_av1.mp4 -c copy -bsf:v trace_headers -f null -
```

The trace offsets include the OBU header and length. The Rust `header_bits`
excludes these and excludes alignment. Independently observed values:

| Input | Header bits | Base Q | Other checked values |
| --- | ---: | ---: | --- |
| real_av1.mp4 | 37 | 128 | 320x240; intrabc; tile bytes 23..820 in sample |
| multi_av1.mp4, sample 0 | 97 | 140 | loop levels 9,9,14,13; CDEF damping 5; tile bytes 29..1740 |
| intra_tiles.obu | 99 | 140 | 2x2 tiles, two-byte tile sizes |
| intra_10bit.obu | 88 | 128 | 10 bit; restoration enabled, types 0,0,0 |

Additional synthetic syntax tests exercise branches not present in those files:
nonuniform tiles, super-resolution, render dimensions, signed quantizer/filter
deltas, segmentation, CDEF secondary-strength remapping, chroma restoration units,
reference-ID wraparound, redundant headers and split tile groups. These are
specification-directed tests, not independently encoded end-to-end videos.

Additional fixtures were generated with the installed FFmpeg/libaom encoder:

```sh
ffmpeg -f lavfi -i testsrc2=size=128x96:rate=1 -frames:v 1 \
  -c:v libaom-av1 -cpu-used 8 -still-picture 1 -crf 0 -b:v 0 \
  -f obu tests/intra_lossless.obu
ffmpeg -f lavfi -i testsrc2=size=256x128:rate=1 -frames:v 1 \
  -c:v libaom-av1 -cpu-used 6 -tiles 2x2 -crf 35 \
  -f obu tests/intra_tiles.obu
ffmpeg -f lavfi -i testsrc2=size=160x96:rate=1 -frames:v 1 \
  -pix_fmt yuv420p10le -c:v libaom-av1 -cpu-used 4 \
  -enable-restoration 1 -crf 32 -f obu tests/intra_10bit.obu
```

Reference: https://aomediacodec.github.io/av1-spec/ — sections 5.9 (frame syntax),
5.11 (tile groups), and 8.2 (symbol decoding). Passing header/symbol tests is not
proof of reconstructed pixel correctness. No pixel reconstruction exists here.

## Phase 1 scalar oracle

`phase1_vectors.c` uses unmodified libaom commit
`394f9f7df624e4268fd8c894ead9f059d279e480` from
https://aomedia.googlesource.com/aom. Configure/build that source separately:

```sh
cmake -S "$AOM_SRC" -B "$AOM_BUILD" -DAOM_TARGET_CPU=generic \
  -DENABLE_DOCS=OFF -DENABLE_TESTS=OFF -DENABLE_TOOLS=OFF -DENABLE_EXAMPLES=OFF
cmake --build "$AOM_BUILD" -j 4
cc -O2 -Wall -Wextra -Werror -I "$AOM_BUILD" -I "$AOM_SRC" \
  tests/oracles/phase1_vectors.c "$AOM_BUILD/libaom.a" -lm -lpthread -o /tmp/phase1-oracle
/tmp/phase1-oracle > /tmp/phase1_vectors.txt
cmp tests/oracles/phase1_vectors.txt /tmp/phase1_vectors.txt
cargo test --locked -p codec-av1 --test phase1_oracles
```

The 6,799 records comprise T=2,980 1D transforms, D=2,385 2D pixel outputs,
Q=768 quantizer pairs, W=192 WHT residuals, S=171 smooth predictors,
C=159 complete scans and P=144 partition streams. D/C use normative transform
size/type enumeration values. T uses DCT=0, ADST=1, identity=2; S uses
smooth=0, smooth-V=1, smooth-H=2. P edge=0/1/2/3 means full/bottom/right/both,
with both adaptation settings and a trailing 0xa5 literal and termination check.

The generator converts libaom's transposed coefficient layout to/from specification
raster order. Spatial pixel output is not transposed. Partition probabilities come
from libaom's initialized frame context and its own edge-gather functions.

Separately verify all 6,760 specification numeric table entries with:

```sh
python3 verification/phase1_tables.py "$AV1_SPEC_CHECKOUT"
```

The specification checkout must be at pinned commit
`5e04f3f75e73a5898d7616c47c52f032144b8f80`. Optional `--write` regenerates the
numeric tables; run `cargo fmt --all` afterward. These are component comparisons,
not complete bitstream-to-picture conformance tests.

## P1 filtering and grain metadata

`p1_vectors.c` uses the same pinned libaom revision as the Phase 1 scalar oracle.
Its 5,568 deterministic records cover S=1,632 super-resolution rows, C=3,072 CDEF
blocks (also direction/variance), W=288 Wiener outputs and G=576 SGRPROJ outputs.
It calls libaom's real frame-row upscaler, CDEF kernels/direction search, Wiener
convolution and self-guided restoration. Restoration input halos are prepared
from distinct pre-/post-CDEF planes according to the normative stripe sampler.

```sh
cc -O2 -Wall -Wextra -Werror -I "$AOM_BUILD" -I "$AOM_SRC" \
  tests/oracles/p1_vectors.c "$AOM_BUILD/libaom.a" -lm -lpthread -o /tmp/p1-oracle
/tmp/p1-oracle > /tmp/p1_vectors.txt
cmp tests/oracles/p1_vectors.txt /tmp/p1_vectors.txt
cargo test --locked -p codec-av1 --test p1_oracles --test p1_lifecycle
```

S records contain depth/downscaled/upscaled widths, source and output samples.
C records contain depth/direction/pixel-domain primary and secondary strengths/
base damping/origin/LCG seed, output samples and reference direction/variance.
W records contain depth/source mode/seed/horizontal and vertical coefficients and
output samples. G records contain depth/source mode/seed/parameter set/XQD values
and output samples. Source modes 0/1/2 are full frame, interior restoration stripe
and picture-edge restoration stripe. Seeds describe the checked-in generator's
wrapping 32-bit LCG input generation, not a replacement reference filter.

`tests/intra_film_grain.obu` is an independently encoded 64x64 intra frame using
libaom's built-in grain test vector 1:

```sh
ffmpeg -f lavfi -i testsrc2=size=64x64:rate=1 -frames:v 1 \
  -pix_fmt yuv420p -f yuv4mpegpipe /tmp/grain-input.y4m
"$AOM_BUILD/aomenc" --codec=av1 --width=64 --height=64 --fps=1/1 --limit=1 \
  --passes=1 --cpu-used=8 --end-usage=q --cq-level=32 --film-grain-test=1 \
  --ivf -o /tmp/grain.ivf /tmp/grain-input.y4m
ffmpeg -i /tmp/grain.ivf -c copy -bsf:v trace_headers -f null -
```

The OBU fixture is the first IVF frame payload (after the 32-byte IVF and 12-byte
frame headers). `grain_header_values.txt` records every signaled grain field from
that independent FFmpeg trace; `p1_lifecycle` compares all fields and the exact
963-bit uncompressed header. This checks grain *syntax*, not grain synthesis.

## Complete lossless core picture and coefficient regressions

`intra_lossless_core.obu` is a constrained re-encoding of the **same** 128x96
8-bit 4:2:0 picture in the original `tests/intra_lossless.obu`. The original
fixture is unchanged: it selects palette prediction and now also passes the
complete sample-by-sample Rust regression using the same raw oracle.
The core fixture uses DC prediction and nonzero DC/AC lossless residuals.
This is one scoped complete-picture regression, not official conformance coverage.

Reference tools use unmodified libaom commit
`394f9f7df624e4268fd8c894ead9f059d279e480` from
https://aomedia.googlesource.com/aom. Configure/build `aomenc` and `aomdec` with
`cmake -S "$AOM_SRC" -B "$AOM_BUILD" -DAOM_TARGET_CPU=generic -DENABLE_TESTS=0`.
Neither library nor executables are linked or invoked by the native Rust decoder.

Reproduce the fixture and oracle (run from the repository root):

```sh
"$AOM_BUILD/aomdec" --rawvideo --i420 --output-bit-depth=8 \
  -o /tmp/core-source.yuv tests/intra_lossless.obu
"$AOM_BUILD/aomenc" --obu --width=128 --height=96 --limit=1 --fps=1/1 \
  --threads=1 --cpu-used=0 --lossless=1 --enable-cdef=0 --enable-restoration=0 \
  --enable-filter-intra=0 --enable-intra-edge-filter=0 --enable-cfl-intra=0 \
  --enable-palette=0 --enable-intrabc=0 --sb-size=64 --enable-tpl-model=0 \
  --deltaq-mode=0 --enable-directional-intra=0 --enable-smooth-intra=0 \
  --enable-paeth-intra=0 --min-partition-size=8 --max-partition-size=64 \
  --enable-rect-partitions=0 --enable-ab-partitions=0 --enable-1to4-partitions=0 \
  -o /tmp/intra_lossless_core.obu /tmp/core-source.yuv
cmp tests/intra_lossless_core.obu /tmp/intra_lossless_core.obu
"$AOM_BUILD/aomdec" --rawvideo --i420 --output-bit-depth=8 \
  -o /tmp/core-reference.yuv /tmp/intra_lossless_core.obu
cmp tests/oracles/intra_lossless_core.yuv /tmp/core-reference.yuv
cmp /tmp/core-source.yuv /tmp/core-reference.yuv
```

The original picture was also independently decoded with FFmpeg's `libdav1d`
decoder; its raw samples matched libaom. SHA-256:

- Compressed core fixture: `84a795e9dd20384d5d9cbe6a2405c72c056cb1ffdca8bf342c6efabdce88080b`.
- Raw reference (18,432 bytes, Y then U then V): `56e52afa30e34107ac278d2cf3e53a4ca3b7e05d547b14df39719905582ea530`.

The Rust test checks every sample and reports frame, plane, x and y on the first
mismatch. The separate core CI gate decodes both compressed inputs afresh, checks
exact frame byte count and compares each plane using `yuv-compare`; its report
records input/output/executable hashes and build identities. Broad profile
acceptance remains a separate blocked gate.

The original fixture additionally exercises palette colors and diagonal index
maps, rectangular/sub-8 blocks, CfL and filter-intra. Its complete 18,432-byte
output must equal `intra_lossless_core.yuv`; decoding success alone is insufficient.
Both streams are covered by truncation and compressed-byte mutation rollback tests.

### Additional independent lossless pictures

`intra_lossless_palette.obu` (48x32) and `intra_lossless_gradient.obu` (64x48)
were encoded and independently decoded using FFmpeg 9.0.2 essentials (Gyan Windows
build), libaom `3.15.0-25-g023743a7d7`. The FFmpeg executable SHA-256 was
`3256173f3f8bffd7df12227c68adf68025edb1832273a9530688a7bb1ed8edec`.
The tools are verification dependencies only and are not checked in or linked.

Reproduce with that FFmpeg build from the repository root:

```sh
ffmpeg -f lavfi -i 'nullsrc=s=48x32,geq=lum=40+80*mod(floor(X/5)+floor(Y/7)\,3):cb=70+50*mod(floor(X/3)+floor(Y/4)\,3):cr=180-40*mod(floor(X/4)+floor(Y/3)\,3)' \
  -frames:v 1 -pix_fmt yuv420p -c:v libaom-av1 -cpu-used 0 -threads 1 \
  -still-picture 1 -crf 0 -b:v 0 \
  -aom-params 'enable-directional-intra=0:enable-intrabc=0:sb-size=64:deltaq-mode=0' \
  -f obu /tmp/intra_lossless_palette.obu
ffmpeg -f lavfi -i 'nullsrc=s=64x48,geq=lum=16+2*X+Y:cb=80+X+Y:cr=160-X+Y' \
  -frames:v 1 -pix_fmt yuv420p -c:v libaom-av1 -cpu-used 0 -threads 1 \
  -still-picture 1 -crf 0 -b:v 0 \
  -aom-params 'enable-directional-intra=0:enable-intrabc=0:sb-size=64:deltaq-mode=0' \
  -f obu /tmp/intra_lossless_gradient.obu
for name in palette gradient; do
  cmp tests/intra_lossless_$name.obu /tmp/intra_lossless_$name.obu
  ffmpeg -c:v libaom-av1 -i /tmp/intra_lossless_$name.obu -frames:v 1 \
    -pix_fmt yuv420p -f rawvideo /tmp/intra_lossless_$name.yuv
  cmp tests/oracles/intra_lossless_$name.yuv /tmp/intra_lossless_$name.yuv
done
```

The palette case exercises luma/chroma palettes, cache reuse and diagonal index
contexts. The gradient exercises Paeth, CfL and filter-intra and detects incorrect
missing-edge defaults at the upper-left pixel. Directional prediction is disabled
in these two encodings; the original fixture also covers zero-delta V/H and smooth.
The Rust tests compare every active sample, including both chroma planes.

SHA-256 values for the new compressed streams and raw oracles are pinned in
`verification/lossless-fixtures.json`; the live independent-decoder CI gate also
includes all four lossless streams in `verification/core-conformance.json`.
`verification/lossless-pixel-report.json` records fresh native CLI versus libaom
comparisons of all four pictures (43,776 samples), including executable hashes,
commands, output hashes and exact-length checks. It is scoped evidence, not the
broad acceptance report. All 818 newly added palette/filter-intra numeric table
entries were compared with the pinned specification used by `phase1_tables.py`.

`coeff_vectors.c` uses the same pinned libaom encoder and default CDF banks to
encode four focused 4x4 coefficient streams: EOB level 3, non-EOB level 3 with a
negative AC coefficient, negative level 15, and positive level 16. They cover both
base-range thresholds, signs and Golomb extensions. An independently encoded
`0xa5` sentinel checks that coefficient decoding consumes exactly the expected
symbols, followed by normative tile termination.

```sh
cc -O2 -Wall -Wextra -Werror -I "$AOM_BUILD" -I "$AOM_SRC" \
  tests/oracles/coeff_vectors.c "$AOM_BUILD/libaom.a" -lm -lpthread \
  -o /tmp/coeff-oracle
/tmp/coeff-oracle > /tmp/coeff_vectors.txt
cmp tests/oracles/coeff_vectors.txt /tmp/coeff_vectors.txt
```

### Independent multi-tile lossless corpus

`intra_lossless_tiles8.obu`, `intra_lossless_tiles10.obu` and
`intra_lossless_tiles8_palette.obu` were independently encoded and decoded with
unmodified libaom at `394f9f7df624e4268fd8c894ead9f059d279e480`.
The source image is deterministic native YUV420; the generator verifies that
reference decoding preserves every input sample. No Rust decoder output is used
as a reference. Each 192x192 picture has four tiles with 128/64-pixel extents in
both axes. The first packs all tiles into a combined Frame OBU; the others use
four TileGroup OBUs. The palette-named encoding enables palette selection; its
name does not establish exhaustive palette syntax coverage.

Reproduce using the pinned libaom build (enable both encoder and decoder):

```sh
python3 verification/make_tiled_lossless.py --aom-build /tmp/aom-build \
  --output-dir /tmp/tiled-reproduction
```

Compare each generated `tests/*.obu` and `tests/oracles/*.yuv` with its checked-in
counterpart. Compressed/oracle SHA-256 values are pinned in
`verification/lossless-fixtures.json`. Rust tests compare all 165,888 new samples,
including non-8-bit-equivalent 10-bit chroma values; they also exercise incomplete
and reordered groups, late entropy errors and recovery without state mutation.
The live core CI gate decodes these same streams with independent libaom, retaining
native output depth rather than forcing the 10-bit picture through 8-bit output.
This extends the verified lossless subset and does not pass the separate lossy
`intra_tiles.obu` acceptance target or establish cross-tile loop filtering.

### First constrained lossy key-picture gate and native depth/chroma matrix

`verification/intra-4x4-conformance.json` pins 27 independently encoded pictures
and their raw Y/U/V reference outputs. They contain 113,664 active samples.
Eight 64x64 lossy pictures cover four quantizer CDF banks with reduced/full intra
transform sets; eighteen 32x32 lossless/lossy pictures cover all combinations of
8/10/12-bit with 420/422/444; a 128x128 lossy picture uses four tile groups.
All use 4x4 coding blocks and disable in-loop filters and directional modes.
The depth/chroma matrix permits CfL, smooth and Paeth mode selection. It is not
an exhaustive feature corpus. The independent generator checks native byte
lengths, lossless source equality and that lossy output actually differs from
its source. High-bit-depth output remains native little-endian u16.

Reproduce with unmodified libaom at the existing pinned revision:

```sh
python3 verification/make_intra_4x4.py --aom-build /tmp/aom-build \
  --output-dir /tmp/intra-4x4-reproduction
```

The pinned aomenc CLI rejects raw 12-bit 422/444 inputs during subsampling setup.
Only those four cases use the installed FFmpeg/libaom encoder adapter. The
manifest records its exact version/build configuration and executable SHA-256;
all reference outputs are decoded by the pinned unmodified aomdec. Reproducing
compressed bytes requires matching encoder builds; the CI acceptance gate uses
checked-in compressed streams and fresh pinned libaom reference decoding.
No color conversion or c-lib-generated output is used as reference data.

`lossy_coeff_vectors.c` uses the pinned libaom's native scans and coefficient
context helpers, CDF banks, arithmetic encoder and CDF adaptation. It emits 84
vectors: seven intra transform types, four quantizer banks, and three sparse /
level-3 / Golomb-extension coefficient patterns. Expected coefficients are
transposed from libaom's column-major representation into specification raster
order. Rust compares all coefficients, reads the independent 0xa5 sentinel and
checks normative entropy termination. Transform-type symbols themselves are
exercised by the complete-picture corpus, rather than these coefficient vectors.

```sh
cc -O2 -Wall -Wextra -Werror -I "$AOM_BUILD" -I "$AOM_SRC" \
  tests/oracles/lossy_coeff_vectors.c "$AOM_BUILD/libaom.a" -lm -lpthread \
  -o /tmp/lossy-coeff-oracle
/tmp/lossy-coeff-oracle > /tmp/lossy_coeff_vectors.txt
cmp tests/oracles/lossy_coeff_vectors.txt /tmp/lossy_coeff_vectors.txt
```

This is a first independently verified lossy keyframe subset. It does not
implement larger-block transform trees, normative filtering, inter decoding,
full-profile conformance, MP4 playback, Vulkan Video or SIMD acceleration.
