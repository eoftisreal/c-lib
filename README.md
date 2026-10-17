# Experimental Rust Codec and Playback Infrastructure

This repository contains AV1 bitstream and frame-header validation, MP4 demuxing,
clock synchronization, a C interface, and experimental codec processing primitives.
The native Rust `decode_intra_frame` entry point decodes constrained AV1
key pictures to real pixels, including lossless 8/10-bit 4:2:0 and 4x4
lossy 8/10/12-bit 4:2:0, 4:2:2 and 4:4:4 subsets. The [intra conformance
workflow](.github/workflows/core-pixel-conformance.yml) verifies 34 pictures
pixel-for-pixel against independent libaom output. Active in-loop filtering
is deliberately unsupported until complete syntax and YUV parity are verified.
See [supported scope](docs/AV1_SUPPORT.md).
**This is not yet a general AV1 video decoder, Vulkan Video backend, or
neural frame-interpolation engine.**

## API, verification and release contracts

See [tested Rust/C API documentation](docs/API.md), the generated
[AV1 profile and feature support matrix](docs/AV1_SUPPORT.md), and
[decoder gates, benchmarks and reproducible artifacts](docs/PRODUCTION.md).
The strict decoded-frame and full-decoder fuzz checks currently fail closed:
a complete compressed-picture backend is unavailable. C ABI 1.0 version and
capability queries report this explicitly. Experimental Linux release bundles
contain tested binaries, generated Rustdoc, C examples, source and checksums.

## Implementation status

The previously missing `codec-dsl` and `codec-neural` crates, core modules,
AV1 module declarations, and fuzz target are now present, allowing workspace
compilation and source-inventory checks. However, these files are **partial
implementations**:

- `frame/inter.rs` parses a limited inter/header prefix; it does not implement
  reference-based pixel reconstruction or complete inter motion compensation.
- `frame/intra.rs` implements basic and smooth prediction; directional, filter-intra
  and screen-content prediction remain incomplete.
- `frame/partition_decode.rs` traverses 64/128 superblocks with partition CDFs
  and immediate leaf callbacks; a restricted lossless compressed leaf decoder is connected.
- `frame/{inverse,transform}.rs` implements staged scalar DCT, ADST/FLIPADST,
  identity, checked WHT and rectangular 2D transforms.
- `vulkan.rs` and `codec-core/hwaccel.rs` validate experimental descriptors;
  they do not submit Vulkan commands or decode a video frame on the GPU.
- `codec-neural` provides scene-cut heuristics and motion-vector conversion,
  not RIFE/GMFSS inference.
- `codec-dsl` provides a small rule compiler and evaluator; this is not a GPU
  shader runtime or a production scheduling DAG.

The architecture table below describes the **intended system** rather than all
features implemented today. Passing `cargo test` establishes that existing
tests pass, **not** full AV1 bitstream conformance or decoded-image correctness.

The CLI `--md5` option computes actual MD5 for each **compressed AV1 sample**
(not reconstructed YUV planes). `--y4m` refuses to produce fabricated output
for the primary MP4 CLI path. Use `av1-decode` for the supported OBU subset. `--benchmark` measures syntax
parsing and validation, not full decoding performance.

## Reconstruction foundations (in progress)

This branch adds reusable **real pixel** data structures and oracle utilities:
`codec-core::PlanarFrame` validates 8/10/12-bit YUV planes and compares actual
samples by plane and coordinate, `codec-cli::y4m` serializes supplied pixel
planes to Y4M or planar raw YUV without writing stride padding, and
`codec-av1::frame::{intra,reconstruction}` provides checked basic intra
prediction and residual addition.

`frame::picture::PictureReconstruction` connects supplied predictions and real
coefficients through inverse transforms to private Y/U/V buffers, rejecting
overlaps and refusing to publish unwritten pixels. The result is pre-filter
reconstruction. See [Phase 1 status](verification/PHASE1_STATUS.md).

`codec_av1::decode_intra_frame` now connects compressed syntax, adaptive CDFs,
coefficient levels, dequantization, inverse WHT and pixel buffers for the
[verified lossless subset](docs/AV1_SUPPORT.md). Run:

```sh
cargo run --release -p codec-cli --bin av1-decode -- \
  --input tests/intra_lossless_core.obu --output picture.yuv
```

The separate [core pixel CI gate](.github/workflows/core-pixel-conformance.yml)
compares fresh native output with pinned libaom output. The original palette fixture now passes independently verified YUV parity;
full AV1 profile acceptance remains unsupported.
The CLI intentionally continues to reject `--y4m` for compressed AV1 input.
See [the milestone plan](verification/AV1_RECONSTRUCTION_ROADMAP.md) for
the required block-syntax, reconstruction, filtering and pixel-exact tests.

## P2 acceptance and robustness (in progress)

The executable [acceptance matrix](verification/conformance.json) pins compressed
fixtures and keeps complete-picture decoding **BLOCKED** until an independent
c-lib decoder exists. The [runner](verification/conformance.py) records hashes,
commands, frame counts and pixel mismatches. New fuzz targets exercise CDFs,
partition/transform/reconstruction and scalar filters. See [P2/P3 status](verification/P2_P3_STATUS.md)
for usage, official-corpus gaps, memory limits and production dependency gates.

## P1 scalar filters and pixel lifecycle (in progress)

The P1 additions provide verified scalar super-resolution, CDEF, Wiener and
SGRPROJ kernels, explicit restoration stripe boundary sources, complete film-grain
parameter parsing, and bounded ownership of actual reference/display pictures.
Required unsupported processing fails before actual reference refresh. Full
inter prediction, frame-wide filter orchestration and grain synthesis remain
incomplete. See [the P1 status and API changes](verification/P1_STATUS.md).

## Reference pixel comparison (independently decoded frames)

The `codec-av1::frame::cfl` module now supports scalar Chroma-from-Luma
prediction for complete reconstructed 4:2:0, 4:2:2 and 4:4:4 blocks.
It is a pixel primitive, **not** a connected AV1 bitstream decoder.

To compare **raw planar YUV frames already decoded** by an external
decoder and a candidate implementation, run:

```sh
cargo run -p codec-cli --bin yuv-compare -- \
  --expected reference.yuv --actual candidate.yuv \
  --width 320 --height 240 --bit-depth 8 --subsampling 420
```

The tool compares **every frame**, checks exact sizes and pixel ranges,
and reports the first difference as frame, plane, x/y, expected and
actual sample. For 10- or 12-bit YUV, use little-endian 16-bit samples.
It does not decode MP4 or AV1 files or prove frame-reconstruction parity
until real frames are supplied from two independent decoders.
The original `codec-cli --y4m` option still refuses to fabricate video.

## Workspace Architecture

| Crate | Responsibility |
| --- | --- |
| `crates/codec-core` | Decoder/packet contracts, timestamp synchronization, bounded planar YUV frame storage and pixel-comparison oracle, SIMD CPU capability detection, wavefront coordination, and experimental Vulkan descriptor types. |
| `crates/codec-bitstream` | Bounds-checked bit-level reading primitives (AV1 `leb128`, `uvlc`, `su(n)`, `ns(n)`, `decode_subexp`) and zero-branch 64-bit register-buffered bit reader (`RegisterBitReader`) with aligned byte lookahead. |
| `crates/codec-memory` | 64-byte cache-aligned, $128\times 128$ superblock-padded plane buffers (`AlignedPlaneBuffer`), zero-copy `FrameStorage` (`HostAligned`, `VulkanImage`, `DmaBuf`), and the normative AV1 Dual-Reference Virtual Buffer Index (VBI) state machine (`DualRefBufferPool` with `DecoderRefCount` and `PlayerRefCount`). |
| `crates/codec-av1` | OBU/sequence and supported intra-header parsing, entropy-symbol primitives, tile framing, filter helpers, basic DC/V/H/Paeth prediction and pixel reconstruction helper; **not yet a complete AV1 picture decoder**. |
| `crates/container-mp4` | Transactional ISOBMFF MP4 demuxer (`demuxer.rs`, `track.rs`), Fragmented MP4 (`fMP4`/CMAF `moof`/`traf`/`trun`) adaptive streaming parser, edit-list (`elst`) presentation timeline composition, $O(\log N)$ keyframe index bisection (`seek_keyframe_bisect`), and multi-track video/audio/subtitle demuxing (`av01`, `mp4a`, `Opus`, `fLaC`, `wvtt`, `tx3g`). |
| `crates/codec-dsl` | Simple declarative rule parser/evaluator; no production media scheduler or GPU runtime. |
| `crates/codec-neural` | Scene-cut heuristic and AV1 motion-vector conversion for possible future inference integration; no inference runtime. |
| `crates/codec-ffi` & `include/codec.h` | Panic-safe (`catch_unwind` -> `CODEC_ERR_INTERNAL_PANIC`) C11 ABI exposing opaque decoder/pipeline handles, zero-copy Vulkan context/frame export (`codec_decoder_set_vulkan_context`, `codec_frame_export_vulkan_image`), DSL compilation (`codec_pipeline_compile_dsl`), and PID sync (`codec_pipeline_step_sync`). |
| `crates/codec-cli` | AV1 syntax-validation CLI, compressed-sample MD5, benchmark, and library Y4M/raw-YUV writers for **already decoded** image planes; `--y4m` still explicitly unsupported without a decoder. |

## Build and Verify

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --workspace --release
cargo test --locked --workspace --release
cargo run -p codec-cli -- --input tests/multi_av1.mp4 --inter-headers --benchmark
```

## Fuzzing and Sanitizers

```sh
rustup toolchain install nightly
cargo install cargo-fuzz --locked
cargo +nightly fuzz run mp4 -- -max_total_time=30 -max_len=65536
cargo +nightly fuzz run av1 -- -max_total_time=30 -max_len=65536
cargo +nightly fuzz run av1_settings -- -max_total_time=30 -max_len=65536
rustup component add rust-src --toolchain nightly
RUSTFLAGS="-Z sanitizer=thread" cargo +nightly test -Zbuild-std --locked --lib --tests -p codec-core -p codec-memory -p codec-av1 --target x86_64-unknown-linux-gnu
```

## License

MIT OR Apache-2.0
