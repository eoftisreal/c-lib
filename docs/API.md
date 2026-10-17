# Rust and C API contract

Library version: **0.1.0**. C ABI: **1.0** (`0x00010000`).
This is experimental codec infrastructure, not a complete AV1 playback library.
Runtime `codec_get_capabilities()` currently returns **0**. Bit 0 represents a
complete AV1 pixel backend, bit 1 Vulkan Video decode, bit 2 film-grain synthesis.
Handle creation and descriptor validation never imply these capabilities.

## Rust

Generate documentation with `cargo doc --locked --workspace --no-deps` and run
examples with `cargo test --locked --doc --workspace`. The crate-level examples in
`codec-core` and `codec-ffi` are compiled and executed by that test suite. Release
artifacts contain the generated Rustdoc tree. The CI documentation artifact also
contains the source C contract and its tested example.

`codec_core::Plane` owns `Vec<u16>` storage. `width`, `height`, `stride` and active
sample values are validated; `row()` returns active pixels without stride padding.
`PlanarFrame` owns Y/U/V, checks depth (8/10/12) and subsampling dimensions, and
exposes borrowed planes valid for the frame's lifetime. Samples use host-native
`u16` in memory. Raw serialization explicitly uses little endian for 10/12-bit.
No reference to a row may outlive its frame; Rust enforces these borrows.

`PictureReconstruction` assembles **supplied** predictions and coefficients; it
refuses to publish unwritten pixels. It does not parse compressed blocks.
`DecoderState` validates supported syntax and manages supplied reconstructed
pictures. Its methods do not constitute a packet/receive-frame AV1 decoder.
A real submit/receive/flush decoder, timestamped frame output and complete color
metadata contract remain unsupported. `Frame` is an infrastructure trait, not an
AV1 decoder implementation.

`codec_av1::decode_intra_frame(data, &mut state)` now returns an owned
`PlanarFrame` for the [verified lossless subset](AV1_SUPPORT.md). It requires one
complete picture in an OBU packet, checks tile termination and complete pixel
coverage, and updates syntax state only after success. Errors leave existing
syntax/reference state unchanged. It does not automatically commit picture slots
or implement a streaming submit/receive API. The C capability remains zero because
the complete AV1 backend and C decoded-frame retrieval remain unavailable.

```rust
use codec_av1::{decode_intra_frame, DecoderState};
let mut state = DecoderState::default();
let pixels = decode_intra_frame(packet, &mut state)?;
let y = pixels.planes()[0].row(0).unwrap();
```

For raw 8-bit Y/U/V output, build `cargo build --release -p codec-cli --bin av1-decode`
and run `av1-decode --input tests/intra_lossless_core.obu --output picture.yuv`.
Unsupported/malformed decoding returns a nonzero exit status before creating or
truncating the output file. The command handles one OBU picture, not MP4 input.

Rust crates follow Cargo semantic versioning: a breaking pre-1.0 public API change
requires a minor version increment; patch releases preserve existing public
signatures and documented semantics. A 1.0 Rust API is not declared by this
change. Rust types have no stable binary ABI and must not be passed directly to C.
All dependent Rust crates must be rebuilt together when compiler/dependencies change.

## C

Use the checked-in `include/codec.h`, generated with cbindgen **0.29.4**. CI verifies
it matches the Rust declarations, compiles it as strict C11, and links/runs both
`crates/codec-ffi/tests/abi.c` and the published [`api_version.c`](../examples/api_version.c)
example. Example for a Linux release bundle:

```sh
cc -std=c11 -Wall -Wextra -Werror -Iinclude examples/api_version.c \
  -Lbin -lcodec_ffi -Wl,-rpath,'$ORIGIN/bin' -o api-example
./api-example
```

`codec_get_version()` returns a process-lifetime NUL-terminated read-only string;
do not free or modify it. Check `codec_get_abi_version() >> 16` before using any
other ABI symbol. Unknown capability bits must be ignored; required known bits
must be checked explicitly. Version/capability queries require no handle and are
safe to call concurrently.

`codec_decoder_create()` writes a new opaque owned handle on success and clears
the output on failure. Destroy it exactly once. NULL destroy is permitted.
Each decoder or pipeline handle must be used by at most one thread at a time;
the caller provides synchronization. Functions retain neither configuration
pointers nor DSL string pointers after returning. Input strings must be valid
UTF-8 and NUL-terminated. Every non-NULL pointer must be correctly aligned, valid
for its documented read/write range, and live for the call. Panic catching cannot
make arbitrary or dangling C pointers safe.

`codec_decoder_set_vulkan_context()` stores validated numeric handles only; it
creates no Vulkan session, retains no Vulkan ownership and submits no work.
`codec_frame_export_vulkan_image()` copies descriptor data; no public decoded-frame
producer exists. Do not fabricate or dereference `CodecFrame` from C. There is no
CPU decoded-plane retrieval, frame-release, packet submission or playback ABI yet.
`codec_pipeline_compile_dsl()` compiles rules; it does not execute inference/GPU
processing. `codec_pipeline_step_sync()` computes clock-control adjustments.

All error codes and enum numeric values in the header are frozen for ABI major 1.
C enums must be 32 bits (do not use `-fshort-enums`); use native platform calling
conventions and default packing. Descriptor layouts are native target C layouts,
not serialized wire formats. Supported build/test targets are the existing
Linux/macOS/Windows CI hosts; reproducible release artifacts currently cover
Linux x86_64 only. No cross-target handle/descriptor interchange is supported.

## Compatibility enforcement

The upper/lower 16 bits of `codec_get_abi_version()` are major/minor. Additive C
symbols require a minor increment. Removing/changing a symbol, error value,
structure field/type/order, ownership or calling convention requires a major
increment. Existing symbol semantics must remain compatible within a major.
Libraries keep old symbols for supported major versions; a major transition
requires a migration guide. The initial ABI 1 baseline covers the existing C
infrastructure functions plus version/capability queries, not future frame APIs.

`verification/api-contract.json` snapshots normalized C declarations including
complete structures, enums and public aliases. CI checks header drift and compares
against the PR base: removals/changes require a major increment, additions require
minor/major. Compiler-run C assertions cover representative field offsets, enum
widths and runtime version. Semantic changes still require human review; this
snapshot is not a proof of compatibility. Update the Rust query, snapshot, docs
and C assertions together when intentionally versioning an ABI change.
