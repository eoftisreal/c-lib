# Phase 1 status

**Phase 1 remains incomplete. A scoped compressed AV1-to-PlanarFrame entry point
now passes its first complete-picture comparison.**
`decode_intra_frame` supports the [verified lossless core](../docs/AV1_SUPPORT.md)
with real entropy-decoded coefficients and no synthetic pixels. Broader syntax,
filters and reference decoding remain incomplete. The component inventory below
predates that narrow integration; see the support page for current entry-point limits.
These changes implement and test independent scalar foundations, without replacing
missing syntax with invented modes, residuals or pixels.

| Area | Implemented | Remaining |
| --- | --- | --- |
| Entropy | Five exact default partition CDF banks, tile-local adaptation and edge gathering | Other syntax CDF banks, quantizer initialization and frame/tile inheritance integration |
| Partitions | All ten shapes, recursive 64/128 traversal, above/left contexts, coded order, clipped edges, tile isolation, callback-error poisoning | Actual compressed-block callback implementation and chroma shape restrictions |
| Compressed syntax | No synthetic mode/skip/segment values | Skip, segmentation, modes, angles, transform trees, CfL alpha and screen-content syntax |
| Coefficients | Six exact quantizer tables, current-Q/segment/plane step selection, scans, dequantization with supplied inverse-QM weights, clipping and low-24-bit reduction | EOB/base/range/sign entropy decoding, coefficient contexts, delta-Q/LF symbols, normative QM bank/selection |
| Intra | Basic/CfL primitives plus Smooth/Smooth-V/Smooth-H at 8/10/12-bit | Directional prediction, edge preparation/filtering/upsampling, filter-intra, palette, IntraBC, syntax connection |
| Transforms | Staged DCT4..64, ADST/FLIPADST4..16, identity4..32, checked/corrected WHT, 19 transform shapes, 2D shifts/scaling/flips/clipping | Bitstream transform-set integration and broader conformance corpus |
| Reconstruction | Real transform-to-YUV writes, overlap rejection, bounds, coverage-gated publication, odd dimensions, 420/422/444 and 8/10/12-bit | Full compressed-input pipeline, normative tile/plane scheduling, in-loop filters/reference publication and monochrome assembly |

## Independent validation

`tests/oracles/phase1_vectors.c` links separately built, unmodified libaom commit
`394f9f7df624e4268fd8c894ead9f059d279e480` from
https://aomedia.googlesource.com/aom. Cargo has no libaom runtime dependency.
The deterministic checked-in fixture contains 6,799 comparisons:
2,980 1D transforms; 2,385 2D pixel cases; 768 quantizer pairs; 192 WHT
residuals; 171 smooth blocks; 159 complete scans; 144 partition streams.

`verification/phase1_tables.py` checks 6,760 numeric entries against AV1 specification
commit `5e04f3f75e73a5898d7616c47c52f032144b8f80`. CI runs this comparison.
Other tests check partition geometry/order/edges/contexts, malformed coefficients,
quantizer derivation, dequantization and incomplete/overlapping picture assembly.

These comparisons establish component behavior, not complete bitstream decoding.
The next gate is the real intra block/coefficient pipeline followed by a full
independently decoded AV1 frame and sample-by-sample final Y/U/V comparison.
Keep compressed-input CLI Y4M output unsupported until that gate passes.

## Additional fixes

WHT previously reversed the second and third frequency inputs in both passes.
MP4 timing validation now rejects cumulative sample counts larger than the sample
table before expanding runs, avoiding billions of iterations on corrupt counts.
