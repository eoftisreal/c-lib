use codec_av1::{decode_intra_frame, DecoderState};

#[test]
fn independently_encoded_palette_and_gradient_match_every_sample() {
    for (name, data, expected, width, height) in [
        (
            "palette",
            include_bytes!("../../../tests/intra_lossless_palette.obu").as_slice(),
            include_bytes!("../../../tests/oracles/intra_lossless_palette.yuv").as_slice(),
            48,
            32,
        ),
        (
            "gradient",
            include_bytes!("../../../tests/intra_lossless_gradient.obu").as_slice(),
            include_bytes!("../../../tests/oracles/intra_lossless_gradient.yuv").as_slice(),
            64,
            48,
        ),
    ] {
        let frame = decode_intra_frame(data, &mut DecoderState::default())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            (frame.width(), frame.height(), frame.bit_depth()),
            (width, height, 8)
        );
        let mut offset = 0;
        for (p, plane) in frame.planes().iter().enumerate() {
            for y in 0..plane.height() {
                for x in 0..plane.width() {
                    assert_eq!(
                        plane.row(y).unwrap()[x],
                        u16::from(expected[offset]),
                        "fixture={name} frame=0 plane={p} x={x} y={y}"
                    );
                    offset += 1;
                }
            }
        }
        assert_eq!(offset, expected.len());
    }
}
#[test]
fn lossless_core_matches_every_reference_sample() {
    let mut state = DecoderState::default();
    let frame = decode_intra_frame(
        include_bytes!("../../../tests/intra_lossless_core.obu"),
        &mut state,
    )
    .unwrap();
    assert_eq!(
        (frame.width(), frame.height(), frame.bit_depth()),
        (128, 96, 8)
    );
    assert_eq!(frame.subsampling(), codec_core::ChromaSubsampling::Cs420);
    let expected = include_bytes!("../../../tests/oracles/intra_lossless_core.yuv");
    let mut offset = 0;
    for (p, plane) in frame.planes().iter().enumerate() {
        for y in 0..plane.height() {
            for x in 0..plane.width() {
                assert_eq!(
                    plane.row(y).unwrap()[x],
                    u16::from(expected[offset]),
                    "frame=0 plane={p} x={x} y={y}"
                );
                offset += 1;
            }
        }
    }
    assert_eq!(offset, expected.len());
}

#[test]
fn original_palette_picture_matches_every_reference_sample() {
    let mut state = DecoderState::default();
    let frame = decode_intra_frame(
        include_bytes!("../../../tests/intra_lossless.obu"),
        &mut state,
    )
    .unwrap();
    let expected = include_bytes!("../../../tests/oracles/intra_lossless_core.yuv");
    let mut offset = 0;
    for (p, plane) in frame.planes().iter().enumerate() {
        for y in 0..plane.height() {
            for x in 0..plane.width() {
                assert_eq!(
                    plane.row(y).unwrap()[x],
                    u16::from(expected[offset]),
                    "frame=0 plane={p} x={x} y={y}"
                );
                offset += 1;
            }
        }
    }
    assert_eq!(offset, expected.len());
}

#[test]
fn truncated_picture_does_not_commit_header_or_reference_state() {
    let mut state = DecoderState::default();
    decode_intra_frame(
        include_bytes!("../../../tests/intra_lossless_core.obu"),
        &mut state,
    )
    .unwrap();
    let before = state.clone();
    for data in [
        include_bytes!("../../../tests/intra_lossless_core.obu").as_slice(),
        include_bytes!("../../../tests/intra_lossless.obu").as_slice(),
    ] {
        // Includes OBU/header cuts and cuts deep into coefficient decoding.
        for length in [
            0,
            1,
            2,
            4,
            8,
            16,
            32,
            64,
            128,
            512,
            1024,
            2048,
            data.len() - 2,
            data.len() - 1,
        ] {
            assert!(
                decode_intra_frame(&data[..length], &mut state).is_err(),
                "length={length}"
            );
            assert_eq!(state, before, "length={length}");
        }
    }
}

#[test]
fn mutated_compressed_pictures_never_panic_and_errors_roll_back() {
    let mut established = DecoderState::default();
    decode_intra_frame(
        include_bytes!("../../../tests/intra_lossless_core.obu"),
        &mut established,
    )
    .unwrap();
    for original in [
        include_bytes!("../../../tests/intra_lossless_core.obu").as_slice(),
        include_bytes!("../../../tests/intra_lossless.obu").as_slice(),
    ] {
        for index in (0..original.len()).step_by(17) {
            let mut data = original.to_vec();
            data[index] ^= 0xff;
            let mut state = established.clone();
            if decode_intra_frame(&data, &mut state).is_err() {
                assert_eq!(state, established, "mutation={index}");
            }
        }
    }
}
