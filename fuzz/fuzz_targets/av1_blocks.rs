#![no_main]
use codec_av1::frame::{
    partition_decode::{BlockGeometry, PartitionRequest, PartitionSyntax, PartitionTraversal},
    picture::{PictureReconstruction, TransformBlock},
    quantization::quantizer_steps,
    reconstruction::reconstruct_block,
    residual::dequantize_block,
    scan::scan_order,
    transform::{inverse_transform, TxSize, TxType2D},
    PartitionType,
};
use codec_core::{ChromaSubsampling, CodecError, CodecResult, PlaneId};
use libfuzzer_sys::fuzz_target;

struct Syntax<'a> {
    symbols: std::slice::Iter<'a, u8>,
    leaves: usize,
}
impl PartitionSyntax for Syntax<'_> {
    fn read_partition(&mut self, request: PartitionRequest) -> CodecResult<PartitionType> {
        let symbol = self.symbols.next().ok_or(CodecError::InvalidData)?;
        let kind = [
            PartitionType::None,
            PartitionType::Horizontal,
            PartitionType::Vertical,
            PartitionType::Split,
            PartitionType::HorzA,
            PartitionType::HorzB,
            PartitionType::VertA,
            PartitionType::VertB,
            PartitionType::Horz4,
            PartitionType::Vert4,
        ][usize::from(symbol % 10)];
        if !request.allows(kind) {
            return Err(CodecError::InvalidData);
        }
        Ok(kind)
    }
    fn decode_leaf(&mut self, _: BlockGeometry) -> CodecResult<()> {
        self.leaves += 1;
        assert!(self.leaves <= 1024);
        Ok(())
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 8 {
        return;
    }
    let depth = [8, 10, 12][usize::from(data[0] % 3)];
    let sizes = [
        TxSize::Tx4x4,
        TxSize::Tx8x8,
        TxSize::Tx16x16,
        TxSize::Tx32x32,
        TxSize::Tx64x64,
        TxSize::Tx4x8,
        TxSize::Tx8x4,
        TxSize::Tx16x32,
        TxSize::Tx64x16,
    ];
    let kinds = [
        TxType2D::DctDct,
        TxType2D::AdstDct,
        TxType2D::DctAdst,
        TxType2D::FlipAdstFlipAdst,
        TxType2D::Idtx,
        TxType2D::HIdtx,
    ];
    let size = sizes[usize::from(data[1]) % sizes.len()];
    let kind = kinds[usize::from(data[2]) % kinds.len()];
    let (w, h) = size.dimensions();
    let mut levels = vec![0; w.min(32) * h.min(32)];
    for (v, bytes) in levels.iter_mut().zip(data[8..].as_chunks::<4>().0.iter()) {
        *v = i32::from_le_bytes(*bytes);
    }
    let steps = quantizer_steps(i32::from(data[3]), i32::from(data[4]), depth).unwrap();
    let coefficients = dequantize_block(&levels, size, steps, depth, None).unwrap();
    let _ = scan_order(size, kind);
    let lossless = data[5] & 1 != 0;
    let prediction = vec![u16::from(data[6]); w * h];
    if let Ok(residual) = inverse_transform(&coefficients, size, kind, depth, lossless) {
        let mut dst = vec![0; (w + 1) * h];
        reconstruct_block(&prediction, &residual, &mut dst, w + 1, w, h, depth).unwrap();
        assert!(dst.iter().all(|&p| p < (1 << depth)));
    }
    let mut picture = PictureReconstruction::new(4, 4, depth, ChromaSubsampling::Cs420).unwrap();
    let result = picture.write_transform(
        PlaneId::Y,
        (0, 0),
        TransformBlock {
            prediction: &prediction,
            coefficients: &coefficients,
            size,
            kind,
            lossless,
        },
    );
    if result.is_err() {
        assert_eq!(picture.sample(PlaneId::Y, 0, 0), None);
    }
    // U and V are never written, so partial pictures must never be published.
    assert!(picture.finish().is_err());
    let mut traversal = PartitionTraversal::new(32, 32, (0, 32), (0, 32), true).unwrap();
    let mut syntax = Syntax {
        symbols: data[8..].iter(),
        leaves: 0,
    };
    if traversal.decode_next(&mut syntax).is_err() {
        assert!(traversal.decode_next(&mut syntax).is_err());
    }
});
