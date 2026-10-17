# Milestone 2: verified subset, not milestone completion

## Repository inspection

Inspected `main` at `4e55e7dd4a418a95c1edf354bc550fae32042230`, merged
PR #1, and `milestone-2-av1-decoder-11522168481306319431` at
`7532b71180b9ba3ba4d8ea9a55bfa38a939040a6`. The branch and merged tree matched.
PR #1 contained FFI/cbindgen work and a report claiming frame/entropy work;
no `frame/`, `entropy.rs`, DecoderState or stateful AV1 validator existed there.
There was no implementation to recover from the available branch.

Milestone 1's MP4 demuxer, bitstream/sequence validation, timestamp/memory checks,
CLI sample iteration and C ABI tests were retained. The existing tests pass
within the expanded suite. Structural sample acceptance was never evidence
of a decoded header or reconstructed image; existing APIs retain that scope.

## Phase 2.1 — implemented subset

- Full uncompressed header syntax for key/intra-only frames, including reduced
  still pictures, show/display flags, frame IDs, order hints, dimensions and
  super-resolution, render dimensions, intrabc and normative intra inference.
- Uniform/nonuniform tile layout, context tile ID, tile sizes, exact ranges,
  split groups and ordered completion. Entropy payloads are deliberately opaque.
- Quantizer deltas, matrices, intra segmentation and lossless inference, delta-Q
  and delta-LF, loop filter deltas, CDEF, loop restoration and transform modes.
- Intra global motion is normatively identity (no syntax bits); inter motion is
  unsupported. Film grain disabled/apply=0 branches work; apply=1 is unsupported.
- Persistent sequence state; eight explicitly metadata-only references; frame-ID
  window and order-hint invalidation; frame-ID continuity checks; reference
  invalidation at header acceptance and refresh only after tile-group completion.
  No reconstructed reference pixels, motion fields or CDF banks are fabricated.
- `ParserState`, pending header, exact redundant-header bit matching, temporal
  delimiters, reset, sequence changes and atomic whole-sample state commits.
- Sequence-header, frame-header, combined-frame, redundant-header, tile-group,
  delimiter, padding and reserved-type handling. All payloads are bounded.
- Strict CLI `--frame-headers` diagnostics. Default CLI and `validate_sample`
  remain structural-only, preserving Milestone 1 behavior.

Unsupported paths explicitly return `Unsupported`: inter/switch/show-existing
frames, applied film grain, decoder-model timing, multiple/filtered operating
points, non-base layers, metadata payload semantics and tile-list OBUs. No claim
of general interframe or whole-stream conformance is made. Eight metadata slots
are not a decoded picture buffer. Callers must check for pending tiles at EOF;
separate API calls can intentionally split a frame across samples.

Small related syntax modules are combined in `quantization.rs` and `filters.rs`.
No empty global-motion/film-grain module scaffolding was created.

## Phase 2.2 — verified arithmetic primitive

Implemented the normative 15-bit initial value, range scaling (EC_PROB_SHIFT=6,
EC_MIN_PROB=4), renormalization, equal-probability Boolean and literal reads,
2–16-symbol ascending CDFs, adaptation count/rates, disable-update behavior,
limited implicit zero padding, tile-end marker and zero-tail validation.
Symbol/CDF mutation is atomic on error. Invalid CDF intervals and exhausted
padding return errors. Signed fixed-width, ns(), UVLC and raw subexponential
bitstream operations are available without substituting generic Exp-Golomb.

Thirty independent libaom-generated/decoded vectors verify 2,880 symbols and
final CDF states. The tests also verify both adaptation thresholds, disabled
updates, Boolean/literal equivalence, tile termination, invalid CDFs and all
byte-prefix truncations. See `tests/oracles/README.md` for the pinned oracle,
reproduction tool and trace provenance.

Still missing: codec default CDF banks, context selection, saved/frame-end CDF
propagation, block/partition/coefficient syntax, prediction, transforms and image
reconstruction. A generic symbol decoder alone is not complete Phase 2.2.

## Verification

Linux x86_64, Rust 1.99.0 stable, GCC 13.3.0. All five requested gates pass:

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Pass |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | Pass |
| `cargo test --locked --workspace` | 47 passed, 0 failed/ignored |
| `cargo build --locked --workspace --release` | Pass |
| `cargo test --locked --workspace --release` | 47 passed, 0 failed/ignored |

Gate logs are in `verification/milestone2/`. C ABI link/execution and deterministic
cbindgen comparison pass in both profiles. The header is now generated into
memory, not into a tracked directory. C ABI declarations did not change.

The provided MP4 fixtures are tested: both keyframes match external header
traces, and all 12 multi-frame samples pass structural validation. Its 11
interframes are explicitly rejected by strict validation with state unchanged.
Additional actual encoded fixtures cover 4 tiles, reduced still-picture lossless
and 10-bit input. Specification-directed tests cover unusual conditional syntax,
bit consumption, truncation, invalid tile transitions and state rollback.

AddressSanitizer fuzzing uses nightly Rust 1.101.0-nightly, cargo-fuzz 0.13.2,
and `ASAN_OPTIONS=detect_leaks=0`. LeakSanitizer cannot operate under this
container's ptrace restrictions; the initial attempt failed at shutdown for this
reason (recorded separately). No leak-sanitizer pass is claimed. ASan remains on.
Final fixed-run smoke results: AV1 10,000 executions; MP4 100,000 executions,
both exit status zero without ASan findings. Results are in the adjacent fuzz logs. An earlier
AV1 run completed 158,725 executions in 31 seconds without an ASan finding.
The expanded AV1 target exercises stateful rollback and symbol decoding as well
as the original structural parser. These smoke runs are not exhaustive proofs.

GitHub Actions cross-platform results must be read from the PR checks; local
Linux results do not imply Windows or macOS passed.

## Recommended next objective

Implement and independently trace inter/show-existing headers and reference
selection first, then add normative codec CDF banks and intra block/partition
syntax. The first pixel-reconstruction target should be a deliberately bounded
single-tile intra stream, compared plane-for-plane with an external oracle.
Do not label that target complete until actual reconstructed pixels agree.
