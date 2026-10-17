use codec_bitstream::BitReader;
use codec_core::{CodecError, CodecResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorConfig {
    pub high_bitdepth: bool,
    pub twelve_bit: bool,
    pub mono_chrome: bool,
    pub color_description_present_flag: bool,
    pub color_primaries: u8,
    pub transfer_characteristics: u8,
    pub matrix_coefficients: u8,
    pub color_range: bool,
    pub subsampling_x: bool,
    pub subsampling_y: bool,
    pub chroma_sample_position: u8,
    pub separate_uv_delta_q: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceHeader {
    pub seq_profile: u8,
    pub still_picture: bool,
    pub reduced_still_picture_header: bool,
    pub timing_info_present_flag: bool,
    pub initial_display_delay_present_flag: bool,
    pub operating_points_cnt_minus_1: u8,
    pub max_frame_width_minus_1: u32,
    pub max_frame_height_minus_1: u32,
    pub frame_width_bits: u8,
    pub frame_height_bits: u8,
    pub frame_id_length: u8,
    pub delta_frame_id_length: u8,
    pub use_128x128_superblock: bool,
    pub seq_force_screen_content_tools: u8,
    pub seq_force_integer_mv: u8,
    pub order_hint_bits: u8,
    pub enable_superres: bool,
    pub enable_cdef: bool,
    pub enable_restoration: bool,
    pub enable_intra_edge_filter: bool,
    pub enable_filter_intra: bool,
    pub film_grain_params_present: bool,
    pub decoder_model_info_present_flag: bool,
    pub operating_point_idc: [u32; 32],
    pub color_config: ColorConfig,
}

pub fn parse_sequence_header(reader: &mut BitReader) -> CodecResult<SequenceHeader> {
    let seq_profile = reader.read_bits(3)? as u8;
    if seq_profile > 2 {
        return Err(CodecError::Unsupported);
    }
    let still_picture = reader.read_bit()?;
    let reduced_still_picture_header = reader.read_bit()?;

    if reduced_still_picture_header && !still_picture {
        return Err(CodecError::InvalidData);
    }

    let mut timing_info_present_flag = false;
    let mut initial_display_delay_present_flag = false;
    let operating_points_cnt_minus_1;

    let mut decoder_model_info_present_flag = false;
    let mut buffer_delay_length_minus_1 = 0;
    let mut operating_points = [0u32; 32];

    if reduced_still_picture_header {
        operating_points_cnt_minus_1 = 0;
        let level = reader.read_bits(5)?;
        if level > 23 && level != 31 {
            return Err(CodecError::InvalidData);
        }
    } else {
        timing_info_present_flag = reader.read_bit()?;
        if timing_info_present_flag {
            // timing_info()
            let _num_units_in_display_tick = reader.read_bits(32)?;
            let _time_scale = reader.read_bits(32)?;
            if _num_units_in_display_tick == 0 || _time_scale == 0 {
                return Err(CodecError::InvalidData);
            }
            let equal_picture_interval = reader.read_bit()?;
            if equal_picture_interval {
                let ticks = reader.read_uvlc()?;
                if ticks == u32::MAX {
                    return Err(CodecError::InvalidData);
                }
            }
            decoder_model_info_present_flag = reader.read_bit()?;
            if decoder_model_info_present_flag {
                // decoder_model_info()
                buffer_delay_length_minus_1 = reader.read_bits(5)?;
                let _num_units_in_decoding_tick = reader.read_bits(32)?;
                if _num_units_in_decoding_tick == 0 {
                    return Err(CodecError::InvalidData);
                }
                let _buffer_removal_time_length_minus_1 = reader.read_bits(5)?;
                let _frame_presentation_time_length_minus_1 = reader.read_bits(5)?;
            }
        }

        initial_display_delay_present_flag = reader.read_bit()?;
        operating_points_cnt_minus_1 = reader.read_bits(5)? as u8;

        for i in 0..=usize::from(operating_points_cnt_minus_1) {
            let idc = reader.read_bits(12)?;
            if operating_points[..i].contains(&idc) {
                return Err(CodecError::InvalidData);
            }
            operating_points[i] = idc;
            let seq_level_idx = reader.read_bits(5)?;
            if seq_level_idx > 23 && seq_level_idx != 31 {
                return Err(CodecError::InvalidData);
            }
            if seq_level_idx > 7 {
                let _seq_tier = reader.read_bit()?;
            }
            if decoder_model_info_present_flag {
                let decoder_model_present_for_this_op = reader.read_bit()?;
                if decoder_model_present_for_this_op {
                    let _decoder_buffer_delay =
                        reader.read_bits(buffer_delay_length_minus_1 as u8 + 1)?;
                    let _encoder_buffer_delay =
                        reader.read_bits(buffer_delay_length_minus_1 as u8 + 1)?;
                    let _low_delay_mode_flag = reader.read_bit()?;
                }
            }
            if initial_display_delay_present_flag {
                let initial_display_delay_present_for_this_op = reader.read_bit()?;
                if initial_display_delay_present_for_this_op {
                    let _initial_display_delay_minus_1 = reader.read_bits(4)?;
                }
            }
        }
    }

    let frame_width_bits_minus_1 = reader.read_bits(4)?;
    let frame_height_bits_minus_1 = reader.read_bits(4)?;
    let max_frame_width_minus_1 = reader.read_bits(frame_width_bits_minus_1 as u8 + 1)?;
    let max_frame_height_minus_1 = reader.read_bits(frame_height_bits_minus_1 as u8 + 1)?;

    let mut frame_id_length = 0;
    let mut delta_frame_id_length = 0;
    let _frame_id_numbers_present_flag = if !reduced_still_picture_header {
        let flag = reader.read_bit()?;
        if flag {
            let _delta_frame_id_length_minus_2 = reader.read_bits(4)?;
            let _additional_frame_id_length_minus_1 = reader.read_bits(3)?;
            delta_frame_id_length = _delta_frame_id_length_minus_2 as u8 + 2;
            frame_id_length = delta_frame_id_length + _additional_frame_id_length_minus_1 as u8 + 1;
            if _delta_frame_id_length_minus_2 + _additional_frame_id_length_minus_1 + 3 > 16 {
                return Err(CodecError::InvalidData);
            }
        }
        flag
    } else {
        false
    };

    let _use_128x128_superblock = reader.read_bit()?;
    let enable_filter_intra = reader.read_bit()?;
    let enable_intra_edge_filter = reader.read_bit()?;

    let mut seq_force_screen_content_tools = 2;
    let mut seq_force_integer_mv = 2;
    let mut order_hint_bits = 0;
    if !reduced_still_picture_header {
        let _interintra_compound = reader.read_bit()?;
        let _masked_compound = reader.read_bit()?;
        let _warp_motion = reader.read_bit()?;
        let _dual_filter = reader.read_bit()?;

        let enable_order_hint = reader.read_bit()?;
        if enable_order_hint {
            let _jnt_comp = reader.read_bit()?;
            let _ref_frame_mvs = reader.read_bit()?;
        }
        let _seq_choose_screen_content_tools = reader.read_bit()?;
        let _seq_force_screen_content_tools = if !_seq_choose_screen_content_tools {
            reader.read_bits(1)?
        } else {
            2
        };
        seq_force_screen_content_tools = _seq_force_screen_content_tools as u8;
        if _seq_choose_screen_content_tools || _seq_force_screen_content_tools > 0 {
            let _seq_choose_integer_mv = reader.read_bit()?;
            if !_seq_choose_integer_mv {
                seq_force_integer_mv = reader.read_bits(1)? as u8;
            }
        }
        if enable_order_hint {
            order_hint_bits = reader.read_bits(3)? as u8 + 1;
        }
    }

    let _enable_superres = reader.read_bit()?;
    let _enable_cdef = reader.read_bit()?;
    let _enable_restoration = reader.read_bit()?;

    // Color Config
    let high_bitdepth = reader.read_bit()?;
    let mut twelve_bit = false;
    if seq_profile == 2 && high_bitdepth {
        twelve_bit = reader.read_bit()?;
    }

    let mono_chrome = if seq_profile == 1 {
        false
    } else {
        reader.read_bit()?
    };

    let color_description_present_flag = reader.read_bit()?;
    let mut color_primaries = 2; // CP_UNSPECIFIED
    let mut transfer_characteristics = 2; // TC_UNSPECIFIED
    let mut matrix_coefficients = 2; // MC_UNSPECIFIED

    if color_description_present_flag {
        color_primaries = reader.read_bits(8)? as u8;
        transfer_characteristics = reader.read_bits(8)? as u8;
        matrix_coefficients = reader.read_bits(8)? as u8;
    }

    let rgb = color_primaries == 1 && transfer_characteristics == 13 && matrix_coefficients == 0;
    let (color_range, subsampling_x, subsampling_y, chroma_sample_position, separate_uv_delta_q) =
        if mono_chrome {
            (reader.read_bit()?, true, true, 0, false)
        } else {
            let (range, sx, sy) = if rgb {
                // RGB signals 4:4:4 directly; no range or subsampling bits follow.
                if seq_profile != 1 && !(seq_profile == 2 && twelve_bit) {
                    return Err(CodecError::InvalidData);
                }
                (true, false, false)
            } else {
                let range = reader.read_bit()?;
                let (sx, sy) = match seq_profile {
                    0 => (true, true),
                    1 => (false, false),
                    _ if twelve_bit => {
                        let sx = reader.read_bit()?;
                        (sx, if sx { reader.read_bit()? } else { false })
                    }
                    _ => (true, false),
                };
                (range, sx, sy)
            };
            if matrix_coefficients == 0 && (sx || sy) {
                return Err(CodecError::InvalidData);
            }
            let position = if sx && sy {
                reader.read_bits(2)? as u8
            } else {
                0
            };
            if position == 3 {
                return Err(CodecError::InvalidData);
            }
            (range, sx, sy, position, reader.read_bit()?)
        };

    if matrix_coefficients == 0 && (subsampling_x || subsampling_y) {
        return Err(CodecError::InvalidData);
    }
    let color_config = ColorConfig {
        high_bitdepth,
        twelve_bit,
        mono_chrome,
        color_description_present_flag,
        color_primaries,
        transfer_characteristics,
        matrix_coefficients,
        color_range,
        subsampling_x,
        subsampling_y,
        chroma_sample_position,
        separate_uv_delta_q,
    };

    let _film_grain_params_present = reader.read_bit()?;

    // trailing_bits(): one followed by zeros to the exact payload boundary.
    if !reader.read_bit()? {
        return Err(CodecError::InvalidData);
    }
    while reader.bits_left() != 0 {
        if reader.read_bit()? {
            return Err(CodecError::InvalidData);
        }
    }

    Ok(SequenceHeader {
        seq_profile,
        still_picture,
        reduced_still_picture_header,
        timing_info_present_flag,
        initial_display_delay_present_flag,
        operating_points_cnt_minus_1,
        max_frame_width_minus_1,
        max_frame_height_minus_1,
        frame_width_bits: frame_width_bits_minus_1 as u8 + 1,
        frame_height_bits: frame_height_bits_minus_1 as u8 + 1,
        frame_id_length,
        delta_frame_id_length,
        use_128x128_superblock: _use_128x128_superblock,
        enable_filter_intra,
        seq_force_screen_content_tools,
        seq_force_integer_mv,
        order_hint_bits,
        enable_superres: _enable_superres,
        enable_cdef: _enable_cdef,
        enable_restoration: _enable_restoration,
        enable_intra_edge_filter,
        film_grain_params_present: _film_grain_params_present,
        decoder_model_info_present_flag,
        operating_point_idc: operating_points,
        color_config,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bits(values: &[(u32, u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut pos = 0;
        for &(value, width) in values {
            for bit in (0..width).rev() {
                if pos % 8 == 0 {
                    out.push(0);
                }
                let last = out.len() - 1;
                out[last] |= (((value >> bit) & 1) as u8) << (7 - pos % 8);
                pos += 1;
            }
        }
        out
    }
    fn prefix(profile: u32) -> Vec<(u32, u8)> {
        vec![
            (profile, 3),
            (1, 1),
            (1, 1),
            (0, 5),
            (0, 4),
            (0, 4),
            (0, 1),
            (0, 1),
            (0, 3),
            (0, 3),
        ]
    }
    #[test]
    fn rgb_twelve_bit_profile_two_does_not_read_subsampling() {
        let mut v = prefix(2);
        v.extend([
            (1, 1),
            (1, 1),
            (0, 1),
            (1, 1),
            (1, 8),
            (13, 8),
            (0, 8),
            (1, 1),
            (0, 1),
            (1, 1),
        ]);
        let bytes = bits(&v);
        let header = parse_sequence_header(&mut BitReader::new(&bytes)).unwrap();
        assert!(header.color_config.twelve_bit);
        assert!(header.color_config.separate_uv_delta_q);
        assert!(!header.color_config.subsampling_x);
        assert!(!header.color_config.subsampling_y);
    }
    #[test]
    fn monochrome_profile_two_skips_subsampling_and_uv_delta() {
        let mut v = prefix(2);
        v.extend([(1, 1), (1, 1), (1, 1), (0, 1), (1, 1), (0, 1), (1, 1)]);
        let bytes = bits(&v);
        let header = parse_sequence_header(&mut BitReader::new(&bytes)).unwrap();
        assert!(header.color_config.mono_chrome);
        assert!(!header.color_config.separate_uv_delta_q);
    }
    #[test]
    fn unsupported_and_inconsistent_headers() {
        assert!(parse_sequence_header(&mut BitReader::new(&[0xe0])).is_err());
        assert!(parse_sequence_header(&mut BitReader::new(&[0x08])).is_err());
        let mut v = prefix(0);
        v.extend([
            (0, 1),
            (0, 1),
            (1, 1),
            (1, 8),
            (13, 8),
            (0, 8),
            (0, 1),
            (0, 1),
            (1, 1),
        ]);
        assert!(parse_sequence_header(&mut BitReader::new(&bits(&v))).is_err());
    }
    #[test]
    fn reserved_chroma_and_missing_trailing_one() {
        for (position, trailing) in [(3, 1), (0, 0)] {
            let mut v = prefix(0);
            v.extend([
                (0, 1),
                (0, 1),
                (0, 1),
                (0, 1),
                (position, 2),
                (0, 1),
                (0, 1),
                (trailing, 1),
            ]);
            assert!(parse_sequence_header(&mut BitReader::new(&bits(&v))).is_err());
        }
    }

    #[test]
    fn timing_and_operating_point_constraints() {
        fn sequence(tick: u32, second_id: u32, max_uvlc: bool) -> Vec<u8> {
            let mut v = vec![
                (0, 3),
                (0, 1),
                (0, 1),
                (1, 1),
                (tick, 32),
                (1, 32),
                (u32::from(max_uvlc), 1),
            ];
            if max_uvlc {
                v.push((0, 32));
            }
            v.extend([
                (0, 1),
                (0, 1),
                (1, 5),
                (0, 12),
                (0, 5),
                (second_id, 12),
                (0, 5),
                (0, 4),
                (0, 4),
                (0, 1),
                (0, 1),
                (0, 1),
                (0, 3),
                (0, 1),
                (0, 1),
                (0, 1),
                (0, 1),
                (0, 1),
                (1, 1),
                (1, 1),
                (0, 3),
                (0, 1),
                (0, 1),
                (0, 1),
                (0, 1),
                (0, 2),
                (0, 1),
                (0, 1),
                (1, 1),
            ]);
            bits(&v)
        }
        assert!(parse_sequence_header(&mut BitReader::new(&sequence(1, 0x101, false))).is_ok());
        for bytes in [
            sequence(0, 0x101, false),
            sequence(1, 0, false),
            sequence(1, 0x101, true),
        ] {
            assert!(parse_sequence_header(&mut BitReader::new(&bytes)).is_err());
        }
    }
    #[test]
    fn reserved_level_and_payload_trailing_bits() {
        let mut v = prefix(0);
        v.extend([
            (0, 1),
            (0, 1),
            (0, 1),
            (0, 1),
            (0, 2),
            (0, 1),
            (0, 1),
            (1, 1),
        ]);
        let valid = bits(&v);
        assert!(parse_sequence_header(&mut BitReader::new(&valid)).is_ok());
        v[3].0 = 24;
        assert!(parse_sequence_header(&mut BitReader::new(&bits(&v))).is_err());
        let mut extra = valid.clone();
        extra.push(1);
        assert!(parse_sequence_header(&mut BitReader::new(&extra)).is_err());
        let mut padded = valid;
        padded.push(0);
        assert!(parse_sequence_header(&mut BitReader::new(&padded)).is_ok());
    }
}
