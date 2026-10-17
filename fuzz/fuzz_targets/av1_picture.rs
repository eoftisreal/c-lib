#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    if data.len() > 65536 { return; }
    let mut state = codec_av1::DecoderState::default();
    if codec_av1::decode_intra_frame(data, &mut state).is_err() {
        assert_eq!(state, codec_av1::DecoderState::default());
    }
});
