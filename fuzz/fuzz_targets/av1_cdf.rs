#![no_main]
use codec_av1::entropy::SymbolDecoder;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    if data.len() < 40 {
        return;
    }
    if let Ok(mut decoder) = SymbolDecoder::new(&data[40..], data[0] & 1 != 0) {
        let n = usize::from(data[1] % 19);
        let mut cdf: Vec<_> = data[2..40]
            .as_chunks::<2>()
            .0
            .iter()
            .take(n)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        // Half of the calls have structurally valid, nonuniform CDFs.
        if data[0] & 2 != 0 && n >= 3 {
            cdf[..n - 2].sort_unstable();
            for v in &mut cdf[..n - 2] {
                *v &= 32767;
            }
            cdf[..n - 2].sort_unstable();
            cdf[n - 2] = 32768;
            cdf[n - 1] = u16::from(data[2] % 33);
        }
        for _ in 0..32 {
            let before = cdf.clone();
            let mut checkpoint = decoder.clone();
            match decoder.read_symbol(&mut cdf) {
                Ok(symbol) => {
                    assert!(symbol < cdf.len() - 1);
                }
                Err(_) => {
                    assert_eq!(cdf, before);
                    assert_eq!(decoder.read_bool(), checkpoint.read_bool());
                    break;
                }
            }
        }
        let _ = decoder.finish();
    }
});
