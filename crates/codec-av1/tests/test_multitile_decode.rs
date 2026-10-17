use codec_av1::obu::validate_sample_with_state;
use codec_av1::{decode_intra_frame, DecoderState, ObuType};

const GRID8: &[u8] = include_bytes!("../../../tests/intra_lossless_tiles8.obu");
const GRID10: &[u8] = include_bytes!("../../../tests/intra_lossless_tiles10.obu");
const PALETTE8: &[u8] = include_bytes!("../../../tests/intra_lossless_tiles8_palette.obu");

fn obu_ranges(data: &[u8]) -> Vec<(ObuType, std::ops::Range<usize>)> {
    let mut reader = codec_bitstream::BitReader::new(data);
    let mut result = Vec::new();
    while reader.bits_left() > 0 {
        let start = data.len() - reader.bits_left() / 8;
        let header = codec_av1::parse_obu_header(&mut reader).unwrap();
        let size = reader.read_leb128().unwrap() as usize;
        reader.read_bytes(size).unwrap();
        result.push((header.obu_type, start..data.len() - reader.bits_left() / 8));
    }
    result
}

#[test]
fn four_tiles_match_every_independent_reference_sample() {
    for (name, data, oracle, depth, groups) in [
        (
            "grid8",
            GRID8,
            include_bytes!("../../../tests/oracles/intra_lossless_tiles8.yuv").as_slice(),
            8,
            0, // Four tiles packed into one combined Frame OBU.
        ),
        (
            "grid10",
            GRID10,
            include_bytes!("../../../tests/oracles/intra_lossless_tiles10.yuv").as_slice(),
            10,
            4,
        ),
        (
            "palette8",
            PALETTE8,
            include_bytes!("../../../tests/oracles/intra_lossless_tiles8_palette.yuv").as_slice(),
            8,
            4,
        ),
    ] {
        let mut state = DecoderState::default();
        let parsed = validate_sample_with_state(data, &mut state).unwrap();
        let header = &parsed.frame_headers[0];
        // Unequal 128/64-pixel tile extents, in both axes.
        assert_eq!(header.tiles.col_starts, [0, 32, 48], "{name}");
        assert_eq!(header.tiles.row_starts, [0, 32, 48], "{name}");
        assert_eq!(parsed.tile_ranges.len(), 4, "{name}");
        assert_eq!(
            obu_ranges(data)
                .iter()
                .filter(|(kind, _)| *kind == ObuType::TileGroup)
                .count(),
            groups,
            "{name}"
        );

        let picture = decode_intra_frame(data, &mut DecoderState::default()).unwrap();
        assert_eq!((picture.width(), picture.height()), (192, 192));
        assert_eq!(picture.bit_depth(), depth);
        let bytes_per_sample = if depth == 8 { 1 } else { 2 };
        let mut offset = 0;
        for (p, plane) in picture.planes().iter().enumerate() {
            for y in 0..plane.height() {
                for (x, &actual) in plane.row(y).unwrap().iter().enumerate() {
                    let expected = if depth == 8 {
                        u16::from(oracle[offset])
                    } else {
                        u16::from_le_bytes([oracle[offset], oracle[offset + 1]])
                    };
                    assert_eq!(actual, expected, "{name} plane={p} x={x} y={y}");
                    offset += bytes_per_sample;
                }
            }
        }
        assert_eq!(offset, oracle.len());
    }
}

#[test]
fn incomplete_or_reordered_groups_do_not_publish_or_commit() {
    let mut state = DecoderState::default();
    decode_intra_frame(
        include_bytes!("../../../tests/intra_lossless_core.obu"),
        &mut state,
    )
    .unwrap();
    let before = state.clone();
    let groups: Vec<_> = obu_ranges(GRID10)
        .into_iter()
        .filter(|(kind, _)| *kind == ObuType::TileGroup)
        .map(|(_, range)| range)
        .collect();
    assert_eq!(groups.len(), 4);
    for group in &groups {
        assert!(decode_intra_frame(&GRID10[..group.start], &mut state).is_err());
        assert_eq!(state, before);
    }
    // Each mutation preserves OBU framing but violates tile group ordering.
    for order in [[1, 0, 2, 3], [0, 0, 2, 3], [0, 2, 1, 3]] {
        let mut data = GRID10[..groups[0].start].to_vec();
        for index in order {
            data.extend_from_slice(&GRID10[groups[index].clone()]);
        }
        assert!(decode_intra_frame(&data, &mut state).is_err());
        assert_eq!(state, before);
    }
}

#[test]
fn last_tile_entropy_failure_rolls_back_after_earlier_tiles_reconstruct() {
    for original in [GRID8, GRID10, PALETTE8] {
        let mut state = DecoderState::default();
        decode_intra_frame(GRID8, &mut state).unwrap();
        let before = state.clone();
        let parsed = validate_sample_with_state(original, &mut state.clone()).unwrap();
        let mut broken = original.to_vec();
        broken[parsed.tile_ranges[3].clone()].fill(0);
        // Tile lengths and headers still validate: the error is in entropy data.
        validate_sample_with_state(&broken, &mut state.clone()).unwrap();
        assert!(decode_intra_frame(&broken, &mut state).is_err());
        assert_eq!(state, before);
        // A failed picture must not poison a later valid picture.
        decode_intra_frame(original, &mut state).unwrap();
    }
}
