use super::{
    dimensions::Dimensions,
    filters::{Cdef, LoopFilter, Restoration},
    quantization::{DeltaParameters, Quantization, Segmentation},
    tile::TileInfo,
};
use crate::SequenceHeader;
use codec_bitstream::BitReader;
use codec_core::{CodecError, CodecResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    Key,
    IntraOnly,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxMode {
    Only4x4,
    Largest,
    Select,
}

/// A fully parsed intra header. Inter/show-existing headers are explicitly unsupported.
/// This carries syntax metadata only, never reconstructed reference pixels or CDFs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameHeader {
    pub frame_type: FrameType,
    pub show_frame: bool,
    pub showable_frame: bool,
    pub error_resilient_mode: bool,
    pub disable_cdf_update: bool,
    pub disable_frame_end_update_cdf: bool,
    pub allow_screen_content_tools: bool,
    pub allow_intrabc: bool,
    pub frame_id: u32,
    pub order_hint: u32,
    pub refresh_frame_flags: u8,
    pub dimensions: Dimensions,
    pub tiles: TileInfo,
    pub quantization: Quantization,
    pub segmentation: Segmentation,
    pub delta: DeltaParameters,
    pub coded_lossless: bool,
    pub loop_filter: LoopFilter,
    pub cdef: Cdef,
    pub restoration: Restoration,
    pub tx_mode: TxMode,
    pub reduced_tx_set: bool,
    pub film_grain: Option<super::grain::FilmGrainParameters>,
    /// Exact number of uncompressed-header bits, excluding alignment/trailing bits.
    pub header_bits: usize,
    pub(crate) ref_order_hints: Option<[u32; 8]>,
}

/// AV1 section 5.9.2, complete key/intra-only syntax including film-grain metadata.
/// Unsupported branches return an error before publishing any header or state.
pub fn parse_uncompressed_header(
    r: &mut BitReader,
    s: &SequenceHeader,
) -> CodecResult<FrameHeader> {
    if s.decoder_model_info_present_flag
        || s.operating_points_cnt_minus_1 != 0
        || s.operating_point_idc[0] != 0
    {
        return Err(CodecError::Unsupported);
    }
    let start = r.bits_left();
    let (frame_type, show_frame, showable_frame, error_resilient_mode) =
        if s.reduced_still_picture_header {
            (FrameType::Key, true, false, true)
        } else {
            if r.read_bit()? {
                return Err(CodecError::Unsupported);
            }
            let frame_type = match r.read_bits(2)? {
                0 => FrameType::Key,
                2 => FrameType::IntraOnly,
                _ => return Err(CodecError::Unsupported),
            };
            let show = r.read_bit()?;
            if s.still_picture && (frame_type != FrameType::Key || !show) {
                return Err(CodecError::InvalidData);
            }
            let showable = if show {
                frame_type != FrameType::Key
            } else {
                r.read_bit()?
            };
            let resilient = (frame_type == FrameType::Key && show) || r.read_bit()?;
            (frame_type, show, showable, resilient)
        };
    let disable_cdf_update = r.read_bit()?;
    let allow_screen_content_tools = if s.seq_force_screen_content_tools == 2 {
        r.read_bit()?
    } else {
        s.seq_force_screen_content_tools == 1
    };
    if allow_screen_content_tools && s.seq_force_integer_mv == 2 {
        r.read_bit()?;
    }
    // force_integer_mv is normatively 1 for every intra frame, after consuming syntax.
    let frame_id = r.read_bits(s.frame_id_length)?;
    let override_size = !s.reduced_still_picture_header && r.read_bit()?;
    let order_hint = r.read_bits(s.order_hint_bits)?;
    let refresh_frame_flags = if frame_type == FrameType::Key && show_frame {
        255
    } else {
        r.read_bits(8)? as u8
    };
    if frame_type == FrameType::IntraOnly && refresh_frame_flags == 255 {
        return Err(CodecError::InvalidData);
    }
    let ref_order_hints =
        if refresh_frame_flags != 255 && error_resilient_mode && s.order_hint_bits > 0 {
            let mut hints = [0; 8];
            for hint in &mut hints {
                *hint = r.read_bits(s.order_hint_bits)?;
            }
            Some(hints)
        } else {
            None
        };
    let dimensions = Dimensions::parse(r, s, override_size)?;
    let allow_intrabc = allow_screen_content_tools
        && dimensions.width == dimensions.upscaled_width
        && r.read_bit()?;
    let disable_frame_end_update_cdf =
        s.reduced_still_picture_header || disable_cdf_update || r.read_bit()?;
    let tiles = TileInfo::parse(r, &dimensions, s.use_128x128_superblock)?;
    let quantization = Quantization::parse(r, s)?;
    let segmentation = Segmentation::parse(r)?;
    let delta = DeltaParameters::parse(r, quantization.base_q_idx, allow_intrabc)?;
    let coded_lossless = segmentation.coded_lossless(&quantization);
    let loop_filter = LoopFilter::parse(
        r,
        s.color_config.mono_chrome,
        coded_lossless || allow_intrabc,
    )?;
    let cdef = Cdef::parse(
        r,
        s.color_config.mono_chrome,
        coded_lossless || allow_intrabc || !s.enable_cdef,
    )?;
    let all_lossless = coded_lossless && dimensions.width == dimensions.upscaled_width;
    let restoration =
        Restoration::parse(r, s, all_lossless || allow_intrabc || !s.enable_restoration)?;
    let tx_mode = if coded_lossless {
        TxMode::Only4x4
    } else if r.read_bit()? {
        TxMode::Select
    } else {
        TxMode::Largest
    };
    // Intra: reference_select=0, skip_mode_present=0, allow_warped_motion=0;
    // all seven global motions are IDENTITY and consume no bits.
    let reduced_tx_set = r.read_bit()?;
    let film_grain = super::grain::FilmGrainParameters::parse(
        r,
        super::grain::GrainContext {
            present: s.film_grain_params_present,
            show_frame,
            showable_frame,
            inter: false,
            monochrome: s.color_config.mono_chrome,
            subsampling_x: s.color_config.subsampling_x,
            subsampling_y: s.color_config.subsampling_y,
        },
        &std::array::from_fn(|_| None),
        &[],
    )?;
    Ok(FrameHeader {
        frame_type,
        show_frame,
        showable_frame,
        error_resilient_mode,
        disable_cdf_update,
        disable_frame_end_update_cdf,
        allow_screen_content_tools,
        allow_intrabc,
        frame_id,
        order_hint,
        refresh_frame_flags,
        dimensions,
        tiles,
        quantization,
        segmentation,
        delta,
        coded_lossless,
        loop_filter,
        cdef,
        restoration,
        tx_mode,
        reduced_tx_set,
        film_grain,
        header_bits: start - r.bits_left(),
        ref_order_hints,
    })
}
