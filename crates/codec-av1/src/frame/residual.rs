//! Dequantization of already decoded coefficient levels. Entropy-coded EOB/level
//! syntax is not supplied here and is never replaced with synthetic coefficients.
use super::{quantization::QuantizerSteps, transform::TxSize};
use codec_core::{CodecError, CodecResult};

/// Apply AV1 section 7.12.3 coefficient scaling, low-24-bit reduction, signed
/// truncation, transform denominator, and bit-depth clipping. `inverse_matrix`
/// must be the selected normative inverse QM weights, or None when QM is off.
/// Matrix selection (including lossless/identity bypass) belongs to block syntax.
pub fn dequantize_block(
    levels: &[i32],
    size: TxSize,
    steps: QuantizerSteps,
    bit_depth: u8,
    inverse_matrix: Option<&[u8]>,
) -> CodecResult<Vec<i32>> {
    let (w, h) = size.dimensions();
    let len = w.min(32) * h.min(32);
    if !matches!(bit_depth, 8 | 10 | 12)
        || levels.len() != len
        || steps.dc == 0
        || steps.ac == 0
        || inverse_matrix.is_some_and(|m| m.len() != len || m.contains(&0))
    {
        return Err(CodecError::InvalidData);
    }
    let denom = match size {
        TxSize::Tx32x32 | TxSize::Tx16x32 | TxSize::Tx32x16 | TxSize::Tx16x64 | TxSize::Tx64x16 => {
            2
        }
        TxSize::Tx64x64 | TxSize::Tx32x64 | TxSize::Tx64x32 => 4,
        _ => 1,
    };
    let limit = 1i64 << (7 + bit_depth);
    Ok(levels
        .iter()
        .enumerate()
        .map(|(i, &level)| {
            let q = i64::from(if i == 0 { steps.dc } else { steps.ac });
            let q = if let Some(matrix) = inverse_matrix {
                (q * i64::from(matrix[i]) + 16) >> 5
            } else {
                q
            };
            let dq = i64::from(level) * q;
            let magnitude = (dq.abs() & 0xffffff) / denom;
            (dq.signum() * magnitude).clamp(-limit, limit - 1) as i32
        })
        .collect())
}
