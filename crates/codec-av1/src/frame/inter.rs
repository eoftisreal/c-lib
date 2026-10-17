//! AV1 inter-frame header-prefix parsing and motion/reference type definitions.
//! Full inter headers, MV prediction and motion compensation are not implemented.

use crate::SequenceHeader;
use codec_bitstream::BitReader;
use codec_core::{CodecError, CodecResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendedFrameType {
    Key = 0,
    Inter = 1,
    IntraOnly = 2,
    Switch = 3,
    /// A show-existing prefix has no frame type until its slot is resolved.
    UnknownReference = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterRefFrame {
    Intra = 0,
    Last = 1,
    Last2 = 2,
    Last3 = 3,
    Golden = 4,
    BwdRef = 5,
    AltRef2 = 6,
    AltRef = 7,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterpFilter {
    EightTapRegular = 0,
    EightTapSmooth = 1,
    EightTapSharp = 2,
    Bilinear = 3,
    Switchable = 4,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MotionVector {
    /// 1/8-pel vertical displacement.
    pub row: i16,
    /// 1/8-pel horizontal displacement.
    pub col: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalMotionModel {
    Identity,
    Translation {
        tx: i32,
        ty: i32,
    },
    RotZoom {
        alpha: i32,
        beta: i32,
        tx: i32,
        ty: i32,
    },
    Affine {
        wmmat: [i32; 6],
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtendedFrameHeader {
    pub show_existing_frame: bool,
    pub frame_to_show_map_idx: Option<u8>,
    pub frame_type: ExtendedFrameType,
    pub show_frame: bool,
    pub showable_frame: bool,
    pub error_resilient_mode: bool,
    pub disable_cdf_update: bool,
    pub frame_id: u32,
    pub order_hint: u32,
    pub refresh_frame_flags: u8,
}

impl ExtendedFrameHeader {
    /// Normative AV1 uncompressed header prefix parser supporting Key, Inter, IntraOnly,
    /// Switch, and `show_existing_frame` across multi-frame streams.
    pub fn parse_prefix(r: &mut BitReader, s: &SequenceHeader) -> CodecResult<Self> {
        if s.decoder_model_info_present_flag {
            return Err(CodecError::Unsupported);
        }
        let mut next = r.clone();
        let header = Self::parse_prefix_inner(&mut next, s)?;
        *r = next;
        Ok(header)
    }
    fn parse_prefix_inner(r: &mut BitReader, s: &SequenceHeader) -> CodecResult<Self> {
        if s.reduced_still_picture_header {
            return Ok(Self {
                show_existing_frame: false,
                frame_to_show_map_idx: None,
                frame_type: ExtendedFrameType::Key,
                show_frame: true,
                showable_frame: false,
                error_resilient_mode: true,
                disable_cdf_update: r.read_bit()?,
                frame_id: 0,
                order_hint: 0,
                refresh_frame_flags: 0xff,
            });
        }

        let show_existing_frame = r.read_bit()?;
        if show_existing_frame {
            let idx = r.read_bits(3)? as u8;
            let frame_id = if s.frame_id_length > 0 {
                r.read_bits(s.frame_id_length)?
            } else {
                0
            };
            return Ok(Self {
                show_existing_frame: true,
                frame_to_show_map_idx: Some(idx),
                frame_type: ExtendedFrameType::UnknownReference,
                show_frame: true,
                showable_frame: false,
                error_resilient_mode: false,
                disable_cdf_update: true,
                frame_id,
                order_hint: 0,
                refresh_frame_flags: 0,
            });
        }

        let frame_type = match r.read_bits(2)? {
            0 => ExtendedFrameType::Key,
            1 => ExtendedFrameType::Inter,
            2 => ExtendedFrameType::IntraOnly,
            3 => ExtendedFrameType::Switch,
            _ => unreachable!(),
        };
        let show_frame = r.read_bit()?;
        if s.still_picture && (frame_type != ExtendedFrameType::Key || !show_frame) {
            return Err(CodecError::InvalidData);
        }
        let showable_frame = if show_frame {
            frame_type != ExtendedFrameType::Key
        } else {
            r.read_bit()?
        };
        let error_resilient_mode = match frame_type {
            ExtendedFrameType::Key if show_frame => true,
            ExtendedFrameType::Switch => true,
            _ => r.read_bit()?,
        };
        let disable_cdf_update = r.read_bit()?;
        let allow_screen_content_tools = if s.seq_force_screen_content_tools == 2 {
            r.read_bit()?
        } else {
            s.seq_force_screen_content_tools == 1
        };
        if allow_screen_content_tools && s.seq_force_integer_mv == 2 {
            let _force_integer_mv = r.read_bit()?;
        }
        let frame_id = if s.frame_id_length > 0 {
            r.read_bits(s.frame_id_length)?
        } else {
            0
        };
        let _frame_size_override_flag = match frame_type {
            ExtendedFrameType::Switch => true,
            _ => r.read_bit()?,
        };
        let order_hint = if s.order_hint_bits > 0 {
            r.read_bits(s.order_hint_bits)?
        } else {
            0
        };
        let is_intra = matches!(
            frame_type,
            ExtendedFrameType::Key | ExtendedFrameType::IntraOnly
        );
        if !is_intra && !error_resilient_mode {
            let _primary_ref_frame = r.read_bits(3)?;
        }
        let refresh_frame_flags = match frame_type {
            ExtendedFrameType::Key if show_frame => 0xff,
            ExtendedFrameType::Switch => 0xff,
            _ => r.read_bits(8)? as u8,
        };

        Ok(Self {
            show_existing_frame: false,
            frame_to_show_map_idx: None,
            frame_type,
            show_frame,
            showable_frame,
            error_resilient_mode,
            disable_cdf_update,
            frame_id,
            order_hint,
            refresh_frame_flags,
        })
    }
}
