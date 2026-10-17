#![no_main]
use codec_bitstream::BitReader;
use codec_core::{CpuFlags, DecoderConfig, DspContext, SuperblockWavefrontGrid};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    let threads = usize::from((data[0] % 8).max(1));
    let suppress_cpu_grain = (data[1] & 1) != 0;
    let operating_point = data[2] & 0x1f;
    let payload = &data[4..];

    let _cfg = DecoderConfig {
        threads,
        suppress_cpu_grain,
        operating_point,
        ..DecoderConfig::default()
    };
    let grid = SuperblockWavefrontGrid::new(threads, 8, 8);
    let _ = grid.is_sb_ready(0, 0, 0);
    let _ = DspContext::new(CpuFlags::SCALAR);

    let mut state = codec_av1::DecoderState::default();
    let _ = codec_av1::obu::validate_sample(payload);
    let _ = codec_av1::obu::validate_sample_with_state(payload, &mut state);
    if let Some(seq) = state.sequence() {
        let _ =
            codec_av1::frame::ExtendedFrameHeader::parse_prefix(&mut BitReader::new(payload), seq);
    }
});
