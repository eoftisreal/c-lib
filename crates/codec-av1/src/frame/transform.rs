//! AV1 16 separable 2D inverse transforms (`DCT`, `ADST`, `FlipADST`, `IDTX`)
//! and lossless 4x4 Walsh-Hadamard Transform (`WHT`).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxType1D {
    Dct,
    Adst,
    FlipAdst,
    Idtx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxType2D {
    DctDct = 0,
    AdstDct = 1,
    DctAdst = 2,
    AdstAdst = 3,
    FlipAdstDct = 4,
    DctFlipAdst = 5,
    FlipAdstFlipAdst = 6,
    AdstFlipAdst = 7,
    FlipAdstAdst = 8,
    Idtx = 9,
    VIdtx = 10,
    HIdtx = 11,
    AdstIdtx = 12,
    IdtxAdst = 13,
    FlipAdstIdtx = 14,
    IdtxFlipAdst = 15,
}

impl TxType2D {
    pub const fn split_1d(self) -> (TxType1D, TxType1D) {
        match self {
            Self::DctDct => (TxType1D::Dct, TxType1D::Dct),
            Self::AdstDct => (TxType1D::Adst, TxType1D::Dct),
            Self::DctAdst => (TxType1D::Dct, TxType1D::Adst),
            Self::AdstAdst => (TxType1D::Adst, TxType1D::Adst),
            Self::FlipAdstDct => (TxType1D::FlipAdst, TxType1D::Dct),
            Self::DctFlipAdst => (TxType1D::Dct, TxType1D::FlipAdst),
            Self::FlipAdstFlipAdst => (TxType1D::FlipAdst, TxType1D::FlipAdst),
            Self::AdstFlipAdst => (TxType1D::Adst, TxType1D::FlipAdst),
            Self::FlipAdstAdst => (TxType1D::FlipAdst, TxType1D::Adst),
            Self::Idtx => (TxType1D::Idtx, TxType1D::Idtx),
            Self::VIdtx => (TxType1D::Dct, TxType1D::Idtx),
            Self::HIdtx => (TxType1D::Idtx, TxType1D::Dct),
            Self::AdstIdtx => (TxType1D::Adst, TxType1D::Idtx),
            Self::IdtxAdst => (TxType1D::Idtx, TxType1D::Adst),
            Self::FlipAdstIdtx => (TxType1D::FlipAdst, TxType1D::Idtx),
            Self::IdtxFlipAdst => (TxType1D::Idtx, TxType1D::FlipAdst),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxSize {
    Tx4x4,
    Tx8x8,
    Tx16x16,
    Tx32x32,
    Tx64x64,
    Tx4x8,
    Tx8x4,
    Tx8x16,
    Tx16x8,
    Tx16x32,
    Tx32x16,
    Tx32x64,
    Tx64x32,
    Tx4x16,
    Tx16x4,
    Tx8x32,
    Tx32x8,
    Tx16x64,
    Tx64x16,
}

/// Lossless 4x4 inverse Walsh-Hadamard transform for validated dequantized input.
/// Coefficients must be within AV1's 12-bit maximum dequantization range
/// [-524288, 524287]. For untrusted input use `inverse_wht_4x4_checked`.
/// This primitive does not validate lossless bitstream residual conformance.
pub fn inverse_wht_4x4(coeffs: &[i32; 16]) -> [i32; 16] {
    wht_4x4(coeffs, None)
}

fn wht_4x4(coeffs: &[i32; 16], bit_depth: Option<u8>) -> [i32; 16] {
    let mut tmp = [0i32; 16];
    for r in 0..4 {
        let mut a = coeffs[r * 4] >> 2;
        let mut c = coeffs[r * 4 + 1] >> 2;
        let mut d = coeffs[r * 4 + 2] >> 2;
        let mut b = coeffs[r * 4 + 3] >> 2;
        a += c;
        d -= b;
        let e = (a - d) >> 1;
        b = e - b;
        c = e - c;
        a -= b;
        d += c;
        tmp[r * 4] = a;
        tmp[r * 4 + 1] = b;
        tmp[r * 4 + 2] = c;
        tmp[r * 4 + 3] = d;
    }
    if let Some(depth) = bit_depth {
        let limit = 1 << (u32::from(depth + 6).max(16) - 1);
        for v in &mut tmp {
            *v = (*v).clamp(-limit, limit - 1);
        }
    }
    let mut out = [0i32; 16];
    for col in 0..4 {
        let mut a = tmp[col];
        let mut c = tmp[4 + col];
        let mut d = tmp[8 + col];
        let mut b = tmp[12 + col];
        a += c;
        d -= b;
        let e = (a - d) >> 1;
        b = e - b;
        c = e - c;
        a -= b;
        d += c;
        out[col] = a;
        out[4 + col] = b;
        out[8 + col] = c;
        out[12 + col] = d;
    }
    out
}

/// Validate dequantization bounds, apply normative inter-pass clipping, and reject
/// nonconforming lossless residuals before returning a block. No destination is
/// modified on failure. AV1 lossless blocks require TX_4X4 and DCT_DCT syntax.
pub fn inverse_wht_4x4_checked(
    coeffs: &[i32; 16],
    bit_depth: u8,
) -> codec_core::CodecResult<[i32; 16]> {
    use codec_core::CodecError;
    if !matches!(bit_depth, 8 | 10 | 12) {
        return Err(CodecError::InvalidData);
    }
    let limit = 1 << (7 + bit_depth);
    if coeffs.iter().any(|&v| v < -limit || v >= limit) {
        return Err(CodecError::InvalidData);
    }
    let out = wht_4x4(coeffs, Some(bit_depth));
    let pixel_limit = 1 << bit_depth;
    if out.iter().any(|&v| v < -pixel_limit || v >= pixel_limit) {
        return Err(CodecError::InvalidData);
    }
    Ok(out)
}

impl TxSize {
    pub const fn dimensions(self) -> (usize, usize) {
        use TxSize::*;
        match self {
            Tx4x4 => (4, 4),
            Tx8x8 => (8, 8),
            Tx16x16 => (16, 16),
            Tx32x32 => (32, 32),
            Tx64x64 => (64, 64),
            Tx4x8 => (4, 8),
            Tx8x4 => (8, 4),
            Tx8x16 => (8, 16),
            Tx16x8 => (16, 8),
            Tx16x32 => (16, 32),
            Tx32x16 => (32, 16),
            Tx32x64 => (32, 64),
            Tx64x32 => (64, 32),
            Tx4x16 => (4, 16),
            Tx16x4 => (16, 4),
            Tx8x32 => (8, 32),
            Tx32x8 => (32, 8),
            Tx16x64 => (16, 64),
            Tx64x16 => (64, 16),
        }
    }
    pub const fn row_shift(self) -> u8 {
        use TxSize::*;
        match self {
            Tx4x4 | Tx4x8 | Tx8x4 => 0,
            Tx8x8 | Tx8x16 | Tx16x8 | Tx16x32 | Tx32x16 | Tx32x64 | Tx64x32 | Tx4x16 | Tx16x4 => 1,
            _ => 2,
        }
    }
}

/// Convert a compact dequantized coefficient array into spatial residuals.
/// Coefficients use specification raster order with width/height capped at 32.
/// Includes rectangular prescaling, staged transforms, row/column shifts,
/// inter-pass clipping, and FLIPADST spatial placement. Block syntax must also
/// validate the allowed transform set for the frame, prediction mode and segment.
pub fn inverse_transform(
    coeffs: &[i32],
    size: TxSize,
    kind: TxType2D,
    bit_depth: u8,
    lossless: bool,
) -> codec_core::CodecResult<Vec<i32>> {
    use super::inverse::inverse_1d;
    use codec_core::CodecError;
    let (w, h) = size.dimensions();
    let (cw, ch) = (w.min(32), h.min(32));
    if !matches!(bit_depth, 8 | 10 | 12) || coeffs.len() != cw * ch {
        return Err(CodecError::InvalidData);
    }
    let limit = 1 << (bit_depth + 7);
    if coeffs.iter().any(|&v| v < -limit || v >= limit) {
        return Err(CodecError::InvalidData);
    }
    if lossless {
        if size != TxSize::Tx4x4 || kind != TxType2D::DctDct {
            return Err(CodecError::InvalidData);
        }
        return Ok(inverse_wht_4x4_checked(
            coeffs.try_into().map_err(|_| CodecError::InvalidData)?,
            bit_depth,
        )?
        .to_vec());
    }
    if (w.max(h) == 64 && kind != TxType2D::DctDct)
        || (w.max(h) == 32 && !matches!(kind, TxType2D::DctDct | TxType2D::Idtx))
    {
        return Err(CodecError::Unsupported);
    }
    let round = |v: i64, shift: u8| {
        if shift == 0 {
            v as i32
        } else {
            ((v + (1 << (shift - 1))) >> shift) as i32
        }
    };
    let (vertical, horizontal) = kind.split_1d();
    let row_bits = bit_depth + 8;
    let col_bits = (bit_depth + 6).max(16);
    let clamp_limit = 1 << (col_bits - 1);
    let mut residual = vec![0; w * h];
    let rect = w.ilog2().abs_diff(h.ilog2()) == 1;
    for y in 0..h {
        let mut row = vec![0; w];
        if y < ch {
            for x in 0..cw {
                row[x] = if rect {
                    round(i64::from(coeffs[y * cw + x]) * 2896, 12)
                } else {
                    coeffs[y * cw + x]
                };
            }
        }
        let transformed = inverse_1d(&row, horizontal, row_bits)?;
        for x in 0..w {
            residual[y * w + x] = round(i64::from(transformed[x]), size.row_shift())
                .clamp(-clamp_limit, clamp_limit - 1);
        }
    }
    let mut out = vec![0; w * h];
    for x in 0..w {
        let col: Vec<_> = (0..h).map(|y| residual[y * w + x]).collect();
        let transformed = inverse_1d(&col, vertical, col_bits)?;
        for (y, &v) in transformed.iter().enumerate() {
            let xx = if horizontal == TxType1D::FlipAdst {
                w - x - 1
            } else {
                x
            };
            let yy = if vertical == TxType1D::FlipAdst {
                h - y - 1
            } else {
                y
            };
            out[yy * w + xx] = round(i64::from(v), 4);
        }
    }
    Ok(out)
}
