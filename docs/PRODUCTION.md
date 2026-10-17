# Verification, performance and releases

## Scoped lossless core gate

`Lossless core pixel conformance` runs on pushes and PRs. It builds the independent
libaom decoder at commit `394f9f7df624e4268fd8c894ead9f059d279e480`, decodes
the core OBU with both it and native `av1-decode`, checks exact byte counts and
compares all Y/U/V pixels. `core-conformance.json` declares only scoped core feature
labels; it does not replace `conformance.json` or establish full-profile support.
CI also seeds the `av1_picture` fuzz target with that compressed fixture.

## Strict decoder gates

`Decoder readiness (strict, manual)` is an explicitly dispatched readiness audit.
It does not run on ordinary development pushes/PRs while the decoder is missing.
Its `decoded-frame-reference` check requires actual native decoded pixels for
**every** pinned acceptance case, independent reference output, exact counts,
matching digests and successful Y/U/V comparisons. It rejects stale commits,
missing/duplicate cases, blocked cases and missing executable identities. It
currently **fails** because the independent compressed-picture decoder is missing.
The report is uploaded even on failure. This manual audit is not a passing
development check or evidence of decoder readiness. Production release automation
must require a successful audit for the same source revision before publishing a
production decoder. Do not add `continue-on-error` or treat
blocked exit 2 as passing. Repository branch/release protection is not changed by these workflows.
Once a real decoder exists, enable automatic readiness checks and require them
for decoder changes before declaring production support.

The reviewed command adapter is `verification/decoder-tools.json`. It deliberately
contains null commands until an independent c-lib raw output adapter exists.
Implement that adapter before configuring commands. Do not substitute libaom or
dav1d as the candidate. Reference and candidate revisions, executable hashes,
manifest hashes and pixel digests are recorded. The gate validates the evidence
structure, not the semantic independence of arbitrary code; source review remains
required. The current manifest has deliberate missing-vector coverage gaps.

`docs/AV1_SUPPORT.md` is generated from `verification/features.json` and the pixel
acceptance matrix. Component status and picture conformance are separate. All
three AV1 profiles are unsupported for complete picture decoding; 8/10/12-bit
scalar helpers do not imply profile support. Regenerate with
`python3 verification/feature_status.py`; CI checks for drift.

## Full decoder fuzzing

The dedicated `full-decoder-fuzz` CI gate checks the built library's AV1-pixel
capability and a distinct reviewed full decoder target before invoking cargo-fuzz.
It currently reports BLOCKED and fails. Existing header, CDF, partition, transform,
picture and filter targets continue to run independently; they do not satisfy
this requirement. No end-to-end decoder fuzz target is claimed as implemented.

When the decoder exists, implement a target covering compressed packet submission,
frame draining and plane validation, multi-frame reference refresh, corrupt-frame
rollback, show-existing/hidden frames, flush/reset/seeking and EOF/truncation.
Fuzz sequences of operations as well as bytes; assert bounded frames/tiles/blocks,
reference immutability after failures and no invented output. Seed with complete
valid intra/inter/multi-tile/8/10/12-bit/lossless/filter streams and malformed
variants. Add its distinct name to `full_decoder_fuzz_target` only after review.
The wrapper sets process/RSS/input/time limits; the target must enforce decoder
allocation and dimension budgets itself. ASan smoke fuzzing is not exhaustive;
continuous campaigns, corpus retention and minimization remain operational work.

## Repeatable performance measurements

`verification/benchmark.py` verifies native pixels against the independent
reference **before** timing. Every timed run must reproduce the verified output
hash. Each workload uses one warmup and seven measured fresh processes, recording
all wall/CPU samples, median time, FPS and p95 process time. Startup and raw output
I/O are included; these are not per-frame latency or decoder-kernel timings.
Allocations, peak RSS and dropped-frame measurements are explicitly unavailable,
not fabricated zeros. Add in-process instrumentation once the decoder exists.

`verification/benchmark.json` reserves 120-frame 720p/1080p/1440p/4K workloads;
fixture paths and digests are missing, so measurements remain BLOCKED. Install
reviewed fixed clips, native geometry/depth/counts and SHA-256 hashes. Keep all
reference versions, compiler/target/flags, thread counts, power/governor settings,
CPU affinity, thermal state and OS constant. No baseline is recorded from parsing
or synthetic data. `benchmark-baseline.json` is explicitly blocked.

The manually dispatched `Decoder performance regression` workflow requires a
self-hosted runner labeled `c-lib-benchmark`; provisioning that hardware is not
performed here. Once workloads and a decoder exist, save a reviewed successful
report as the baseline. Comparisons require identical context and workload sets,
at least five samples, positive finite timings and no blocked/failed cases. A
median process-time increase over 10% fails. Upload every result for trend history;
update baselines only after reviewing real changes. Hosted runner timing is not
used to silently approve a regression. This is a repeatable benchmark harness and
regression gate; genuine decoder performance numbers remain unavailable.

## Tested artifacts and reproducibility

Rust is pinned to **1.99.1**, dependency versions/checksums to `Cargo.lock`, and C
header generation to cbindgen **0.29.4**. Build with `--locked`; release compilation
uses offline dependencies after an initial checksum-verified fetch. Set up the
same compiler/linker/OS environment for reproduction. No cross-OS or different
linker byte identity is promised.

On Linux x86_64, from a clean committed tree:

```sh
python3 verification/release.py --out /tmp/c-lib-release
(cd /tmp/c-lib-release && sha256sum -c SHA256SUMS)
```

The release script builds two clean target directories with normalized paths,
compares all three shipped binaries byte for byte, runs the release test/doc
examples, builds Rustdoc with warnings denied, and packages binaries, C header,
Rustdoc, API/support docs, C example, exact git source archive and build manifest.
Tar entries use sorted names, normalized ownership/modes and commit timestamps;
gzip timestamps are zeroed. Checksums and toolchain/source provenance accompany
the bundle. Binary identity is checked across the two builds; Rustdoc/archive
identity across separate hosts is not established by that check.

`Tested experimental artifacts` runs on version tags or manual dispatch and
uploads the bundle/checksum as workflow artifacts. It creates no tag or public
GitHub Release. These are experimental library artifacts with capability mask 0,
not a production AV1 decoder release. Production decoder publication must require
the strict decoder gates and a measured supported workload baseline. Target
coverage beyond Linux x86_64 and long-lived signed release hosting remain future
work; workflow artifact retention follows repository settings.

## Sanitizer doctests

The AddressSanitizer job sets both `RUSTFLAGS` and `RUSTDOCFLAGS` to
`-Z sanitizer=address`. Rustdoc does not inherit rustc's flags. With an
instrumented standard library and no rustdoc sanitizer flag, doctest linking
fails with undefined `__asan_*` symbols. Keep unit, integration **and** doctests
in the ASan job; normal Rustdoc examples also run in the API documentation job.
