# Verification results — 2026-10-13 (India)

## Actual execution

Linux x86_64, Rust 1.99.0 stable, GCC 13.3.0. Fuzzing used Rust
1.101.0-nightly and cargo-fuzz 0.13.2.

| Gate | Actual result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo fmt --manifest-path fuzz/Cargo.toml -- --check` | Passed |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | Passed |
| `cargo test --locked --workspace` | 31 passed; zero failed/ignored |
| `cargo build --locked --workspace --release` | Passed |
| `cargo test --locked --workspace --release` | 31 passed; zero failed/ignored |
| C11 strict compilation, Rust shared-library linking, execution | Passed in debug and release |
| C ABI cbindgen consistency | Passed in debug and release |
| Deliberately nonexistent `CC` | Failed as required, with explicit missing-compiler error |
| Original real AV1 MP4 | All 1 samples parsed; sequence dimensions verified |
| Generated multi-frame AV1 MP4 | All 12 samples parsed; sequence dimensions verified |
| CLI sample-10 corruption | Nonzero exit, sample number reported |
| MP4 AddressSanitizer fuzz smoke | 556,428 runs in 31 seconds; no crash |
| AV1 AddressSanitizer fuzz smoke | 725,270 runs in 31 seconds; no crash |
| Windows C ABI linking and tests | Pending external CI verification |
| macOS C ABI linking and tests | Pending external CI verification |

Logs for the executed test/build/Clippy/fuzz commands accompany this report.
Formatting commands exited zero without output. The missing-compiler log is an
intentional negative test, not a passing test-suite log.

## Fuzz limitation

The initial MP4 run completed 1,144,774 inputs, then LeakSanitizer failed during
process inspection because this environment restricts `/proc`/ptrace access.
Final fuzz runs used `ASAN_OPTIONS=detect_leaks=0`; AddressSanitizer remained
active. Leak checking was therefore **not verified** locally. CI keeps default
sanitizer settings. Each final target used `-max_total_time=30 -max_len=65536`.
These are smoke tests, not exhaustive fuzzing or conformance certification.

During incremental builds, two linker failures involving missing generated or
Rust test-harness symbols cleared after cleaning the affected package. Final
recorded debug/release tests passed without source workarounds or disabling LTO.

## Regression coverage

- MP4: required/duplicate/truncated tables, huge entry counts, malformed sample
  descriptions and mdhd, zero timescale, unsigned/signed ctts, ctts version and
  count checks, stsc order/ranges/description indices/exact coverage, timing
  coverage, stss ordering, out-of-mdat and oversized co64 offsets, empty ctts in
  different box orders, trailing malformed bytes, repeated parsing, EOF, and
  deterministic truncation/byte mutation.
- AV1: RGB 12-bit profile 2, monochrome 12-bit profile 2, unsupported profiles,
  inconsistent reduced headers, reserved chroma/level/header bits, duplicate
  operating points, invalid timing values, UVLC maximum, bounded/truncated OBUs,
  reserved OBU types, missing size fields, and sequence trailing-bit validation.
- Core/memory: invalid bases, wide exact rescaling, overflow/negative results,
  exact comparison, aggregate allocation limits, huge/zero dimensions, odd-size
  10-bit planes, and shared reference-counted plane storage.
- ABI: version string, valid create/destroy, unsupported codec, invalid raw ID,
  null output pointer, null destroy, C compiler/linker execution, and missing CC.

## Completion boundary

The requested fixes are implemented and the local gates above pass. Milestone 1
cross-platform sign-off is pending successful Windows and macOS jobs in CI.
Correction after inspecting merged PR #1: its tree did not contain the claimed frame parser, decoder state, or entropy decoder. The earlier Milestone 2 completion claim was inaccurate. See MILESTONE2.md for the subsequently implemented and verified subset.
This project does not yet decode AV1 frame pixels; see README for supported formats, API changes, allocation policy, and remaining limitations.
