//! Assembly of supplied real transform blocks; not a compressed AV1 parser.
//! Private coverage masks prevent publishing any unwritten active pixels.
use super::transform::{inverse_transform, TxSize, TxType2D};
use codec_core::{ChromaSubsampling, CodecError, CodecResult, PlanarFrame, Plane, PlaneId};
#[derive(Clone, Copy)]
pub struct TransformBlock<'a> {
    pub prediction: &'a [u16],
    pub coefficients: &'a [i32],
    pub size: TxSize,
    pub kind: TxType2D,
    pub lossless: bool,
}
pub struct PictureReconstruction {
    planes: [Plane; 3],
    written: [Vec<bool>; 3],
    remaining: usize,
    bit_depth: u8,
    subsampling: ChromaSubsampling,
}
fn index(p: PlaneId) -> usize {
    match p {
        PlaneId::Y => 0,
        PlaneId::U => 1,
        PlaneId::V => 2,
    }
}
impl PictureReconstruction {
    pub fn new(
        width: usize,
        height: usize,
        bit_depth: u8,
        subsampling: ChromaSubsampling,
    ) -> CodecResult<Self> {
        if width == 0
            || height == 0
            || width > 65536
            || height > 65536
            || !matches!(bit_depth, 8 | 10 | 12)
        {
            return Err(CodecError::InvalidData);
        }
        let (sx, sy) = match subsampling {
            ChromaSubsampling::Cs420 => (1, 1),
            ChromaSubsampling::Cs422 => (1, 0),
            ChromaSubsampling::Cs444 => (0, 0),
            _ => return Err(CodecError::Unsupported),
        };
        let (cw, ch) = (width.div_ceil(1 << sx), height.div_ceil(1 << sy));
        let total = width
            .checked_mul(height)
            .and_then(|y| {
                cw.checked_mul(ch)
                    .and_then(|c| c.checked_mul(2))
                    .and_then(|c| y.checked_add(c))
            })
            .ok_or(CodecError::InvalidData)?;
        // At most 384 MiB for u16 planes plus byte-per-sample coverage masks.
        if total > 128 * 1024 * 1024 {
            return Err(CodecError::InvalidData);
        }
        fn allocate(w: usize, h: usize) -> CodecResult<(Plane, Vec<bool>)> {
            let mut pixels = Vec::new();
            pixels
                .try_reserve_exact(w * h)
                .map_err(|_| CodecError::OutOfMemory)?;
            pixels.resize(w * h, 0);
            let mut mask = Vec::new();
            mask.try_reserve_exact(w * h)
                .map_err(|_| CodecError::OutOfMemory)?;
            mask.resize(w * h, false);
            Ok((Plane::new(w, h, w, pixels)?, mask))
        }
        let (y, ym) = allocate(width, height)?;
        let (u, um) = allocate(cw, ch)?;
        let (v, vm) = allocate(cw, ch)?;
        Ok(Self {
            planes: [y, u, v],
            written: [ym, um, vm],
            remaining: total,
            bit_depth,
            subsampling,
        })
    }
    pub fn sample(&self, plane: PlaneId, x: usize, y: usize) -> Option<u16> {
        let p = index(plane);
        let image = &self.planes[p];
        if x >= image.width() || y >= image.height() || !self.written[p][y * image.width() + x] {
            return None;
        }
        image.row(y).map(|row| row[x])
    }
    /// Commit a validated transform atomically, clipping writes at active edges.
    /// Prediction and coefficients retain the full coded transform geometry.
    pub fn write_transform(
        &mut self,
        plane: PlaneId,
        origin: (usize, usize),
        block: TransformBlock<'_>,
    ) -> CodecResult<()> {
        let p = index(plane);
        let (x, y) = origin;
        let image = &self.planes[p];
        let (w, h) = block.size.dimensions();
        if x >= image.width()
            || y >= image.height()
            || block.prediction.len() != w * h
            || block
                .prediction
                .iter()
                .any(|&v| v >= (1u16 << self.bit_depth))
        {
            return Err(CodecError::InvalidData);
        }
        let (aw, ah) = (w.min(image.width() - x), h.min(image.height() - y));
        for r in y..y + ah {
            if self.written[p][r * image.width() + x..r * image.width() + x + aw]
                .iter()
                .any(|&v| v)
            {
                return Err(CodecError::InvalidData);
            }
        }
        let residual = inverse_transform(
            block.coefficients,
            block.size,
            block.kind,
            self.bit_depth,
            block.lossless,
        )?;
        let max = (1i64 << self.bit_depth) - 1;
        let width = self.planes[p].width();
        for r in 0..ah {
            let dst = self.planes[p]
                .row_mut(y + r)
                .ok_or(CodecError::InvalidData)?;
            for c in 0..aw {
                let i = r * w + c;
                dst[x + c] =
                    (i64::from(block.prediction[i]) + i64::from(residual[i])).clamp(0, max) as u16;
            }
            self.written[p][(y + r) * width + x..(y + r) * width + x + aw].fill(true);
        }
        self.remaining -= aw * ah;
        Ok(())
    }
    /// All active Y/U/V samples must be written. Result is pre-filter pixels;
    /// bitstream parsing, normative scheduling and in-loop filtering are separate.
    pub fn finish(self) -> CodecResult<PlanarFrame> {
        if self.remaining != 0 {
            return Err(CodecError::NeedMoreData);
        }
        let [y, u, v] = self.planes;
        PlanarFrame::new(self.bit_depth, self.subsampling, y, u, v)
    }
}
