use codec_av1::entropy::SymbolDecoder;
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|x| u8::from_str_radix(std::str::from_utf8(x).unwrap(), 16).unwrap())
        .collect()
}
fn nums(s: &str) -> Vec<u16> {
    s.split(',').map(|x| x.parse().unwrap()).collect()
}
#[test]
fn libaom_known_answers_all_alphabets_and_update_modes() {
    for line in include_str!("../../../tests/oracles/entropy_vectors.txt").lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        let n: usize = fields[0].parse().unwrap();
        let adaptive = fields[1] == "1";
        let data = hex(fields[2]);
        let symbols = nums(fields[3]);
        let expected = nums(fields[4]);
        let mut cdf: Vec<u16> = (1..=n).map(|i| (i * 32768 / n) as u16).chain([0]).collect();
        let mut decoder = SymbolDecoder::new(&data, !adaptive).unwrap();
        for (i, symbol) in symbols.iter().enumerate() {
            assert_eq!(
                decoder.read_symbol(&mut cdf).unwrap(),
                usize::from(*symbol),
                "n={n} adaptive={adaptive} symbol={i}"
            );
        }
        assert_eq!(cdf, expected, "n={n}, adaptive={adaptive}");
        decoder.finish().unwrap();
        // Removing any trailing byte cannot complete this independently known stream.
        for cut in 0..data.len() {
            let mut cdf: Vec<u16> = (1..=n).map(|i| (i * 32768 / n) as u16).chain([0]).collect();
            let result = SymbolDecoder::new(&data[..cut], !adaptive).and_then(|mut d| {
                for &s in &symbols {
                    if d.read_symbol(&mut cdf)? != usize::from(s) {
                        return Err(codec_core::CodecError::InvalidData);
                    }
                }
                d.finish()
            });
            assert!(result.is_err(), "accepted truncated oracle n={n} cut={cut}");
        }
    }
}
#[test]
fn invalid_cdf_and_errors_are_atomic() {
    let mut d = SymbolDecoder::new(&[0x80], false).unwrap();
    for mut cdf in [
        vec![],
        vec![32768, 0],
        vec![20000, 10000, 32768, 0],
        vec![0, 32000, 0],
        vec![16384, 32768, 33],
        vec![0, 32768, 0],
    ] {
        let before = cdf.clone();
        assert!(d.read_symbol(&mut cdf).is_err());
        assert_eq!(cdf, before);
    }
    let mut original = SymbolDecoder::new(&[0x80], false).unwrap();
    assert_eq!(d.read_bool().unwrap(), original.read_bool().unwrap());
    let mut cdf = [16384, 32768, 0];
    loop {
        let previous = cdf;
        if d.read_symbol(&mut cdf).is_err() {
            assert_eq!(cdf, previous);
            assert!(d.read_symbol(&mut cdf).is_err());
            break;
        }
    }
    assert!(SymbolDecoder::new(&[], false).is_err());
    assert!(SymbolDecoder::new(&[0], false).unwrap().finish().is_err());
    SymbolDecoder::new(&[0x80], false)
        .unwrap()
        .finish()
        .unwrap();
}
#[test]
fn bool_and_literal_match_fixed_binary_oracle() {
    let fields: Vec<_> = include_str!("../../../tests/oracles/entropy_vectors.txt")
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .collect();
    let data = hex(fields[2]);
    let expected = nums(fields[3]);
    let mut d = SymbolDecoder::new(&data, false).unwrap();
    for &v in &expected {
        assert_eq!(d.read_bool().unwrap(), v != 0);
    }
    d.finish().unwrap();
    let mut d = SymbolDecoder::new(&data, true).unwrap();
    for chunk in expected.chunks(32) {
        let value = chunk.iter().fold(0u32, |a, b| (a << 1) | u32::from(*b));
        assert_eq!(d.read_literal(32).unwrap(), value);
    }
    d.finish().unwrap();
}
