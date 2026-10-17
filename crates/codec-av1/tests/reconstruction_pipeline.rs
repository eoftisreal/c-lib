use codec_av1::frame::{
    inverse::inverse_1d,
    picture::{PictureReconstruction, TransformBlock},
    quantization::{quantizer_steps, Quantization, QuantizerSteps, Segmentation},
    residual::dequantize_block,
    transform::{inverse_transform, inverse_wht_4x4_checked, TxSize, TxType2D},
};
use codec_core::{ChromaSubsampling, CodecError, PlaneId};
#[test]
fn qindex_segments_deltas_and_clipping() {
    let q = Quantization {
        base_q_idx: 100,
        deltas: [-3, 2, -4, 5, -6],
        qm: None,
    };
    let mut s = Segmentation::default();
    s.features[2][0] = Some(-10);
    assert_eq!(
        q.steps(&s, 2, 0, 8, None).unwrap(),
        quantizer_steps(97, 100, 8).unwrap()
    );
    s.enabled = true;
    assert_eq!(
        q.steps(&s, 2, 0, 8, None).unwrap(),
        quantizer_steps(87, 90, 8).unwrap()
    );
    assert_eq!(
        q.steps(&s, 2, 1, 10, Some(120)).unwrap(),
        quantizer_steps(112, 106, 10).unwrap()
    );
    assert_eq!(
        q.steps(&s, 2, 2, 12, Some(120)).unwrap(),
        quantizer_steps(115, 104, 12).unwrap()
    );
    assert_eq!(
        quantizer_steps(i32::MIN, i32::MAX, 12).unwrap(),
        QuantizerSteps { dc: 4, ac: 29247 }
    );
    assert!(q.steps(&s, 8, 0, 8, None).is_err());
    assert!(q.steps(&s, 0, 3, 8, None).is_err());
    let q = Quantization {
        base_q_idx: 0,
        deltas: [0; 5],
        qm: None,
    };
    s.features[0][0] = Some(i32::MAX);
    assert!(!s.coded_lossless(&q));
    s.enabled = false;
    assert!(s.coded_lossless(&q));
}
#[test]
fn dequantization_rounding_sign_matrix_and_mask() {
    let mut l = vec![0; 512];
    l[0] = 3;
    l[1] = -3;
    let q = QuantizerSteps { dc: 5, ac: 7 };
    assert_eq!(
        &dequantize_block(&l, TxSize::Tx16x32, q, 8, None).unwrap()[..3],
        &[7, -10, 0]
    );
    assert_eq!(
        &dequantize_block(&l, TxSize::Tx16x32, q, 8, Some(&vec![48; 512])).unwrap()[..3],
        &[12, -16, 0]
    );
    let mut l = vec![0; 16];
    l[0] = i32::MAX;
    l[1] = i32::MIN;
    assert_eq!(
        &dequantize_block(&l, TxSize::Tx4x4, QuantizerSteps { dc: 1, ac: 1 }, 8, None).unwrap()
            [..3],
        &[32767, 0, 0]
    );
    assert!(dequantize_block(&l[..15], TxSize::Tx4x4, q, 8, None).is_err());
    assert!(dequantize_block(&l, TxSize::Tx4x4, q, 8, Some(&[0; 16])).is_err());
}
fn block<'a>(p: &'a [u16], c: &'a [i32]) -> TransformBlock<'a> {
    TransformBlock {
        prediction: p,
        coefficients: c,
        size: TxSize::Tx4x4,
        kind: TxType2D::DctDct,
        lossless: true,
    }
}
#[test]
fn picture_assembly_odd_edges_all_depths_and_subsampling() {
    for bd in [8, 10, 12] {
        for sub in [
            ChromaSubsampling::Cs420,
            ChromaSubsampling::Cs422,
            ChromaSubsampling::Cs444,
        ] {
            let mut picture = PictureReconstruction::new(7, 5, bd, sub).unwrap();
            let pred = [1u16 << (bd - 1); 16];
            let mut coeff = [0; 16];
            coeff[0] = 16;
            for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
                let (w, h) = if plane == PlaneId::Y {
                    (7, 5)
                } else {
                    match sub {
                        ChromaSubsampling::Cs420 => (4, 3),
                        ChromaSubsampling::Cs422 => (4, 5),
                        _ => (7, 5),
                    }
                };
                for y in (0..h).step_by(4) {
                    for x in (0..w).step_by(4) {
                        assert_eq!(picture.sample(plane, x, y), None);
                        picture
                            .write_transform(plane, (x, y), block(&pred, &coeff))
                            .unwrap();
                        assert_eq!(picture.sample(plane, x, y), Some(pred[0] + 1));
                        assert!(picture
                            .write_transform(plane, (x, y), block(&pred, &coeff))
                            .is_err());
                    }
                }
            }
            let f = picture.finish().unwrap();
            for p in f.planes() {
                for y in 0..p.height() {
                    assert!(p.row(y).unwrap().iter().all(|&v| v == pred[0] + 1));
                }
            }
        }
    }
}
#[test]
fn failed_blocks_and_incomplete_pictures_never_publish() {
    let mut p = PictureReconstruction::new(4, 4, 8, ChromaSubsampling::Cs420).unwrap();
    assert!(p
        .write_transform(PlaneId::Y, (0, 0), block(&[256; 16], &[0; 16]))
        .is_err());
    assert!(p
        .write_transform(PlaneId::Y, (usize::MAX, 0), block(&[128; 16], &[0; 16]))
        .is_err());
    assert!(p
        .write_transform(PlaneId::Y, (0, 0), block(&[128; 16], &[i32::MAX; 16]))
        .is_err());
    assert_eq!(p.sample(PlaneId::Y, 0, 0), None);
    p.write_transform(PlaneId::Y, (0, 0), block(&[128; 16], &[0; 16]))
        .unwrap();
    assert_eq!(p.finish(), Err(CodecError::NeedMoreData));
    assert!(PictureReconstruction::new(65536, 65536, 8, ChromaSubsampling::Cs444).is_err());
}
#[test]
fn malformed_transform_inputs_never_panic() {
    use codec_av1::frame::transform::TxType1D;
    assert!(inverse_1d(&[i32::MIN; 4], TxType1D::Dct, 16).is_err());
    assert!(inverse_1d(&[0; 3], TxType1D::Dct, 16).is_err());
    assert!(inverse_1d(&[0; 32], TxType1D::Adst, 16).is_err());
    assert!(inverse_wht_4x4_checked(&[i32::MAX; 16], 12).is_err());
    assert!(inverse_transform(&[0; 16], TxSize::Tx4x4, TxType2D::AdstAdst, 8, true).is_err());
    let mut seed = 0x482192efu32;
    for bd in [8u8, 10, 12] {
        for size in [TxSize::Tx4x4, TxSize::Tx8x16, TxSize::Tx64x64] {
            let (w, h) = size.dimensions();
            for _ in 0..128 {
                let c: Vec<_> = (0..w.min(32) * h.min(32))
                    .map(|_| {
                        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        let limit = 1i32 << (bd + 7);
                        (seed % (2 * limit) as u32) as i32 - limit
                    })
                    .collect();
                let _ = inverse_transform(&c, size, TxType2D::DctDct, bd, false);
            }
        }
    }
}

#[test]
fn rectangular_and_square_lossy_block_reconstruction() {
    for size in [
        TxSize::Tx4x8,
        TxSize::Tx8x4,
        TxSize::Tx8x8,
        TxSize::Tx8x16,
        TxSize::Tx16x8,
        TxSize::Tx16x16,
        TxSize::Tx16x32,
        TxSize::Tx32x16,
        TxSize::Tx32x32,
    ] {
        let (w, h) = size.dimensions();
        let mut picture = PictureReconstruction::new(w, h, 8, ChromaSubsampling::Cs444).unwrap();
        let pred = vec![128u16; w * h];
        let mut coeffs = vec![0i32; w.min(32) * h.min(32)];
        coeffs[0] = 32;
        let dequant =
            dequantize_block(&coeffs, size, QuantizerSteps { dc: 4, ac: 4 }, 8, None).unwrap();
        let block = TransformBlock {
            prediction: &pred,
            coefficients: &dequant,
            size,
            kind: TxType2D::DctDct,
            lossless: false,
        };
        picture.write_transform(PlaneId::Y, (0, 0), block).unwrap();
        picture.write_transform(PlaneId::U, (0, 0), block).unwrap();
        picture.write_transform(PlaneId::V, (0, 0), block).unwrap();
        let frame = picture.finish().unwrap();
        assert_eq!(frame.width(), w);
        assert_eq!(frame.height(), h);
        for p in frame.planes() {
            let sample = p.row(0).unwrap()[0];
            assert!(sample >= 128);
        }
    }
}
