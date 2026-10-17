use codec_av1::frame::{cdef::CdefKernel, superres::upscale_row};
use codec_core::Plane;
fn values(s: &str) -> Vec<u16> {
    s.split(',').map(|v| v.parse().unwrap()).collect()
}
#[test]
fn libaom_superres_and_cdef_known_answers() {
    let mut counts = [0; 4];
    for (line_no, line) in include_str!("../../../tests/oracles/p1_vectors.txt")
        .lines()
        .enumerate()
    {
        let f: Vec<_> = line.split_whitespace().collect();
        let n = |i: usize| f[i].parse::<usize>().unwrap();
        match f[0] {
            "S" => {
                let src = values(f[4]);
                assert_eq!(src.len(), n(2));
                assert_eq!(
                    upscale_row(&src, n(3), n(1) as u8).unwrap(),
                    values(f[5]),
                    "superres line {}",
                    line_no + 1
                );
                counts[0] += 1;
            }
            "C" => {
                let mut seed = n(8) as u32;
                let image = (0..256)
                    .map(|_| {
                        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        ((128 + (seed % 25) as i32 - 12) << (n(1) - 8)) as u16
                    })
                    .collect();
                let src = Plane::new(16, 16, 16, image).unwrap();
                let kernel = CdefKernel {
                    bit_depth: n(1) as u8,
                    direction: n(2) as u8,
                    primary: n(3) as u16,
                    secondary: n(4) as u16,
                    damping: n(5) as u8,
                };
                assert_eq!(
                    kernel.apply(&src, (n(6), n(7)), (8, 8)).unwrap(),
                    values(f[9]),
                    "CDEF line {}",
                    line_no + 1
                );
                let mut block = [0u16; 64];
                for y in 0..8 {
                    block[y * 8..y * 8 + 8]
                        .copy_from_slice(&src.row(n(7) + y).unwrap()[n(6)..n(6) + 8]);
                }
                assert_eq!(
                    codec_av1::frame::filters::cdef_find_direction_8x8(&block, 8, n(1) as u8),
                    (n(10) as u8, n(11) as u32),
                    "direction/variance line {}",
                    line_no + 1
                );
                counts[1] += 1;
            }
            "W" | "G" => {
                use codec_av1::frame::restoration::{
                    RestorationSource, SgrprojParameters, WienerCoefficients,
                };
                let mut seed = n(3) as u32;
                let mut pre = Vec::new();
                let mut post = Vec::new();
                let mask = (1u32 << n(1)) - 1;
                for _ in 0..256 {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    pre.push((seed & mask) as u16);
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    post.push((seed & mask) as u16);
                }
                let pre = Plane::new(16, 16, 16, pre).unwrap();
                let post = Plane::new(16, 16, 16, post).unwrap();
                let (stripe, origin) = match n(2) {
                    0 => ((0, 16), (4, 4)),
                    1 => ((4, 12), (4, 4)),
                    _ => ((0, 8), (0, 0)),
                };
                let source = RestorationSource::new(&pre, &post, stripe, n(1) as u8).unwrap();
                let (out, expected) = if f[0] == "W" {
                    let coeff = |s: &str| {
                        s.split(',')
                            .map(|v| v.parse::<i16>().unwrap())
                            .collect::<Vec<_>>()
                            .try_into()
                            .unwrap()
                    };
                    counts[2] += 1;
                    (
                        WienerCoefficients {
                            horizontal: coeff(f[4]),
                            vertical: coeff(f[5]),
                        }
                        .apply(&source, origin, (8, 8))
                        .unwrap(),
                        values(f[6]),
                    )
                } else {
                    counts[3] += 1;
                    (
                        SgrprojParameters {
                            set: n(4) as u8,
                            xqd: [f[5].parse().unwrap(), f[6].parse().unwrap()],
                        }
                        .apply(&source, origin, (8, 8))
                        .unwrap(),
                        values(f[7]),
                    )
                };
                assert_eq!(out, expected, "restoration line {}", line_no + 1);
            }
            _ => panic!("unknown fixture"),
        }
    }
    assert_eq!(counts, [1632, 3072, 288, 576]);
}
