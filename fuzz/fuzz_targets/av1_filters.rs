#![no_main]
use codec_av1::frame::{
    cdef::CdefKernel,
    restoration::{RestorationSource, SgrprojParameters, WienerCoefficients},
    superres::upscale_row,
};
use codec_core::Plane;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    if data.len() < 16 {
        return;
    }
    let depth = [8, 10, 12][usize::from(data[0] % 3)];
    let w = usize::from(data[1] % 16) + 1;
    let h = usize::from(data[2] % 16) + 1;
    let max = (1u16 << depth) - 1;
    let samples: Vec<_> = (0..w * h)
        .map(|i| u16::from(data[16..].get(i).copied().unwrap_or(0)) & max)
        .collect();
    let pre = Plane::new(w, h, w, samples.clone()).unwrap();
    let post = Plane::new(w, h, w, samples).unwrap();
    let start = usize::from(data[3]) % h;
    let end = start + 1 + usize::from(data[4]) % (h - start);
    let source = RestorationSource::new(&pre, &post, (start, end), depth).unwrap();
    let origin = (0, start);
    let size = (w, end - start);
    let kernel = |a: u8, b: u8, c: u8| {
        [
            i16::from(a % 16) - 5,
            i16::from(b % 32) - 23,
            i16::from(c % 64) - 17,
        ]
    };
    for output in [
        WienerCoefficients {
            horizontal: kernel(data[5], data[6], data[7]),
            vertical: kernel(data[8], data[9], data[10]),
        }
        .apply(&source, origin, size),
        SgrprojParameters {
            set: data[11] % 17,
            xqd: [
                i16::from(data[12] % 128) - 96,
                i16::from(data[13] % 128) - 32,
            ],
        }
        .apply(&source, origin, size),
        CdefKernel {
            direction: data[5] % 9,
            primary: u16::from(data[6]),
            secondary: u16::from(data[7]),
            damping: data[8] % 8,
            bit_depth: depth,
        }
        .apply(&post, (0, 0), (w.min(8), h.min(8))),
        upscale_row(post.row(0).unwrap(), w + usize::from(data[14]), depth),
    ]
    .into_iter()
    .flatten()
    {
        assert!(output.iter().all(|&p| p <= max));
    }
    // Exercise checked overflow paths without constructing oversized buffers.
    assert!(WienerCoefficients {
        horizontal: [0; 3],
        vertical: [0; 3]
    }
    .apply(&source, (usize::MAX, start), size)
    .is_err());
});
