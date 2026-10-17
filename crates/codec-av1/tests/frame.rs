use codec_av1::{
    frame::{ExtendedFrameHeader, ExtendedFrameType, FrameType},
    obu::{validate_sample, validate_sample_with_state},
    DecoderState, ObuType, ParserState,
};
use codec_bitstream::BitReader;
use codec_core::{CodecError, SuperblockWavefrontGrid};
use container_mp4::Mp4Demuxer;
const REAL: &[u8] = include_bytes!("../../../tests/real_av1.mp4");
const MULTI: &[u8] = include_bytes!("../../../tests/multi_av1.mp4");
fn first(bytes: &[u8]) -> Vec<u8> {
    let mut d = Mp4Demuxer::new(bytes);
    d.parse_moov().unwrap();
    d.get_sample(0, 0).unwrap().data.to_vec()
}
#[test]
fn real_keyframes_match_external_trace() {
    for bytes in [REAL, MULTI] {
        let data = first(bytes);
        let mut state = DecoderState::default();
        let parsed = validate_sample_with_state(&data, &mut state).unwrap();
        assert_eq!(parsed.frame_headers.len(), 1);
        let h = &parsed.frame_headers[0];

        assert_eq!(h.frame_type, FrameType::Key);
        assert!(h.show_frame && h.error_resilient_mode);
        assert_eq!((h.dimensions.width, h.dimensions.height), (320, 240));
        assert_eq!(h.refresh_frame_flags, 255);
        assert_eq!(h.tiles.col_starts, vec![0, 80]);
        assert_eq!(h.tiles.row_starts, vec![0, 60]);
        assert_eq!(parsed.tile_ranges.len(), 1);
        assert_eq!(parsed.tile_ranges[0].end, data.len());
        assert!(state
            .references()
            .iter()
            .all(|r| r.as_ref().unwrap().header == *h));
        assert!(matches!(state.parser(), ParserState::AwaitingHeader));
        if bytes == REAL {
            assert_eq!(h.header_bits, 37);
            assert_eq!(h.quantization.base_q_idx, 128);
            assert!(h.allow_intrabc);
            assert_eq!(parsed.tile_ranges[0], 23..820);
        }
        if bytes == MULTI {
            assert_eq!(h.header_bits, 97);
            assert_eq!(h.quantization.base_q_idx, 140);
            assert_eq!(h.delta.q_res, Some(2));
            assert_eq!(h.loop_filter.levels, [9, 9, 14, 13]);
            assert_eq!(h.cdef.damping, 5);
            assert_eq!(h.cdef.strengths, vec![[0, 2, 0, 2], [11, 2, 11, 2]]);
            assert_eq!(parsed.tile_ranges[0].len(), 1711);
        }
    }
}
#[test]
fn multiframes_preserve_structural_api_and_reject_unsupported_transactionally() {
    let mut d = Mp4Demuxer::new(MULTI);
    d.parse_moov().unwrap();
    let mut state = DecoderState::default();
    for i in 0..12 {
        let packet = d.get_sample(0, i).unwrap();
        validate_sample(&packet.data).unwrap();
        if i == 0 {
            validate_sample_with_state(&packet.data, &mut state).unwrap();
        } else {
            let saved = state.clone();
            assert!(matches!(
                validate_sample_with_state(&packet.data, &mut state),
                Err(CodecError::Unsupported)
            ));
            assert_eq!(state, saved);
        }
    }
}
#[test]
fn extended_inter_and_show_existing_headers_across_all_12_multi_av1_samples() {
    let mut d = Mp4Demuxer::new(MULTI);
    d.parse_moov().unwrap();
    let mut seq = None;
    let mut decoded_headers = Vec::new();
    for i in 0..12 {
        let packet = d.get_sample(0, i).unwrap();
        let mut r = BitReader::new(&packet.data);
        while r.bits_left() != 0 {
            let obu = codec_av1::parse_obu_header(&mut r).unwrap();
            let sz = r.read_leb128().unwrap() as usize;
            let payload = r.read_bytes(sz).unwrap();
            if obu.obu_type == ObuType::SequenceHeader {
                seq = Some(codec_av1::parse_sequence_header(&mut BitReader::new(payload)).unwrap());
            } else if matches!(obu.obu_type, ObuType::Frame | ObuType::FrameHeader) {
                let h = ExtendedFrameHeader::parse_prefix(
                    &mut BitReader::new(payload),
                    seq.as_ref().unwrap(),
                )
                .unwrap();
                decoded_headers.push((i, h));
            }
        }
    }
    assert_eq!(decoded_headers[0].1.frame_type, ExtendedFrameType::Key);
    assert!(decoded_headers
        .iter()
        .any(|(_, h)| h.show_existing_frame && h.frame_to_show_map_idx == Some(3)));
    assert!(decoded_headers
        .iter()
        .any(|(_, h)| h.show_existing_frame && h.frame_to_show_map_idx == Some(2)));
    assert!(decoded_headers
        .iter()
        .any(|(_, h)| h.show_existing_frame && h.frame_to_show_map_idx == Some(6)));
}
#[test]
fn coverage_decode_no_panic_across_corrupted_multitile_and_thread_counts() {
    let base = include_bytes!("../../../tests/intra_tiles.obu");
    for threads in 1..=8 {
        for cut in (0..base.len()).step_by(7) {
            let mut corrupted = base[..cut].to_vec();
            if let Some(last) = corrupted.last_mut() {
                *last ^= 0x5a;
            }
            let mut state = DecoderState::default();
            let _ = validate_sample(&corrupted);
            let _ = validate_sample_with_state(&corrupted, &mut state);
            let grid = SuperblockWavefrontGrid::new(threads, 4, 4);
            assert!(grid.is_sb_ready(0, 0, 0));
        }
    }
}
#[test]
fn failed_sample_rolls_back_sequence_and_frame_changes() {
    let good = first(MULTI);
    let mut state = DecoderState::default();
    validate_sample_with_state(&good, &mut state).unwrap();
    let saved = state.clone();
    let mut bad = good.clone();
    bad.extend([0x12, 1, 0]);
    assert!(validate_sample_with_state(&bad, &mut state).is_err());
    assert_eq!(state, saved);
    // Every byte truncation, excluding a complete sequence-only prefix, fails.
    for cut in 1..good.len() {
        let mut s = saved.clone();
        if validate_sample_with_state(&good[..cut], &mut s).is_err() {
            assert_eq!(s, saved);
        }
    }
    assert!(validate_sample_with_state(&[0x22, 1, 0x80], &mut state).is_err());
    assert_eq!(state, saved);
    validate_sample_with_state(&[0x12, 0], &mut state).unwrap();
    assert_eq!(state.temporal_unit(), 1);
    state.reset();
    assert_eq!(state, DecoderState::default());
}

#[test]
fn independently_encoded_lossless_and_four_tiles() {
    let mut state = DecoderState::default();
    let r = validate_sample_with_state(
        include_bytes!("../../../tests/intra_lossless.obu"),
        &mut state,
    )
    .unwrap();
    let h = &r.frame_headers[0];
    assert!(state.sequence().unwrap().reduced_still_picture_header);
    assert!(h.coded_lossless);
    assert_eq!(h.quantization.base_q_idx, 0);
    assert_eq!(h.loop_filter.levels, [0; 4]);
    assert_eq!(h.tx_mode, codec_av1::frame::TxMode::Only4x4);
    assert!(h.disable_frame_end_update_cdf);
    let mut state = DecoderState::default();
    let bytes = include_bytes!("../../../tests/intra_tiles.obu");
    let r = validate_sample_with_state(bytes, &mut state).unwrap();
    let h = &r.frame_headers[0];
    assert_eq!(h.tiles.col_starts, vec![0, 32, 64]);
    assert_eq!(h.tiles.row_starts, vec![0, 16, 32]);
    assert_eq!(h.tiles.tile_size_bytes, 2);
    assert_eq!(h.header_bits, 99);
    assert_eq!(r.tile_ranges.len(), 4);
    assert_eq!(r.tile_ranges.last().unwrap().end, bytes.len());
    for (i, pair) in r.tile_ranges.windows(2).enumerate() {
        assert_eq!(pair[0].end + if i == 2 { 0 } else { 2 }, pair[1].start);
    }
}

fn obu(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![(kind << 3) | 2];
    let mut n = payload.len();
    loop {
        let byte = (n & 127) as u8;
        n >>= 7;
        out.push(byte | if n > 0 { 128 } else { 0 });
        if n == 0 {
            break;
        }
    }
    out.extend(payload);
    out
}
fn split_frame(data: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut r = codec_bitstream::BitReader::new(data);
    let mut prefix = Vec::new();
    while r.bits_left() > 0 {
        let h = codec_av1::parse_obu_header(&mut r).unwrap();
        let size = r.read_leb128().unwrap() as usize;
        let p = r.read_bytes(size).unwrap();
        if h.obu_type == codec_av1::ObuType::Frame {
            return (prefix, p.to_vec());
        }
        prefix.extend(obu(h.obu_type as u8, p));
    }
    panic!("no combined frame");
}
#[test]
fn split_headers_redundancy_and_tile_completion() {
    let original = first(MULTI);
    let (prefix, frame) = split_frame(&original);
    // Trace-verified header is 97 bits, followed by 7 zero alignment bits.
    let mut header = frame[..13].to_vec();
    header[12] |= 0x40;
    let header_obu = obu(3, &header);
    let tile_obu = obu(4, &frame[13..]);
    let mut state = DecoderState::default();
    validate_sample_with_state(&original, &mut state).unwrap();
    validate_sample_with_state(&prefix, &mut state).unwrap();
    validate_sample_with_state(&header_obu, &mut state).unwrap();
    assert!(state.references().iter().all(Option::is_none));
    assert!(matches!(
        state.parser(),
        ParserState::AwaitingTiles { next_tile: 0, .. }
    ));
    let pending = state.clone();
    for bad in [&header_obu[..], &[0x12, 0][..], &[0x22, 0][..]] {
        assert!(validate_sample_with_state(bad, &mut state).is_err());
        assert_eq!(state, pending);
    }
    validate_sample_with_state(&obu(7, &header), &mut state).unwrap();
    assert_eq!(state, pending);
    // force_integer_mv is overwritten to 1 for intra frames. Even if parsed values
    // coincide, a different encoded header is not a valid redundant copy.
    let mut changed = header.clone();
    changed[0] ^= 2;
    assert!(validate_sample_with_state(&obu(7, &changed), &mut state).is_err());
    assert_eq!(state, pending);
    validate_sample_with_state(&tile_obu, &mut state).unwrap();
    assert!(state.references().iter().all(Option::is_some));
    assert!(matches!(state.parser(), ParserState::AwaitingHeader));
    let complete = state.clone();
    assert!(validate_sample_with_state(&obu(7, &header), &mut state).is_err());
    assert_eq!(state, complete);
}
#[test]
fn partial_tile_groups_must_be_ordered_and_bounded() {
    let original = include_bytes!("../../../tests/intra_tiles.obu");
    let mut complete = DecoderState::default();
    let parsed = validate_sample_with_state(original, &mut complete).unwrap();
    let (prefix, frame) = split_frame(original);
    // 99 bits, then trailing one in place of combined-frame zero alignment.
    let mut header = frame[..13].to_vec();
    header[12] |= 0x10;
    let mut state = DecoderState::default();
    validate_sample_with_state(&prefix, &mut state).unwrap();
    validate_sample_with_state(&obu(3, &header), &mut state).unwrap();
    for i in 0..4 {
        // flag=1, start=i (2 bits), end=i (2 bits), zero byte alignment.
        let mut payload = vec![0x80 | ((i as u8) << 5) | ((i as u8) << 3)];
        payload.extend(&original[parsed.tile_ranges[i].clone()]);
        let before = state.clone();
        if i < 3 {
            let mut bad = payload.clone();
            bad[0] |= 1;
            assert!(validate_sample_with_state(&obu(4, &bad), &mut state).is_err());
            assert_eq!(state, before);
        }
        validate_sample_with_state(&obu(4, &payload), &mut state).unwrap();
        let saved = state.clone();
        assert!(validate_sample_with_state(&obu(4, &payload), &mut state).is_err());
        assert_eq!(state, saved);
    }
    assert_eq!(state.references(), complete.references());
}
#[test]
fn ten_bit_restoration_syntax_matches_trace() {
    let mut state = DecoderState::default();
    let parsed =
        validate_sample_with_state(include_bytes!("../../../tests/intra_10bit.obu"), &mut state)
            .unwrap();
    assert!(state.sequence().unwrap().color_config.high_bitdepth);
    assert!(state.sequence().unwrap().enable_restoration);
    let h = &parsed.frame_headers[0];
    assert_eq!(h.header_bits, 88);
    assert_eq!(h.quantization.base_q_idx, 128);
    assert_eq!(h.restoration.types, [0, 0, 0]);
}
