use codec_av1::{
    entropy::SymbolDecoder,
    frame::{
        coeff_decode::{decode_tx_block_4x4, update_entropy_context, TileCoeffCdfs},
        default_cdfs::*,
        partition::BlockSize,
    },
};

#[test]
fn libaom_coefficient_vectors_preserve_levels_signs_and_entropy_cursor() {
    for line in include_str!("../../../tests/oracles/coeff_vectors.txt").lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        let bytes: Vec<_> = fields[1]
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|hex| u8::from_str_radix(std::str::from_utf8(hex).unwrap(), 16).unwrap())
            .collect();
        let mut skip = DEFAULT_TXB_SKIP_CDFS;
        let mut eob = DEFAULT_EOB_MULTI16_CDFS;
        let mut extra = DEFAULT_EOB_EXTRA_CDFS;
        let mut base_eob = DEFAULT_COEFF_BASE_EOB_CDFS;
        let mut base = DEFAULT_COEFF_BASE_CDFS;
        let mut br = DEFAULT_COEFF_BR_CDFS;
        let mut sign = DEFAULT_DC_SIGN_CDFS;
        let mut cdfs = TileCoeffCdfs {
            txb_skip_cdfs: &mut skip,
            eob_multi16_cdfs: &mut eob,
            eob_extra_cdfs: &mut extra,
            base_eob_cdfs: &mut base_eob,
            base_cdfs: &mut base,
            br_cdfs: &mut br,
            dc_sign_cdfs: &mut sign,
        };
        let mut decoder = SymbolDecoder::new(&bytes, false).unwrap();
        let (coeffs, zero) =
            decode_tx_block_4x4(&mut decoder, &mut cdfs, 0, BlockSize::Block4x4, 0, 0, false)
                .unwrap();
        let mut expected = [0; 16];
        expected[0] = fields[2].parse().unwrap();
        expected[1] = fields[3].parse().unwrap();
        assert_eq!(coeffs, expected, "case={}", fields[0]);
        assert!(!zero);
        let mut sentinel = 0u8;
        for _ in 0..8 {
            sentinel = (sentinel << 1) | u8::from(decoder.read_bool().unwrap());
        }
        assert_eq!(sentinel, 0xa5, "case={}", fields[0]);
        decoder.finish().unwrap();
    }
}

#[test]
fn entropy_context_saturates_even_for_extreme_public_inputs() {
    assert_eq!(update_entropy_context(&[i32::MAX; 16]), 0xbf);
    assert_eq!(update_entropy_context(&[i32::MIN; 16]), 0x7f);
}
