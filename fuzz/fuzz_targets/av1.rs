#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    let _ = codec_av1::obu::validate_sample(data);
    let _ = codec_av1::parse_sequence_header(&mut codec_bitstream::BitReader::new(data));
    let mut state = codec_av1::DecoderState::default();
    let _ = codec_av1::obu::validate_sample_with_state(data, &mut state);
    // Exercise failures after a valid sequence and frame have established state.
    let _ = codec_av1::obu::validate_sample_with_state(
        include_bytes!("../../tests/intra_tiles.obu"),
        &mut state,
    );
    let before = state.clone();
    if codec_av1::obu::validate_sample_with_state(data, &mut state).is_err() {
        assert_eq!(state, before);
    }
    if let Ok(mut decoder) = codec_av1::entropy::SymbolDecoder::new(data, false) {
        let mut cdf = [8192, 16384, 24576, 32768, 0];
        for _ in 0..256 {
            if decoder.read_symbol(&mut cdf).is_err() {
                break;
            }
        }
        let _ = decoder.finish();
    }
});
