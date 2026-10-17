use codec_av1::{decode_intra_frame, DecoderState};
use codec_core::ChromaSubsampling;
use std::path::Path;

fn corpus() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../verification/intra-4x4-conformance.json"
    ))
    .unwrap()
}

#[test]
fn independent_keyframes_match_all_native_samples_across_depth_chroma_and_quantizers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let corpus = corpus();
    let cases = corpus["cases"].as_array().unwrap();
    let mut quantizer_contexts = [false; 4];
    let mut samples = 0;
    for case in cases {
        let name = case["id"].as_str().unwrap();
        let data = std::fs::read(root.join(case["path"].as_str().unwrap())).unwrap();
        let oracle = std::fs::read(root.join(case["oracle"].as_str().unwrap())).unwrap();
        let depth = case["bit_depth"].as_u64().unwrap() as u8;
        let layout = match case["layout"].as_str().unwrap() {
            "420" => ChromaSubsampling::Cs420,
            "422" => ChromaSubsampling::Cs422,
            "444" => ChromaSubsampling::Cs444,
            _ => panic!("unexpected fixture layout"),
        };
        let mut state = DecoderState::default();
        let frame =
            decode_intra_frame(&data, &mut state).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(frame.bit_depth(), depth, "{name}");
        assert_eq!(frame.subsampling(), layout, "{name}");
        assert_eq!(
            frame.width() as u64,
            case["width"].as_u64().unwrap(),
            "{name}"
        );
        assert_eq!(
            frame.height() as u64,
            case["height"].as_u64().unwrap(),
            "{name}"
        );
        let header = state.last_header().unwrap();
        let lossless = case["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x == "lossless");
        assert_eq!(header.coded_lossless, lossless, "{name}");
        if !lossless {
            let q = header.quantization.base_q_idx;
            let ctx = if q <= 20 {
                0
            } else if q <= 60 {
                1
            } else if q <= 120 {
                2
            } else {
                3
            };
            quantizer_contexts[ctx] = true;
        }
        if name == "lossy_4x4_tiles.obu" {
            assert_eq!(header.tiles.count(), 4);
        }
        let bytes_per_sample = if depth == 8 { 1 } else { 2 };
        let mut offset = 0;
        for (p, plane) in frame.planes().iter().enumerate() {
            for y in 0..plane.height() {
                for (x, &actual) in plane.row(y).unwrap().iter().enumerate() {
                    let expected = if depth == 8 {
                        u16::from(oracle[offset])
                    } else {
                        u16::from_le_bytes([oracle[offset], oracle[offset + 1]])
                    };
                    assert_eq!(actual, expected, "{name}: plane={p} x={x} y={y}");
                    offset += bytes_per_sample;
                    samples += 1;
                }
            }
        }
        assert_eq!(
            offset,
            oracle.len(),
            "{name}: incomplete reference consumption"
        );
    }
    assert_eq!(cases.len(), 27);
    assert_eq!(quantizer_contexts, [true; 4]);
    assert_eq!(samples, 113664);
}

#[test]
fn invalid_or_unsupported_pictures_cannot_mutate_established_state() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut state = DecoderState::default();
    decode_intra_frame(
        include_bytes!("../../../tests/intra_lossless_core.obu"),
        &mut state,
    )
    .unwrap();
    let before = state.clone();
    for case in corpus()["cases"].as_array().unwrap() {
        let name = case["id"].as_str().unwrap();
        let bytes = std::fs::read(root.join(case["path"].as_str().unwrap())).unwrap();
        for cut in [0, 1, bytes.len() / 2, bytes.len() - 1] {
            assert!(
                decode_intra_frame(&bytes[..cut], &mut state).is_err(),
                "{name}: cut={cut}"
            );
            assert_eq!(state, before, "{name}: cut={cut}");
        }
        let parsed =
            codec_av1::obu::validate_sample_with_state(&bytes, &mut state.clone()).unwrap();
        let mut broken = bytes;
        broken[parsed.tile_ranges.last().unwrap().clone()].fill(0);
        assert!(decode_intra_frame(&broken, &mut state).is_err(), "{name}");
        assert_eq!(state, before, "{name}");
    }
    // Existing broad fixtures need larger transforms, filters or inter decoding.
    for bytes in [
        include_bytes!("../../../tests/intra_10bit.obu").as_slice(),
        include_bytes!("../../../tests/intra_tiles.obu").as_slice(),
    ] {
        assert!(decode_intra_frame(bytes, &mut state).is_err());
        assert_eq!(state, before);
    }
}
