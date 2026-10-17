//! Complete film-grain parameter syntax (AV1 5.9.30), not grain synthesis.
use codec_bitstream::BitReader;
use codec_core::{CodecError, CodecResult};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilmGrainParameters {
    pub seed: u16,
    pub y_points: Vec<[u8; 2]>,
    pub cb_points: Vec<[u8; 2]>,
    pub cr_points: Vec<[u8; 2]>,
    pub chroma_scaling_from_luma: bool,
    pub scaling_shift: u8,
    pub ar_coeff_lag: u8,
    pub ar_coeffs_y: Vec<i8>,
    pub ar_coeffs_cb: Vec<i8>,
    pub ar_coeffs_cr: Vec<i8>,
    pub ar_coeff_shift: u8,
    pub grain_scale_shift: u8,
    /// mult, luma_mult and offset retain their unsigned signaled representations.
    pub cb_scaling: Option<(u8, u8, u16)>,
    pub cr_scaling: Option<(u8, u8, u16)>,
    pub overlap: bool,
    pub clip_to_restricted_range: bool,
}
#[derive(Debug, Clone, Copy)]
pub struct GrainContext {
    pub present: bool,
    pub show_frame: bool,
    pub showable_frame: bool,
    pub inter: bool,
    pub monochrome: bool,
    pub subsampling_x: bool,
    pub subsampling_y: bool,
}
impl FilmGrainParameters {
    /// Reader and reference parameters are unchanged on any failure. For inter
    /// reuse, the selected reference must be one of the current seven references.
    pub fn parse(
        r: &mut BitReader<'_>,
        c: GrainContext,
        references: &[Option<Self>; 8],
        ref_indices: &[u8],
    ) -> CodecResult<Option<Self>> {
        let mut next = r.clone();
        let value = Self::parse_inner(&mut next, c, references, ref_indices)?;
        *r = next;
        Ok(value)
    }
    fn parse_inner(
        r: &mut BitReader<'_>,
        c: GrainContext,
        references: &[Option<Self>; 8],
        ref_indices: &[u8],
    ) -> CodecResult<Option<Self>> {
        if !c.present || (!c.show_frame && !c.showable_frame) {
            return Ok(None);
        }
        if !r.read_bit()? {
            return Ok(None);
        }
        let seed = r.read_bits(16)? as u16;
        if c.inter && !r.read_bit()? {
            let idx = r.read_bits(3)? as usize;
            if ref_indices.len() != 7
                || ref_indices.iter().any(|&v| v >= 8)
                || !ref_indices.contains(&(idx as u8))
            {
                return Err(CodecError::InvalidData);
            }
            let mut p = references[idx].clone().ok_or(CodecError::InvalidData)?;
            p.seed = seed;
            return Ok(Some(p));
        }
        fn points(r: &mut BitReader<'_>, max: usize) -> CodecResult<Vec<[u8; 2]>> {
            let n = r.read_bits(4)? as usize;
            if n > max {
                return Err(CodecError::InvalidData);
            }
            let mut out: Vec<[u8; 2]> = Vec::with_capacity(n);
            for _ in 0..n {
                let pair = [r.read_bits(8)? as u8, r.read_bits(8)? as u8];
                if out.last().is_some_and(|last| last[0] >= pair[0]) {
                    return Err(CodecError::InvalidData);
                }
                out.push(pair);
            }
            Ok(out)
        }
        let y_points = points(r, 14)?;
        let chroma_scaling_from_luma = !c.monochrome && r.read_bit()?;
        let (cb_points, cr_points) = if c.monochrome
            || chroma_scaling_from_luma
            || (c.subsampling_x && c.subsampling_y && y_points.is_empty())
        {
            (vec![], vec![])
        } else {
            (points(r, 10)?, points(r, 10)?)
        };
        if c.subsampling_x && c.subsampling_y && cb_points.is_empty() != cr_points.is_empty() {
            return Err(CodecError::InvalidData);
        }
        let scaling_shift = r.read_bits(2)? as u8 + 8;
        let ar_coeff_lag = r.read_bits(2)? as u8;
        let n = usize::from(2 * ar_coeff_lag * (ar_coeff_lag + 1));
        let nc = n + usize::from(!y_points.is_empty());
        fn coeffs(r: &mut BitReader<'_>, n: usize) -> CodecResult<Vec<i8>> {
            (0..n)
                .map(|_| r.read_bits(8).map(|v| (v as i16 - 128) as i8))
                .collect()
        }
        let ar_coeffs_y = coeffs(r, if y_points.is_empty() { 0 } else { n })?;
        let ar_coeffs_cb = coeffs(
            r,
            if chroma_scaling_from_luma || !cb_points.is_empty() {
                nc
            } else {
                0
            },
        )?;
        let ar_coeffs_cr = coeffs(
            r,
            if chroma_scaling_from_luma || !cr_points.is_empty() {
                nc
            } else {
                0
            },
        )?;
        let ar_coeff_shift = r.read_bits(2)? as u8 + 6;
        let grain_scale_shift = r.read_bits(2)? as u8;
        fn scaling(r: &mut BitReader<'_>, on: bool) -> CodecResult<Option<(u8, u8, u16)>> {
            if on {
                Ok(Some((
                    r.read_bits(8)? as u8,
                    r.read_bits(8)? as u8,
                    r.read_bits(9)? as u16,
                )))
            } else {
                Ok(None)
            }
        }
        let cb_scaling = scaling(r, !cb_points.is_empty())?;
        let cr_scaling = scaling(r, !cr_points.is_empty())?;
        let overlap = r.read_bit()?;
        let clip_to_restricted_range = r.read_bit()?;
        Ok(Some(Self {
            seed,
            y_points,
            cb_points,
            cr_points,
            chroma_scaling_from_luma,
            scaling_shift,
            ar_coeff_lag,
            ar_coeffs_y,
            ar_coeffs_cb,
            ar_coeffs_cr,
            ar_coeff_shift,
            grain_scale_shift,
            cb_scaling,
            cr_scaling,
            overlap,
            clip_to_restricted_range,
        }))
    }
}
