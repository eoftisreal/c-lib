//! Actual pixel reference ownership, separate from header-validation metadata.
//! Only the implemented no-deblock/no-CDEF/no-restoration intra subset can
//! commit pictures. Required unsupported processing fails before state changes.
use super::{grain::FilmGrainParameters, inter::InterRefFrame, FrameHeader, FrameType};
use crate::SequenceHeader;
use codec_core::{CodecError, CodecResult, ColorRange, PlanarFrame, Timestamp};
use std::sync::Arc;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencePicture {
    pub pixels: Arc<PlanarFrame>,
    pub header: FrameHeader,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayPicture {
    pub pixels: Arc<PlanarFrame>,
    pub pts: Option<Timestamp>,
    pub temporal_unit: u64,
    pub render_width: u32,
    pub render_height: u32,
    pub color_range: ColorRange,
    pub color_primaries: u8,
    pub transfer_characteristics: u8,
    pub matrix_coefficients: u8,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredPicture {
    picture: ReferencePicture,
    color_range: ColorRange,
    color_primaries: u8,
    transfer_characteristics: u8,
    matrix_coefficients: u8,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedPictureBuffer {
    slots: [Option<StoredPicture>; 8],
    budget: usize,
}
impl Default for DecodedPictureBuffer {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
            budget: 512 * 1024 * 1024,
        }
    }
}
impl DecodedPictureBuffer {
    pub fn with_budget(bytes: usize) -> Self {
        Self {
            budget: bytes,
            ..Self::default()
        }
    }
    pub fn reset(&mut self) {
        self.slots.fill(None);
    }
    pub fn reference(&self, index: u8) -> CodecResult<Option<&ReferencePicture>> {
        self.slots
            .get(usize::from(index))
            .map(|v| v.as_ref().map(|v| &v.picture))
            .ok_or(CodecError::InvalidData)
    }
    /// Resolve LAST..ALTREF through the seven signaled slot indices. This does
    /// not construct MV candidates or claim motion compensation is available.
    pub fn reference_for(
        &self,
        reference: InterRefFrame,
        mapping: [u8; 7],
    ) -> CodecResult<&ReferencePicture> {
        if reference == InterRefFrame::Intra || mapping.iter().any(|&i| i >= 8) {
            return Err(CodecError::InvalidData);
        }
        self.reference(mapping[reference as usize - 1])?
            .ok_or(CodecError::InvalidData)
    }
    pub fn grain_references(&self) -> [Option<FilmGrainParameters>; 8] {
        std::array::from_fn(|i| {
            self.slots[i]
                .as_ref()
                .and_then(|s| s.picture.header.film_grain.clone())
        })
    }
    fn display(
        s: &StoredPicture,
        pts: Option<Timestamp>,
        temporal_unit: u64,
    ) -> CodecResult<DisplayPicture> {
        if pts.is_some_and(|t| t.time_base_num == 0 || t.time_base_den == 0) {
            return Err(CodecError::InvalidData);
        }
        // Never expose ungrained pixels as successful normative grain output.
        if s.picture.header.film_grain.is_some() {
            return Err(CodecError::Unsupported);
        }
        Ok(DisplayPicture {
            pixels: Arc::clone(&s.picture.pixels),
            pts,
            temporal_unit,
            render_width: s.picture.header.dimensions.render_width,
            render_height: s.picture.header.dimensions.render_height,
            color_range: s.color_range,
            color_primaries: s.color_primaries,
            transfer_characteristics: s.transfer_characteristics,
            matrix_coefficients: s.matrix_coefficients,
        })
    }
    /// Commit supplied real reconstruction after checking the complete header
    /// requirements. No required unimplemented filter may be silently skipped.
    pub fn commit_intra(
        &mut self,
        sequence: &SequenceHeader,
        header: &FrameHeader,
        mut pixels: PlanarFrame,
        pts: Option<Timestamp>,
        temporal_unit: u64,
    ) -> CodecResult<Option<DisplayPicture>> {
        use codec_core::ChromaSubsampling;
        if sequence.frame_id_length > 16
            || (sequence.frame_id_length > 0
                && (sequence.delta_frame_id_length >= sequence.frame_id_length
                    || header.frame_id >= (1u32 << sequence.frame_id_length)))
            || sequence.order_hint_bits > 8
            || (sequence.order_hint_bits > 0
                && header.order_hint >= (1u32 << sequence.order_hint_bits))
            || (header.frame_type == FrameType::Key
                && header.show_frame
                && header.refresh_frame_flags != 255)
            || (header.frame_type == FrameType::IntraOnly && header.refresh_frame_flags == 255)
        {
            return Err(CodecError::InvalidData);
        }
        let d = &header.dimensions;
        let depth = if sequence.color_config.twelve_bit {
            12
        } else if sequence.color_config.high_bitdepth {
            10
        } else {
            8
        };
        let sub = match (
            sequence.color_config.mono_chrome,
            sequence.color_config.subsampling_x,
            sequence.color_config.subsampling_y,
        ) {
            (false, true, true) => ChromaSubsampling::Cs420,
            (false, true, false) => ChromaSubsampling::Cs422,
            (false, false, false) => ChromaSubsampling::Cs444,
            _ => return Err(CodecError::Unsupported),
        };
        if pixels.width() != d.width as usize
            || pixels.height() != d.height as usize
            || pixels.bit_depth() != depth
            || pixels.subsampling() != sub
            || d.render_width == 0
            || d.render_height == 0
            || d.render_width > 65536
            || d.render_height > 65536
            || !(8..=16).contains(&d.superres_denom)
            || d.upscaled_width == 0
            || d.upscaled_width > 65536
            || d.width
                != (d.upscaled_width * 8 + u32::from(d.superres_denom) / 2)
                    / u32::from(d.superres_denom)
            || pts.is_some_and(|t| t.time_base_num == 0 || t.time_base_den == 0)
        {
            return Err(CodecError::InvalidData);
        }
        if header.film_grain.is_some() {
            return Err(CodecError::Unsupported);
        }
        if header.cdef.bits > 3
            || header.cdef.strengths.len()
                != 1usize
                    .checked_shl(u32::from(header.cdef.bits))
                    .ok_or(CodecError::InvalidData)?
        {
            return Err(CodecError::InvalidData);
        }
        super::filters::deblock_frame(&mut pixels, &header.loop_filter)?;
        super::filters::cdef_frame(&mut pixels, &header.cdef)?;
        let mut pixels = if d.width != d.upscaled_width {
            super::superres::upscale_picture(&pixels, d.upscaled_width as usize)?
        } else {
            pixels
        };
        super::filters::restoration_frame(&mut pixels, &header.restoration)?;
        let c = &sequence.color_config;
        let stored = StoredPicture {
            picture: ReferencePicture {
                pixels: Arc::new(pixels),
                header: header.clone(),
            },
            color_range: if c.color_range {
                ColorRange::Full
            } else {
                ColorRange::Limited
            },
            color_primaries: c.color_primaries,
            transfer_characteristics: c.transfer_characteristics,
            matrix_coefficients: c.matrix_coefficients,
        };
        let display = if header.show_frame {
            Some(Self::display(&stored, pts, temporal_unit)?)
        } else {
            None
        };
        let mut next = self.slots.clone();
        for (i, slot) in next.iter_mut().enumerate() {
            if let Some(old) = slot {
                let bad_hint = header
                    .ref_order_hints
                    .is_some_and(|h| h[i] != old.picture.header.order_hint);
                let bad_id = if sequence.frame_id_length > 0 {
                    if sequence.frame_id_length > 16
                        || sequence.delta_frame_id_length >= sequence.frame_id_length
                    {
                        return Err(CodecError::InvalidData);
                    }
                    let current = header.frame_id;
                    let window = 1u32 << sequence.delta_frame_id_length;
                    let old = old.picture.header.frame_id;
                    if current > window {
                        old > current || old < current - window
                    } else {
                        old > current && old < (1u32 << sequence.frame_id_length) + current - window
                    }
                } else {
                    false
                };
                if bad_hint || bad_id {
                    *slot = None;
                }
            }
            if header.frame_type == FrameType::Key && header.show_frame {
                *slot = None;
            }
            if header.refresh_frame_flags & (1 << i) != 0 {
                *slot = Some(stored.clone());
            }
        }
        // Count each shared picture once, even if all eight slots reference it.
        let mut seen: Vec<&Arc<PlanarFrame>> = Vec::new();
        let mut used = 0usize;
        for s in next.iter().flatten() {
            let p = &s.picture.pixels;
            if seen.iter().any(|old| Arc::ptr_eq(old, p)) {
                continue;
            }
            for plane in p.planes() {
                used = used
                    .checked_add(
                        plane
                            .stride()
                            .checked_mul(plane.height())
                            .and_then(|v| v.checked_mul(2))
                            .ok_or(CodecError::InvalidData)?,
                    )
                    .ok_or(CodecError::InvalidData)?;
            }
            seen.push(p);
        }
        if used > self.budget {
            return Err(CodecError::OutOfMemory);
        }
        self.slots = next;
        Ok(display)
    }
    /// Display an existing reference with the *new* presentation timestamp.
    /// Hidden key pictures are showable only once and refresh all eight slots.
    /// Motion-field/CDF restoration remains outside this pixel ownership layer.
    pub fn show_existing(
        &mut self,
        index: u8,
        display_frame_id: Option<u32>,
        pts: Option<Timestamp>,
        temporal_unit: u64,
    ) -> CodecResult<DisplayPicture> {
        let s = self
            .slots
            .get(usize::from(index))
            .and_then(Option::as_ref)
            .ok_or(CodecError::InvalidData)?;
        if !s.picture.header.showable_frame
            || display_frame_id.is_some_and(|id| id != s.picture.header.frame_id)
        {
            return Err(CodecError::InvalidData);
        }
        let out = Self::display(s, pts, temporal_unit)?;
        if s.picture.header.frame_type == FrameType::Key {
            let mut s = s.clone();
            s.picture.header.showable_frame = false;
            self.slots.fill(Some(s));
        }
        Ok(out)
    }
}
