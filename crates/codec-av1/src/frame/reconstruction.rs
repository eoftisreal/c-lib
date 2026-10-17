//! Scalar reconstructed pixel addition for a single coded block.
//!
//! Coefficient entropy decoding and inverse transforms must supply the residuals;
//! this function never fabricates residuals from a compressed bitstream.

use codec_core::{CodecError, CodecResult};

/// Combine compact block prediction and residuals into a strided pixel destination.
/// Reject invalid inputs before modifying any output sample.
pub fn reconstruct_block(
    prediction: &[u16],
    residual: &[i32],
    dst: &mut [u16],
    stride: usize,
    width: usize,
    height: usize,
    bit_depth: u8,
) -> CodecResult<()> {
    if !matches!(bit_depth, 8 | 10 | 12)
        || width == 0
        || height == 0
        || width > 128
        || height > 128
        || stride < width
    {
        return Err(CodecError::InvalidData);
    }
    let count = width.checked_mul(height).ok_or(CodecError::InvalidData)?;
    let storage = stride.checked_mul(height).ok_or(CodecError::InvalidData)?;
    if prediction.len() != count || residual.len() != count || dst.len() < storage {
        return Err(CodecError::InvalidData);
    }
    let max = i64::from((1u16 << bit_depth) - 1);
    if prediction.iter().any(|&pixel| i64::from(pixel) > max) {
        return Err(CodecError::InvalidData);
    }
    for y in 0..height {
        for x in 0..width {
            let src = y * width + x;
            let output = i64::from(prediction[src]) + i64::from(residual[src]);
            dst[y * stride + x] = output.clamp(0, max) as u16;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconstruct_8bit_and_10bit_and_keep_padding() {
        let mut out = [777u16; 6];
        reconstruct_block(
            &[0, 100, 200, 255],
            &[-9, 20, 200, -500],
            &mut out,
            3,
            2,
            2,
            8,
        )
        .unwrap();
        assert_eq!(out, [0, 120, 777, 255, 0, 777]);
        reconstruct_block(
            &[1000, 700, 0, 1023],
            &[100, -500, -100, 8],
            &mut out,
            3,
            2,
            2,
            10,
        )
        .unwrap();
        assert_eq!(out, [1023, 200, 777, 0, 1023, 777]);
    }

    #[test]
    fn large_residuals_are_clamped_without_overflow() {
        let mut out = [0u16; 1];
        reconstruct_block(&[4095], &[i32::MAX], &mut out, 1, 1, 1, 12).unwrap();
        assert_eq!(out, [4095]);
        reconstruct_block(&[0], &[i32::MIN], &mut out, 1, 1, 1, 12).unwrap();
        assert_eq!(out, [0]);
    }

    #[test]
    fn invalid_layout_rejected_without_writing() {
        let mut out = [100u16; 4];
        assert!(reconstruct_block(&[0; 4], &[0; 4], &mut out, 1, 2, 2, 8).is_err());
        assert!(reconstruct_block(&[0; 4], &[0; 4], &mut out, 2, 2, 2, 9).is_err());
        assert!(reconstruct_block(&[0; 3], &[0; 4], &mut out, 2, 2, 2, 8).is_err());
        assert!(reconstruct_block(&[256; 4], &[0; 4], &mut out, 2, 2, 2, 8).is_err());
        assert_eq!(out, [100; 4]);
    }
}
