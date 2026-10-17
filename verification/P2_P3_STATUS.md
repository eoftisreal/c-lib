# P2 verification and P3 dependency gates

A first scoped compressed-picture test now passes: `intra_lossless_core.obu`
decodes through native entropy, coefficients, dequantization and reconstruction,
and all 18,432 samples match libaom. See [scope](../docs/AV1_SUPPORT.md),
[fixture provenance](../tests/oracles/README.md), and the separate
[core CI gate](../.github/workflows/core-pixel-conformance.yml). The original
palette fixture now also matches the independent raw oracle exactly. The broad
acceptance matrix remains incomplete. Full-profile
fuzz/readiness gates continue to fail closed; `av1_picture` fuzzes the implemented
native core and does not assert complete decoder capability.

## Executable acceptance matrix

`conformance.json` pins the existing compressed fixtures by SHA-256, native layout,
dimensions, depth and expected displayed frame count. `conformance-report.json`
is the current reproducible snapshot: every case is BLOCKED. Missing 12-bit,
filtered, malformed-no-output and official suites remain explicit coverage gaps.
Feature labels are acceptance requirements, not evidence that a fixture exercises
every relevant branch. In particular, separate filter-specific fixtures are needed.

Run from the repository root (Python standard library only):

```sh
python3 verification/conformance.py --report conformance-report.json
# exits 2: blocked acceptance, NOT success
python3 -m unittest discover -s verification -p test_conformance.py -v
```

Exit codes: 0 means all listed cases passed pixel comparisons, 1 means a failure,
2 means at least one blocked case (or an empty suite). A suite with both failures
and blocked cases returns 1. Missing fixtures and unsupported native layouts are
blocked; hash changes, process errors/timeouts, bad lengths and pixel differences
fail. The checked-in snapshot can be regenerated with the first command. CI
uploads the matrix and explicitly expects exit 2 until the decoder is implemented;
this job's green status is **not** decoded-picture conformance.

When an independent c-lib decoder exists, pass JSON argv arrays to `--candidate`
and `--reference`, with `{input}` and `{output}` placeholders, plus
`--candidate-revision`, `--reference-revision` and `--comparator` pointing to the
built `yuv-compare` binary. Each decoder must emit tightly packed native Y/U/V,
10/12-bit little endian, in display order without padding. The harness records
commands, declared build revisions, input/output hashes, executable hashes and
first-mismatch stderr. Pin decoder dependencies and compiler configuration too.
Command revisions and independence are caller attestations requiring source review;
this tool cannot establish that an arbitrary executable actually uses c-lib.
Using libaom/dav1d for both outputs is not c-lib verification.

The harness uses fresh outputs per case, exact manifest frame counts and process
timeouts. Logs are read only to a 4096-byte tail. It executes trusted local tools
without a shell; it is not a sandbox or disk quota for arbitrary decoder programs.
Malformed-input acceptance needs a separate no-output/error contract and remains
blocked; it must not be treated as an ordinary successful raw-pixel decode.

`tests/real_av1.mp4` is native RGB (`gbrp` in ffprobe), not YUV420. Its comparison
is blocked pending a native RGB contract; a color conversion would not establish
bit-exact decoder output. The other checked-in fixtures have native YUV420 output.
Fixture geometry/counts were inspected with ffprobe; this is metadata inspection,
not c-lib decoding. The broad manifest snapshot is separate from the passing
seven-picture lossless corpus in `core-conformance.json` and
`lossless-pixel-report.json`.

## Official corpus

AOMedia publishes the [Argon Streams AV1 verification tool and streams](https://aomedia.org/av1-video-decoder-verification-tool/),
including output YUV digests. These have **not** been imported or run here. Do not
label repository-generated samples as official conformance streams. Import a
reviewed subset with redistribution/license review, upstream archive/member hashes,
native pixel geometry, expected displayed counts, and feature coverage into the
manifest. Compare native pixels against libaom or dav1d and retain upstream digests
as an additional independent check. A corpus digest alone does not diagnose a
pixel mismatch or prove every feature is covered.

## Robustness delivered in this change

- Raw YUV comparison validates all samples directly in the two input buffers,
  avoiding four extra decoded frame allocations. Even an invalid sample after an
  earlier mismatch is rejected. Tests cover all Y/U/V planes, 8/10/12-bit and
  420/422/444 layouts with odd geometry.
- Verification input buffers and raw plane deserialization use fallible allocation;
  the existing checked 512 MiB per raw frame bound remains. This is a verification
  utility limit, not a global decoder memory budget.
- New `av1_cdf` fuzz target exercises arbitrary/valid CDFs, symbol bounds and error
  atomicity. `av1_blocks` covers bounded partition callbacks, poison-on-error,
  dequantization, scan selection, transforms and incomplete picture rejection.
  `av1_filters` covers CDEF, super-resolution, Wiener/SGR and stripe boundaries.
  These target existing primitives; coefficient entropy decoding is still absent.
- CI runs the new targets under cargo-fuzz/AddressSanitizer and adds an ASan unit
  suite alongside existing TSan, scalar checks and Linux/macOS/Windows jobs.
  Short fuzz runs are smoke tests, not proof of memory safety. Continuous fuzzing,
  dedicated grain/lifecycle targets, strict production frame/tile/block budgets,
  full malformed corpus and broader sanitizers remain necessary.

## P3 gates (not implemented by this change)

| Area | Current capability | Gate before acceptance |
| --- | --- | --- |
| Scalar / SIMD | Tested scalar primitives; existing dispatch helpers | Complete bit-exact scalar decoder, then per-kernel differential SIMD tests |
| Workers / pools | Scheduling and memory helpers | Real tile dependencies, bounded reference/output pools, stress tests |
| Performance | CLI measures parser throughput | Decode actual pictures; measure FPS, latency, allocations and drops at 720p/1080p/1440p/4K |
| Vulkan | Descriptor validation | Capability probing, Video sessions, resources, submissions, DPB, synchronization and recovery |
| Rust / C playback | PlanarFrame, DPB ownership and C descriptor helpers | Packet/receive API, CPU-plane lifetime/stride/color ABI, flush/reset, PTS and real MP4 playback |
| Optional processing | Experimental DSL / neural helpers | Actual execution/inference, resource policy and separate display processing preserving reference pixels |

No production decoding, hardware acceleration or genuine decoding speed claim is
made from header parsing, synthetic frames, descriptor construction or helper tests.

## Local validation

Workspace debug and release suites: 110 tests passed in each configuration.
Formatting, workspace and fuzz-target Clippy (`-D warnings`), release build,
source inventory, 7,272 pinned specification table entries and nine Python harness
regressions passed. New cargo-fuzz targets were run with cargo-fuzz 0.13.2 and
Rust nightly 1.101.0-nightly (4ebd25825, 2026-10-15), AddressSanitizer enabled.
Local LeakSanitizer shutdown cannot inspect processes in this environment;
`ASAN_OPTIONS=detect_leaks=0` is required locally. CI retains its default leak
configuration. These checks do not establish full-picture conformance.

Successful sanitizer smoke runs (31 seconds each, timer limit 30 seconds):
`av1_blocks`: 362,977 inputs; `av1_filters`: 169,867 inputs;
`av1_cdf`: 2,448,863 inputs. No target panic or AddressSanitizer violation was
reported. Leak detection was disabled locally as described above.
