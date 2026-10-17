use super::{FrameHeader, FrameType};
use crate::SequenceHeader;

/// Header-derived metadata only. Presence does not assert a reconstructed reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceMetadata {
    pub frame_id: u32,
    pub order_hint: u32,
    pub header: FrameHeader,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ParserState {
    #[default]
    AwaitingHeader,
    AwaitingTiles {
        header: Box<FrameHeader>,
        next_tile: u32,
        header_payload: Vec<u8>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DecoderState {
    pub(crate) sequence: Option<SequenceHeader>,
    pub(crate) references: [Option<ReferenceMetadata>; 8],
    pub(crate) parser: ParserState,
    pub(crate) last_header: Option<FrameHeader>,
    pub(crate) temporal_unit: u64,
    pub(crate) picture_pending: bool,
    pub(crate) pictures: super::lifecycle::DecodedPictureBuffer,
}
impl DecoderState {
    pub fn sequence(&self) -> Option<&SequenceHeader> {
        self.sequence.as_ref()
    }
    pub fn references(&self) -> &[Option<ReferenceMetadata>; 8] {
        &self.references
    }
    pub fn parser(&self) -> &ParserState {
        &self.parser
    }
    pub fn last_header(&self) -> Option<&FrameHeader> {
        self.last_header.as_ref()
    }
    pub fn temporal_unit(&self) -> u64 {
        self.temporal_unit
    }
    /// Actual pixel slots, independent of header-only validation metadata.
    pub fn pictures(&self) -> &super::lifecycle::DecodedPictureBuffer {
        &self.pictures
    }
    /// Finalize only the last completely parsed intra header. Required unsupported
    /// filters/grain fail before refreshing any actual picture reference.
    pub fn commit_intra_picture(
        &mut self,
        pixels: codec_core::PlanarFrame,
        pts: Option<codec_core::Timestamp>,
    ) -> codec_core::CodecResult<Option<super::lifecycle::DisplayPicture>> {
        if !self.picture_pending || !matches!(self.parser, ParserState::AwaitingHeader) {
            return Err(codec_core::CodecError::InvalidData);
        }
        let header = self
            .last_header
            .as_ref()
            .ok_or(codec_core::CodecError::InvalidData)?;
        let sequence = self
            .sequence
            .as_ref()
            .ok_or(codec_core::CodecError::InvalidData)?;
        let output =
            self.pictures
                .commit_intra(sequence, header, pixels, pts, self.temporal_unit)?;
        self.picture_pending = false;
        Ok(output)
    }
    /// Display previously finalized pixels; timing comes from this presentation,
    /// not the packet that originally reconstructed the reference picture.
    pub fn show_existing_picture(
        &mut self,
        index: u8,
        display_frame_id: Option<u32>,
        pts: Option<codec_core::Timestamp>,
    ) -> codec_core::CodecResult<super::lifecycle::DisplayPicture> {
        if self.picture_pending || !matches!(self.parser, ParserState::AwaitingHeader) {
            return Err(codec_core::CodecError::InvalidData);
        }
        let seq = self
            .sequence
            .as_ref()
            .ok_or(codec_core::CodecError::InvalidData)?;
        if seq.reduced_still_picture_header
            || (seq.frame_id_length > 0) != display_frame_id.is_some()
        {
            return Err(codec_core::CodecError::InvalidData);
        }
        let result =
            self.pictures
                .show_existing(index, display_frame_id, pts, self.temporal_unit)?;
        if self
            .pictures
            .reference(index)?
            .is_some_and(|r| r.header.frame_type == FrameType::Key)
        {
            let h = self
                .pictures
                .reference(index)?
                .ok_or(codec_core::CodecError::InvalidData)?
                .header
                .clone();
            self.references = std::array::from_fn(|_| {
                Some(ReferenceMetadata {
                    frame_id: h.frame_id,
                    order_hint: h.order_hint,
                    header: h.clone(),
                })
            });
        }
        Ok(result)
    }
    /// This subset returns presentations synchronously and has no reorder queue.
    /// Flush refuses unfinished syntax/reconstruction rather than inventing output.
    pub fn flush(&self) -> codec_core::CodecResult<()> {
        if self.picture_pending || !matches!(self.parser, ParserState::AwaitingHeader) {
            return Err(codec_core::CodecError::NeedMoreData);
        }
        Ok(())
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn begin(&mut self, header: &FrameHeader) {
        if header.frame_type == FrameType::Key && header.show_frame {
            self.references.fill(None);
        }
        let s = self
            .sequence
            .as_ref()
            .expect("sequence checked before header");
        for (i, slot) in self.references.iter_mut().enumerate() {
            if let Some(reference) = slot {
                let bad_hint = header
                    .ref_order_hints
                    .is_some_and(|h| h[i] != reference.order_hint);
                let bad_id = if s.frame_id_length > 0 {
                    let current = header.frame_id;
                    let window = 1u32 << s.delta_frame_id_length;
                    if current > window {
                        reference.frame_id > current || reference.frame_id < current - window
                    } else {
                        reference.frame_id > current
                            && reference.frame_id < (1u32 << s.frame_id_length) + current - window
                    }
                } else {
                    false
                };
                if bad_hint || bad_id {
                    *slot = None;
                }
            }
        }
    }
    pub(crate) fn complete(&mut self, header: FrameHeader) {
        self.picture_pending = true;
        for (i, slot) in self.references.iter_mut().enumerate() {
            if header.refresh_frame_flags & (1 << i) != 0 {
                *slot = Some(ReferenceMetadata {
                    frame_id: header.frame_id,
                    order_hint: header.order_hint,
                    header: header.clone(),
                });
            }
        }
        self.last_header = Some(header);
        self.parser = ParserState::AwaitingHeader;
    }
}
