use codec_av1::frame::{
    intra::{predict_basic_block, IntraMode},
    inverse::inverse_1d,
    quantization::quantizer_steps,
    transform::{
        inverse_transform, inverse_wht_4x4, inverse_wht_4x4_checked, TxSize, TxType1D, TxType2D,
    },
};
fn nums(s: &str) -> Vec<i32> {
    s.split(',').map(|v| v.parse().unwrap()).collect()
}
#[test]
fn independently_compiled_libaom_scalar_known_answers() {
    let mut counts = [0; 7];
    for (line_number, line) in include_str!("../../../tests/oracles/phase1_vectors.txt")
        .lines()
        .enumerate()
    {
        let f: Vec<_> = line.split_whitespace().collect();
        let n = |i: usize| f[i].parse::<usize>().unwrap();
        match f[0] {
            "T" => {
                let kind = [TxType1D::Dct, TxType1D::Adst, TxType1D::Idtx][n(1)];
                let input = nums(f[4]);
                assert_eq!(input.len(), n(2));
                assert_eq!(
                    inverse_1d(&input, kind, n(3) as u8).unwrap(),
                    nums(f[5]),
                    "line {}",
                    line_number + 1
                );
                if kind == TxType1D::Adst {
                    assert_eq!(
                        inverse_1d(&input, TxType1D::FlipAdst, n(3) as u8).unwrap(),
                        nums(f[5])
                    );
                }
                counts[0] += 1;
            }
            "Q" => {
                let steps = quantizer_steps(n(2) as i32, n(2) as i32, n(1) as u8).unwrap();
                assert_eq!(
                    (steps.dc as usize, steps.ac as usize),
                    (n(3), n(4)),
                    "line {}",
                    line_number + 1
                );
                counts[1] += 1;
            }
            "S" => {
                let (w, h, bd) = (n(2), n(3), n(4) as u8);
                let above: Vec<_> = nums(f[5]).iter().map(|&v| v as u16).collect();
                let left: Vec<_> = nums(f[6]).iter().map(|&v| v as u16).collect();
                let mut dst = vec![65535; (w + 3) * h];
                let mode = [
                    IntraMode::SmoothPred,
                    IntraMode::SmoothVPred,
                    IntraMode::SmoothHPred,
                ][n(1)];
                predict_basic_block(
                    mode,
                    &mut dst,
                    w + 3,
                    w,
                    h,
                    bd,
                    Some(&above),
                    Some(&left),
                    None,
                )
                .unwrap();
                let output: Vec<_> = dst
                    .chunks_exact(w + 3)
                    .flat_map(|row| row[..w].iter().map(|&v| i32::from(v)))
                    .collect();
                assert_eq!(output, nums(f[7]), "line {}", line_number + 1);
                assert!(dst.chunks_exact(w + 3).all(|row| row[w..] == [65535; 3]));
                counts[2] += 1;
            }
            "W" => {
                let input: [i32; 16] = nums(f[2]).try_into().unwrap();
                let expected = nums(f[3]);
                assert_eq!(
                    inverse_wht_4x4(&input).as_slice(),
                    expected,
                    "unchecked WHT line {}",
                    line_number + 1
                );
                assert_eq!(
                    inverse_wht_4x4_checked(&input, n(1) as u8)
                        .unwrap()
                        .as_slice(),
                    expected,
                    "checked WHT line {}",
                    line_number + 1
                );
                counts[3] += 1;
            }
            "D" => {
                use TxSize::*;
                use TxType2D::*;
                let size = [
                    Tx4x4, Tx8x8, Tx16x16, Tx32x32, Tx64x64, Tx4x8, Tx8x4, Tx8x16, Tx16x8, Tx16x32,
                    Tx32x16, Tx32x64, Tx64x32, Tx4x16, Tx16x4, Tx8x32, Tx32x8, Tx16x64, Tx64x16,
                ][n(1)];
                let kind = [
                    DctDct,
                    AdstDct,
                    DctAdst,
                    AdstAdst,
                    FlipAdstDct,
                    DctFlipAdst,
                    FlipAdstFlipAdst,
                    AdstFlipAdst,
                    FlipAdstAdst,
                    Idtx,
                    VIdtx,
                    HIdtx,
                    AdstIdtx,
                    IdtxAdst,
                    FlipAdstIdtx,
                    IdtxFlipAdst,
                ][n(2)];
                let (w, h) = size.dimensions();
                let (cw, ch) = (w.min(32), h.min(32));
                let bd = n(3) as u8;
                let pattern = n(4);
                let mut seed = n(5) as u32;
                let input: Vec<_> = (0..cw * ch)
                    .map(|i| {
                        let value = match pattern {
                            0 => 0,
                            1 => {
                                if i == 0 {
                                    64
                                } else {
                                    0
                                }
                            }
                            2 => {
                                if i == cw + 1 {
                                    -32
                                } else {
                                    0
                                }
                            }
                            3 => {
                                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                                ((seed >> 16) % 65) as i32 - 32
                            }
                            _ => {
                                if i & 1 != 0 {
                                    -64
                                } else {
                                    64
                                }
                            }
                        };
                        value * (1i32 << (bd - 8))
                    })
                    .collect();
                let residual = inverse_transform(&input, size, kind, bd, false).unwrap();
                let mut out = vec![0; w * h];
                codec_av1::frame::reconstruction::reconstruct_block(
                    &vec![1u16 << (bd - 1); w * h],
                    &residual,
                    &mut out,
                    w,
                    w,
                    h,
                    bd,
                )
                .unwrap();
                let expected = nums(f[6]);
                assert_eq!(
                    out.iter().map(|&v| i32::from(v)).collect::<Vec<_>>(),
                    expected,
                    "2D line {} size={size:?} kind={kind:?}",
                    line_number + 1
                );
                counts[4] += 1;
            }
            "C" => {
                use TxSize::*;
                use TxType2D::*;
                let size = [
                    Tx4x4, Tx8x8, Tx16x16, Tx32x32, Tx64x64, Tx4x8, Tx8x4, Tx8x16, Tx16x8, Tx16x32,
                    Tx32x16, Tx32x64, Tx64x32, Tx4x16, Tx16x4, Tx8x32, Tx32x8, Tx16x64, Tx64x16,
                ][n(1)];
                let kind = [
                    DctDct,
                    AdstDct,
                    DctAdst,
                    AdstAdst,
                    FlipAdstDct,
                    DctFlipAdst,
                    FlipAdstFlipAdst,
                    AdstFlipAdst,
                    FlipAdstAdst,
                    Idtx,
                    VIdtx,
                    HIdtx,
                    AdstIdtx,
                    IdtxAdst,
                    FlipAdstIdtx,
                    IdtxFlipAdst,
                ][n(2)];
                let scan = codec_av1::frame::scan::scan_order(size, kind).unwrap();
                assert_eq!(
                    scan.iter().map(|&v| i32::from(v)).collect::<Vec<_>>(),
                    nums(f[3])
                );
                let mut sorted = scan.to_vec();
                sorted.sort_unstable();
                assert!(sorted.iter().enumerate().all(|(i, &v)| i == usize::from(v)));
                counts[6] += 1;
            }
            "P" => {
                use codec_av1::{
                    entropy::SymbolDecoder,
                    frame::{
                        partition::{BlockSize, PartitionCdfs},
                        partition_decode::{BlockGeometry, PartitionRequest},
                    },
                };
                let width = n(1) as u32;
                let edge = n(3);
                let data: Vec<_> = f[5]
                    .as_bytes()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
                    .collect();
                let mut decoder = SymbolDecoder::new(&data, n(4) == 0).unwrap();
                let mut cdfs = PartitionCdfs::default();
                let initial = cdfs.clone();
                let request = PartitionRequest {
                    block: BlockGeometry {
                        mi_row: 0,
                        mi_col: 0,
                        bsize: BlockSize::from_dimensions_px(width, width).unwrap(),
                    },
                    context: n(2),
                    has_rows: edge != 1 && edge != 3,
                    has_cols: edge != 2 && edge != 3,
                };
                for symbol in nums(f[6]) {
                    assert_eq!(
                        cdfs.read_partition(&mut decoder, request).unwrap() as i32,
                        symbol,
                        "partition line {}",
                        line_number + 1
                    );
                }
                if edge != 0 || n(4) == 0 {
                    assert_eq!(cdfs, initial);
                } else {
                    assert_ne!(cdfs, initial);
                }
                assert_eq!(decoder.read_literal(8).unwrap(), 0xa5);
                decoder.finish().unwrap();
                counts[5] += 1;
            }
            _ => panic!("unknown fixture tag"),
        }
    }
    assert!(counts[0] >= 2400);
    assert_eq!(counts[1], 768);
    assert_eq!(counts[2], 171);
    assert_eq!(counts[3], 192);
    assert_eq!(counts[4], 2385);
    assert_eq!(counts[5], 144);
    assert_eq!(counts[6], 159);
}
