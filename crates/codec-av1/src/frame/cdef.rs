//! Normative scalar CDEF sample kernel. Strength selection, direction search,
//! skip maps, chroma direction remapping and frame orchestration are separate.
use super::filters::cdef_constrain;
use codec_core::{CodecError, CodecResult, Plane};
const DIRECTIONS: [[(isize, isize); 2]; 8] = [
    [(-1, 1), (-2, 2)],
    [(0, 1), (-1, 2)],
    [(0, 1), (0, 2)],
    [(0, 1), (1, 2)],
    [(1, 1), (2, 2)],
    [(1, 0), (2, 1)],
    [(1, 0), (2, 0)],
    [(1, 0), (2, -1)],
];
#[derive(Debug, Clone, Copy)]
pub struct CdefKernel {
    pub direction: u8,
    /// Pixel-domain strengths, including fractional-8-bit luma adjustment at
    /// high bit depth. Secondary strength is already shifted by bit_depth - 8.
    pub primary: u16,
    pub secondary: u16,
    pub damping: u8,
    pub bit_depth: u8,
}
impl CdefKernel {
    /// Derive the luma strength *after* bit-depth scaling, preserving the exact
    /// high-bit-depth rounding required by the variance adjustment process.
    pub fn for_luma(
        direction: u8,
        primary: u8,
        secondary: u8,
        damping: u8,
        bit_depth: u8,
        variance: u32,
    ) -> CodecResult<Self> {
        if !matches!(bit_depth, 8 | 10 | 12)
            || primary > 15
            || !matches!(secondary, 0 | 1 | 2 | 4)
            || direction > 7
            || !(3..=6).contains(&damping)
        {
            return Err(CodecError::InvalidData);
        }
        let shift = bit_depth - 8;
        let p = u16::from(primary) << shift;
        let v = variance >> 6;
        let scale = if v == 0 { 0 } else { v.ilog2().min(12) as u16 };
        let adjusted = if variance == 0 {
            0
        } else {
            (p * (4 + scale) + 8) >> 4
        };
        Ok(Self {
            direction: if primary == 0 { 0 } else { direction },
            primary: adjusted,
            secondary: u16::from(secondary) << shift,
            damping,
            bit_depth,
        })
    }
    /// Chroma uses the luma direction remapped for its sampling grid and one
    /// less unit of damping. Strengths are not variance-adjusted for chroma.
    #[allow(clippy::too_many_arguments)]
    pub fn for_chroma(
        luma_direction: u8,
        subsampling_x: bool,
        subsampling_y: bool,
        primary: u8,
        secondary: u8,
        damping: u8,
        bit_depth: u8,
    ) -> CodecResult<Self> {
        if !matches!(bit_depth, 8 | 10 | 12)
            || luma_direction > 7
            || primary > 15
            || !matches!(secondary, 0 | 1 | 2 | 4)
            || !(3..=6).contains(&damping)
        {
            return Err(CodecError::InvalidData);
        }
        const MAP: [[[u8; 8]; 2]; 2] = [
            [[0, 1, 2, 3, 4, 5, 6, 7], [1, 2, 2, 2, 3, 4, 6, 0]],
            [[7, 0, 2, 4, 5, 6, 6, 6], [0, 1, 2, 3, 4, 5, 6, 7]],
        ];
        let direction = if primary == 0 {
            0
        } else {
            MAP[usize::from(subsampling_x)][usize::from(subsampling_y)][usize::from(luma_direction)]
        };
        Ok(Self {
            direction,
            primary: u16::from(primary) << (bit_depth - 8),
            secondary: u16::from(secondary) << (bit_depth - 8),
            damping: damping - 1,
            bit_depth,
        })
    }
    /// `src` is the prepared filter region; pixels beyond its edges are absent,
    /// never clamped. Block dimensions may be clipped at a picture boundary.
    pub fn apply(
        self,
        src: &Plane,
        origin: (usize, usize),
        size: (usize, usize),
    ) -> CodecResult<Vec<u16>> {
        let (x0, y0) = origin;
        let (w, h) = size;
        if !matches!(self.bit_depth, 8 | 10 | 12)
            || self.direction > 7
            || self.primary > (15u16 << (self.bit_depth - 8))
            || ![
                0u16,
                1u16 << (self.bit_depth - 8),
                2u16 << (self.bit_depth - 8),
                4u16 << (self.bit_depth - 8),
            ]
            .contains(&self.secondary)
            || !(2..=6).contains(&self.damping)
            || w == 0
            || h == 0
            || w > 8
            || h > 8
            || x0.checked_add(w).is_none_or(|x| x > src.width())
            || y0.checked_add(h).is_none_or(|y| y > src.height())
        {
            return Err(CodecError::InvalidData);
        }
        let shift = self.bit_depth - 8;
        let pri = self.primary as u8;
        let sec = self.secondary as u8;
        let damping = self.damping + shift;
        let max_sample = (1u16 << self.bit_depth) - 1;
        let mut out = vec![0; w * h];
        for y in 0..h {
            for x in 0..w {
                let gx = x0 + x;
                let gy = y0 + y;
                let center = src.row(gy).ok_or(CodecError::InvalidData)?[gx];
                if center > max_sample {
                    return Err(CodecError::InvalidData);
                }
                let mut lo = i32::from(center);
                let mut hi = lo;
                let mut sum = 0i32;
                for k in 0..2 {
                    for sign in [-1isize, 1] {
                        for offset in [0i32, -2, 2] {
                            let dir = ((i32::from(self.direction) + offset) & 7) as usize;
                            let (dy, dx) = DIRECTIONS[dir][k];
                            let yy = gy as isize + sign * dy;
                            let xx = gx as isize + sign * dx;
                            if yy < 0
                                || xx < 0
                                || yy as usize >= src.height()
                                || xx as usize >= src.width()
                            {
                                continue;
                            }
                            let p =
                                src.row(yy as usize).ok_or(CodecError::InvalidData)?[xx as usize];
                            if p > max_sample {
                                return Err(CodecError::InvalidData);
                            }
                            let p = i32::from(p);
                            lo = lo.min(p);
                            hi = hi.max(p);
                            let (weight, strength) = if offset == 0 {
                                (
                                    if (self.primary >> shift) & 1 == 0 {
                                        [4, 2][k]
                                    } else {
                                        3
                                    },
                                    pri,
                                )
                            } else {
                                ([2, 1][k], sec)
                            };
                            sum +=
                                weight * cdef_constrain(p - i32::from(center), strength, damping);
                        }
                    }
                }
                out[y * w + x] = (i32::from(center) + ((sum + 8 - i32::from(sum < 0)) >> 4))
                    .clamp(lo, hi) as u16;
            }
        }
        Ok(out)
    }
}
