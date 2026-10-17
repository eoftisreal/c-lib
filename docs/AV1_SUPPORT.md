# AV1 support

The native Rust decoder reconstructs a verified, constrained subset of AV1 key
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
passing constrained picture gates above.

| Profile | Name | Complete picture decoding |
| --- | --- | --- |
| 0 | Main | Unsupported |
| 1 | High | Unsupported |
| 2 | Professional | Unsupported |

| Feature | Implementation | Picture acceptance | Evidence |
| --- | --- | --- | --- |
| intra | partial | BLOCKED | [crates/codec-av1/tests/reconstruction_pipeline.rs](../crates/codec-av1/tests/reconstruction_pipeline.rs) |
| inter | missing | BLOCKED | [crates/codec-av1/src/frame/inter.rs](../crates/codec-av1/src/frame/inter.rs) |
| lossless | partial | BLOCKED | [crates/codec-av1/tests/phase1_oracles.rs](../crates/codec-av1/tests/phase1_oracles.rs) |
| lossy | partial | BLOCKED | [crates/codec-av1/tests/phase1_oracles.rs](../crates/codec-av1/tests/phase1_oracles.rs) |
| 8-bit | partial | BLOCKED | [crates/codec-cli/src/oracle.rs](../crates/codec-cli/src/oracle.rs) |
| 10-bit | partial | BLOCKED | [crates/codec-cli/src/oracle.rs](../crates/codec-cli/src/oracle.rs) |
| 12-bit | partial | BLOCKED | [crates/codec-cli/src/oracle.rs](../crates/codec-cli/src/oracle.rs) |
| multi-tile | partial | BLOCKED | [crates/codec-av1/tests/frame.rs](../crates/codec-av1/tests/frame.rs) |
| deblocking | scalar-kernel-verified | BLOCKED | [crates/codec-av1/src/frame/filters.rs](../crates/codec-av1/src/frame/filters.rs) |
| cdef | scalar-kernel-verified | BLOCKED | [crates/codec-av1/tests/p1_oracles.rs](../crates/codec-av1/tests/p1_oracles.rs) |
| superres | scalar-kernel-verified | BLOCKED | [crates/codec-av1/tests/p1_oracles.rs](../crates/codec-av1/tests/p1_oracles.rs) |
| wiener | scalar-kernel-verified | BLOCKED | [crates/codec-av1/tests/p1_oracles.rs](../crates/codec-av1/tests/p1_oracles.rs) |
| sgrproj | scalar-kernel-verified | BLOCKED | [crates/codec-av1/tests/p1_oracles.rs](../crates/codec-av1/tests/p1_oracles.rs) |
| film-grain | syntax-only | BLOCKED | [crates/codec-av1/src/frame/grain.rs](../crates/codec-av1/src/frame/grain.rs) |
| display-order | partial | BLOCKED | [crates/codec-av1/tests/p1_lifecycle.rs](../crates/codec-av1/tests/p1_lifecycle.rs) |
| mp4 | demux-only | BLOCKED | [crates/container-mp4/src/demuxer.rs](../crates/container-mp4/src/demuxer.rs) |
| rgb | syntax-only | BLOCKED | [crates/codec-av1/src/sequence.rs](../crates/codec-av1/src/sequence.rs) |
| malformed-no-output | decoder-missing | BLOCKED | [fuzz/fuzz_targets/av1.rs](../fuzz/fuzz_targets/av1.rs) |
| official-vectors | corpus-missing | BLOCKED | [verification/P2_P3_STATUS.md](../verification/P2_P3_STATUS.md) |
