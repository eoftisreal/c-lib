use crate::SequenceHeader;
use codec_bitstream::BitReader;
use codec_core::{CodecError, CodecResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dimensions {
    pub upscaled_width: u32,
    pub width: u32,
    pub height: u32,
    pub render_width: u32,
    pub render_height: u32,
    pub superres_denom: u8,
}
impl Dimensions {
    pub(crate) fn parse(
        r: &mut BitReader,
        s: &SequenceHeader,
        override_size: bool,
    ) -> CodecResult<Self> {
        let (upscaled_width, height) = if override_size {
            (
                r.read_bits(s.frame_width_bits)? + 1,
                r.read_bits(s.frame_height_bits)? + 1,
            )
        } else {
            (
                s.max_frame_width_minus_1 + 1,
                s.max_frame_height_minus_1 + 1,
            )
        };
        if upscaled_width > s.max_frame_width_minus_1 + 1 || height > s.max_frame_height_minus_1 + 1
        {
            return Err(CodecError::InvalidData);
        }
        let superres_denom = if s.enable_superres && r.read_bit()? {
            r.read_bits(3)? as u8 + 9
        } else {
            8
        };
        let width =
            (upscaled_width * 8 + u32::from(superres_denom) / 2) / u32::from(superres_denom);
        let (render_width, render_height) = if r.read_bit()? {
            (r.read_bits(16)? + 1, r.read_bits(16)? + 1)
        } else {
            (upscaled_width, height)
        };
        Ok(Self {
            upscaled_width,
            width,
            height,
            render_width,
            render_height,
            superres_denom,
        })
    }
    pub fn mi_cols(&self) -> u32 {
        2 * self.width.div_ceil(8)
    }
    pub fn mi_rows(&self) -> u32 {
        2 * self.height.div_ceil(8)
    }
}
