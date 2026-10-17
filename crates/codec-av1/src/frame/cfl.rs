//! Scalar Chroma-from-Luma (CfL) prediction for complete, reconstructed blocks.
//!
//! This is a pixel-domain primitive, **not** AV1 compressed block parsing.
//! Edge extension and incomplete frame-edge blocks are intentionally not supported
//! until the normative block-level reconstruction path supplies those samples.

use codec_core::{ChromaSubsampling, CodecError, CodecResult};

/// Parameters for a complete CfL prediction block. Dimensions refer to the
/// **chroma** output plane, not the luma input plane.
#[derive(Debug, Clone, Copy)]
pub struct CflBlock {
    pub width: usize,
    pub height: usize,
    pub luma_stride: usize,
    pub chroma_stride: usize,
    pub subsampling: ChromaSubsampling,
    pub bit_depth: u8,
    /// Signed CfL alpha, normally parsed from the AV1 bitstream.
    pub alpha: i8,
    /// Already reconstructed DC predictor for the target chroma plane.
    pub dc: u16,
}

/// AV1 symmetric fixed-point rounding by 6 bits. Negative ties round
/// away from zero, matching Round2Signed.
fn round2_signed_6(value: i32) -> i32 {
    if value >= 0 {
        (value + 32) >> 6
    } else {
        -((-value + 32) >> 6)
    }
}

/// Create CfL-predicted chroma samples from already-reconstructed luma.
///
/// Only full 4/8/16/32 chroma blocks are supported. Callers must extend
/// boundaries according to AV1 rules before invoking this function; this
/// function rejects truncated input rather than inventing border pixels.
///
/// Validated input errors leave every destination sample unchanged. Output
/// row padding is never modified.
pub fn predict_cfl_block(luma: &[u16], chroma_out: &mut [u16], block: CflBlock) -> CodecResult<()> {
    let CflBlock {
        width,
        height,
        luma_stride,
        chroma_stride,
        subsampling,
        bit_depth,
        alpha,
        dc,
    } = block;
    if !matches!(bit_depth, 8 | 10 | 12)
        || !matches!(width, 4 | 8 | 16 | 32)
        || !matches!(height, 4 | 8 | 16 | 32)
        || !(-16..=16).contains(&alpha)
        || chroma_stride < width
        || u32::from(dc) >= (1u32 << bit_depth)
    {
        return Err(CodecError::InvalidData);
    }
    let (sx, sy) = match subsampling {
        ChromaSubsampling::Cs420 => (1usize, 1usize),
        ChromaSubsampling::Cs422 => (1, 0),
        ChromaSubsampling::Cs444 => (0, 0),
        ChromaSubsampling::Monochrome => return Err(CodecError::Unsupported),
    };
    let lw = width << sx;
    let lh = height << sy;
    if luma_stride < lw
        || luma_stride.checked_mul(lh).is_none_or(|n| n > luma.len())
        || chroma_stride
            .checked_mul(height)
            .is_none_or(|n| n > chroma_out.len())
    {
        return Err(CodecError::InvalidData);
    }
    let max_sample = (1u16 << bit_depth) - 1;
    // Validate *all* active input samples before writing anything.
    for y in 0..lh {
        if luma[y * luma_stride..y * luma_stride + lw]
            .iter()
            .any(|&p| p > max_sample)
        {
            return Err(CodecError::InvalidData);
        }
    }
    let size = width * height; // <= 1024 by input validation
    let mut q3 = vec![0i32; size];
    let mut total_q3 = 0i32;
    let scale = 8i32 >> (sx + sy);
    for cy in 0..height {
        for cx in 0..width {
            let mut sum = 0i32;
            for dy in 0..(1 << sy) {
                for dx in 0..(1 << sx) {
                    sum +=
                        i32::from(luma[(cy * (1 << sy) + dy) * luma_stride + cx * (1 << sx) + dx]);
                }
            }
            let value = sum * scale;
            q3[cy * width + cx] = value;
            total_q3 += value;
        }
    }
    // Average Q3 chroma-grid luma, rounded once after subsampling.
    let avg_q3 = (total_q3 + (size / 2) as i32) / size as i32;
    for cy in 0..height {
        for cx in 0..width {
            let ac_q3 = q3[cy * width + cx] - avg_q3;
            let predicted = i32::from(dc) + round2_signed_6(i32::from(alpha) * ac_q3);
            chroma_out[cy * chroma_stride + cx] = predicted.clamp(0, i32::from(max_sample)) as u16;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(layout: ChromaSubsampling, alpha: i8) -> CflBlock {
        CflBlock {
            width: 4,
            height: 4,
            luma_stride: 8,
            chroma_stride: 5,
            subsampling: layout,
            bit_depth: 8,
            alpha,
            dc: 128,
        }
    }

    #[test]
    fn cfl_444_signed_rounding_and_padding() {
        let mut luma = vec![0u16; 8 * 4];
        for y in 0..4 {
            for x in 0..4 {
                luma[y * 8 + x] = (x * 10) as u16;
            }
        }
        let mut out = vec![999u16; 20];
        predict_cfl_block(&luma, &mut out, cfg(ChromaSubsampling::Cs444, 16)).unwrap();
        for row in out.as_chunks::<5>().0 {
            assert_eq!(row, &[98, 118, 138, 158, 999]);
        }
        predict_cfl_block(&luma, &mut out, cfg(ChromaSubsampling::Cs444, -16)).unwrap();
        for row in out.as_chunks::<5>().0 {
            assert_eq!(row, &[158, 138, 118, 98, 999]);
        }
    }

    #[test]
    fn cfl_420_downsampling_and_dc_only() {
        let mut luma = vec![0u16; 8 * 8];
        for y in 0..8 {
            for x in 0..8 {
                luma[y * 8 + x] = ((x / 2) * 10) as u16;
            }
        }
        let mut out = vec![777u16; 20];
        predict_cfl_block(&luma, &mut out, cfg(ChromaSubsampling::Cs420, 16)).unwrap();
        for row in out.as_chunks::<5>().0 {
            assert_eq!(row, &[98, 118, 138, 158, 777]);
        }
        predict_cfl_block(&luma, &mut out, cfg(ChromaSubsampling::Cs420, 0)).unwrap();
        for row in out.as_chunks::<5>().0 {
            assert_eq!(row, &[128, 128, 128, 128, 777]);
        }
    }

    #[test]
    fn cfl_422_subsampling_and_12_bit_clipping() {
        let mut luma = vec![0u16; 8 * 4];
        for row in luma.as_chunks_mut::<8>().0 {
            row.copy_from_slice(&[0, 0, 100, 100, 200, 200, 300, 300]);
        }
        let mut out = vec![555u16; 20];
        let mut params = cfg(ChromaSubsampling::Cs422, 16);
        params.bit_depth = 12;
        params.dc = 4090;
        predict_cfl_block(&luma, &mut out, params).unwrap();
        for row in out.as_chunks::<5>().0 {
            assert_eq!(row, &[3790, 3990, 4095, 4095, 555]);
        }
    }

    #[test]
    fn invalid_input_rejected_atomically() {
        let mut out = vec![123u16; 20];
        let source = vec![0u16; 64];
        let mut params = cfg(ChromaSubsampling::Cs420, 17);
        assert_eq!(
            predict_cfl_block(&source, &mut out, params),
            Err(CodecError::InvalidData)
        );
        params.alpha = 16;
        assert_eq!(
            predict_cfl_block(&source[..63], &mut out, params),
            Err(CodecError::InvalidData)
        );
        params.width = 5;
        assert_eq!(
            predict_cfl_block(&source, &mut out, params),
            Err(CodecError::InvalidData)
        );
        params.width = 4;
        params.luma_stride = usize::MAX;
        assert_eq!(
            predict_cfl_block(&source, &mut out, params),
            Err(CodecError::InvalidData)
        );
        assert_eq!(out, vec![123u16; 20]);
    }
}
