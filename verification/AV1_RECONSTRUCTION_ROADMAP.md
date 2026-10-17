# AV1 Reconstruction Roadmap and Exit Criteria

Status: **incremental implementation plan**, not evidence of a finished decoder.
This file supersedes the feature-completeness assertions in prior exploratory
milestone notes. Green CI demonstrates the existing test suite, **not**
bit-exact AV1 frame reconstruction.

## Current implementation order

1. Palette and the original `intra_lossless.obu`: complete native decoding with
   exact equality for all 18,432 oracle samples. Retain the constrained fixture too.
2. Expand intra modes/block syntax with additional independent lossless fixtures.
   The additional 48x32 palette and 64x48 gradient fixtures now match every sample.
   Multi-tile traversal is connected for the supported lossless subset: three
   192x192 pictures match all 165,888 independent samples across unequal tiles,
   including four separate tile groups and native 10-bit output. This does not
   establish lossy multi-tile, cross-tile filtering, or complete AV1 support.
   Rectangular/sub-8 blocks, CfL, smooth/Paeth and filter-intra are connected;
   oblique directional edge preparation and angle deltas remain unsupported.
3. Add lossy intra reconstruction and integrate all normative reference filtering;
   require independent pixel equality with each enabled filter combination.
4. Add inter decoding, decoded reference management and multi-frame playback;
   require complete display-order pixel comparisons and reference lifecycle tests.
5. Begin SIMD, Vulkan and performance work only after the preceding pixel gates.

The existing filter and reference lifecycle component tests do not satisfy steps
3 or 4: those stages still require compressed-stream integration and pixel oracles.

## Stage 1A — First 8-bit 4:2:0 intra frame

- Retain existing OBU, sequence/header, tile framing, and arithmetic-symbol primitives.
- Implement the actual AV1 block CDF families and compressed tile syntax: partition
  contexts, mode info, transform-tree syntax, coefficient EOB/base/range/sign
  decoding, dequantization tables, and inverse transforms.
- Implement the remaining intra prediction modes (directional angles with edge
  filtering/upsampling, smooth, CfL, palette, IntraBC) when the bitstream path
  requires them. Unsupported features must return an error, not fake pixels.
- Reconstruct blocks in raster order into actual active Y/U/V planes, respecting
  4:2:0 chroma geometry and exact integer rounding.
- Compare **every reconstructed pixel** against a trusted reference decoder
  for one explicitly constrained, single-tile 8-bit sample.

**Gate:** independently encoded constrained sample yields complete frame output
and the exact reference pixel values. Parser-only success is insufficient.

## Stage 1B — Full intra frames and mandatory reference filtering

- Expand to multi-tile, 10/12-bit, 4:2:2, 4:4:4, screen-content,
  lossless, and supported operating-point cases.
- Integrate deblocking, CDEF, super-resolution, and loop restoration with
  precise edge availability and saved strip-boundary input buffers.
- Keep film-grain display synthesis **separate** from the restored reference
  frame that future inter frames must use.
- Verify independent 8/10/12-bit intra streams including
  `tests/intra_10bit.obu`, `tests/intra_lossless.obu` and
  `tests/intra_tiles.obu`.

**Gate:** YUV sample-by-sample equivalence to a trusted decoder across the
supported intra corpus with filtering enabled; no implicit fallback to fake
frames or bypassed normative filters.

## Stage 2 — Repeatable decoded-image oracle

- The initial reusable primitives are `codec_core::picture::{Plane, PlanarFrame,
  first_pixel_mismatch}` and `codec_cli::y4m::{Y4mWriter,write_raw_yuv}`.
- Verify that active image samples, not internal stride padding, are written.
  For 10/12-bit planar output, write **little-endian 16-bit** samples.
- Once a real AV1 decoder can emit frames, connect those validated frames to
  the writer; until then, the CLI's `--y4m` flag **must remain unsupported**.
- Construct external reference pixel buffers using libaom/dav1d and compare the
  first mismatch's plane, coordinate, expected sample and actual sample.
- Keep compact unit tests, malformed inputs, no-panic fuzzing, independent
  codec oracles, and a C ABI test. Pixel equality is not replaced by
  compressed-sample MD5.

**Gate:** each supported decoded sample has exact pixel equivalence to
externally decoded YUV data with mismatch diagnostics and reproducible fixtures.

## Report-based improvements: CfL and independent YUV oracle

This implementation adds a bounded and tested **CfL pixel-domain predictor**
in `codec-av1/src/frame/cfl.rs`, including 4:2:0, 4:2:2 and 4:4:4
reconstructed-luma subsampling to Q3, chroma-grid mean subtraction,
signed AV1 alpha scaling/rounding, and bit-depth clipping. It supports
**complete** 4/8/16/32-sample chroma blocks and rejects unsupported edge
extension instead of inventing samples. It is **not connected** to AV1
entropy-coded CfL mode selection or full image decoding yet.

The independent raw-YUV verification tool allows actual decoder output
to be compared with an external reference decoder:

```sh
cargo run -p codec-cli --bin yuv-compare -- \
  --expected reference.yuv --actual candidate.yuv \
  --width 320 --height 240 --bit-depth 8 --subsampling 420
```

Both inputs must be **raw planar YUV** of the given geometry and depth:
8-bit samples are bytes; 10/12-bit samples are little-endian u16 words.
The tool validates raw frame size and pixel ranges, scans every frame,
and fails at the first difference with frame index, plane, x/y and
expected/actual values. It rejects empty, truncated and unequal streams.
It does **not** decode MP4/AV1 input files. The existing `--y4m`
flag remains unsupported until a real decoder can produce pixels.

The supplied reconstruction report also included a master loop-filter
function with methods that simply returned `Ok(())`, as well as type
names (`Picture`, `FrameState`) absent from this workspace.
Those examples are **architectural pseudocode**, not a production
implementation. They must not be wired into a path that claims to
output conformant AV1 pixels. Full tile reconstruction, transform
coefficients, filter activation metadata and correct stripe boundary
buffers still need to be implemented and oracle-verified.

## Stage 3 — Inter-frame state and prediction

- Build eight decoded **picture** reference slots holding actual reconstructed
  and restored pixels plus the required motion fields, saved CDF context,
  order hints, segmentation and loop filter metadata.
- Implement show-existing-frame, reference sizing, tile context inheritance,
  reference frame IDs, MV derivation and candidate ordering.
- Add subpel motion compensation, warp, OBMC, compound predictions, and
  inter-intra modes incrementally with focused scalar oracle tests.
- Apply refresh_frame_flags **only after** the frame is reconstructed and
  required reference filtering completes.

**Gate:** exact frame-by-frame reference equality across multi-frame MP4/OBU
corpus and correct state after random seeking or corrupted samples.

## Stage 4 — Full conformance

- Expand spec coverage for color profiles, transforms, filtering, superres,
  tile boundaries, film-grain display path and reference-frame behavior.
- Use official and independently encoded AOMedia conformance vectors.
- Do not label individual filter helpers as a full connected normative cascade.

**Gate:** documented pass/fail matrix by AV1 profile and feature, including
pixel-accurate output and zero regressions in fuzz/property tests.

## Stage 5 — Acceleration and playback ecosystem

- SIMD with scalar differential tests first; then tile/wavefront/thread
  scheduling with determinism and sanitizer testing.
- Implement real Vulkan Video decode submissions separately from the existing
  experimental descriptor validation.
- Build the declarative scheduling/runtime and neural inference backends
  only after reference video output and timing correctness.
- Test scalar, SIMD, threading, and hardware against the same oracle frames,
  and preserve stable FFI semantics.

**Gate:** demonstrated output equality, measured throughput, bounded memory
use, race-free operation, and explicit hardware fallback/error handling.

## Current checked-in building blocks (not a full decoder)

- `codec-core/src/picture.rs`: validated planar frame geometry, depth, stride,
  active pixel values, and first-mismatch reporting.
- `codec-cli/src/y4m.rs`: Y4M and raw YUV encoding from supplied pixel planes.
- `codec-av1/src/frame/intra.rs`: DC, vertical, horizontal and Paeth
  prediction for **already available** reconstructed neighbors.
- `codec-av1/src/frame/reconstruction.rs`: prediction-plus-residual
  addition and depth-based clipping.
- The missing critical link is **entropy-coded block parsing, complete inverse
  transforms and the working AV1 decoding state machine**.

Run the existing quality checks:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --workspace --release
```

None of these replaces an external decoder's pixel-level reference comparison.

## Constrained lossy keyframe gate

The first lossy compressed-picture gate now passes with **4x4 coding blocks**,
filters disabled, and actual entropy-coded transform types and coefficients.
The 27-picture native depth/chroma corpus covers 113,664 samples; see
`intra-4x4-conformance.json` and `docs/AV1_SUPPORT.md` for exact limits. All seven
4x4 intra transform types and four quantizer CDF banks also pass 84 independent
libaom coefficient/context vectors. 12-bit, 422 and 444 are connected only for
this constrained 4x4 path. Larger lossy transforms, transform-depth syntax,
quantization matrices and required reference filtering remain the next core
work. The original lossy 10-bit/multi-tile fixtures remain unsupported.
