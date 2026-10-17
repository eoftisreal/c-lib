//! AV1 section 7.16 scalar 64-phase, eight-tap super-resolution.
use super::superres_tables;
use codec_core::{CodecError, CodecResult, PlanarFrame, Plane};
/// One active row with replicated picture-edge extension. Source/destination
/// widths are plane dimensions, so odd chroma dimensions are handled directly.
pub fn upscale_row(src: &[u16], width: usize, bit_depth: u8) -> CodecResult<Vec<u16>> {
    if !matches!(bit_depth, 8 | 10 | 12)
        || src.is_empty()
        || width < src.len()
        || width > 65536
        || src.len() > 65536
        || width > 2 * src.len()
        || src.iter().any(|&v| v >= (1u16 << bit_depth))
    {
        return Err(CodecError::InvalidData);
    }
    if width == src.len() {
        return Ok(src.to_vec());
    }
    let down = src.len() as i64;
    let up = width as i64;
    let step = ((down << 14) + up / 2) / up;
    let err = up * step - (down << 14);
    // AV1 integer divisions truncate toward zero, unlike signed right shift.
    let initial = ((-(up - down) * (1 << 13) + up / 2) / up + 128 - err / 2) & 16383;
    let max = (1i64 << bit_depth) - 1;
    Ok((0..width)
        .map(|x| {
            let pos = -(1 << 14) + initial + x as i64 * step;
            let base = pos >> 14;
            let phase = ((pos & 16383) >> 8) as usize;
            let mut sum = 0i64;
            for k in 0..8 {
                let ix = (base + k as i64 - 3).clamp(0, down - 1) as usize;
                sum += i64::from(src[ix]) * i64::from(superres_tables::UPSCALE_FILTER[phase][k]);
            }
            ((sum + 64) >> 7).clamp(0, max) as u16
        })
        .collect())
}
pub fn upscale_picture(src: &PlanarFrame, width: usize) -> CodecResult<PlanarFrame> {
    use codec_core::ChromaSubsampling;
    if width < src.width() || width > 65536 || width > 2 * src.width() {
        return Err(CodecError::InvalidData);
    }
    let sx = match src.subsampling() {
        ChromaSubsampling::Cs420 | ChromaSubsampling::Cs422 => 1,
        ChromaSubsampling::Cs444 => 0,
        _ => return Err(CodecError::Unsupported),
    };
    let mut total = 0usize;
    for (i, plane) in src.planes().iter().enumerate() {
        let w = if i == 0 {
            width
        } else {
            width.div_ceil(1 << sx)
        };
        total = total
            .checked_add(
                w.checked_mul(plane.height())
                    .ok_or(CodecError::InvalidData)?,
            )
            .ok_or(CodecError::InvalidData)?;
    }
    if total > 256 * 1024 * 1024 {
        return Err(CodecError::InvalidData);
    }
    let mut planes = Vec::new();
    for (i, plane) in src.planes().iter().enumerate() {
        let w = if i == 0 {
            width
        } else {
            width.div_ceil(1 << sx)
        };
        let count = w
            .checked_mul(plane.height())
            .ok_or(CodecError::InvalidData)?;
        if count > 128 * 1024 * 1024 {
            return Err(CodecError::InvalidData);
        }
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(count)
            .map_err(|_| CodecError::OutOfMemory)?;
        for y in 0..plane.height() {
            samples.extend(upscale_row(
                plane.row(y).ok_or(CodecError::InvalidData)?,
                w,
                src.bit_depth(),
            )?);
        }
        planes.push(Plane::new(w, plane.height(), w, samples)?);
    }
    let [y, u, v]: [Plane; 3] = planes.try_into().map_err(|_| CodecError::InvalidData)?;
    PlanarFrame::new(src.bit_depth(), src.subsampling(), y, u, v)
}
