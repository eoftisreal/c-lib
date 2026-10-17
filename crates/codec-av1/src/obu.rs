use codec_bitstream::BitReader;
use codec_core::{CodecError, CodecResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObuType {
    Reserved = 0,
    SequenceHeader = 1,
    TemporalDelimiter = 2,
    FrameHeader = 3,
    TileGroup = 4,
    Metadata = 5,
    Frame = 6,
    RedundantFrameHeader = 7,
    TileList = 8,
    Padding = 15,
}

#[derive(Debug, Clone)]
pub struct ObuHeader {
    pub obu_type: ObuType,
    pub obu_ext_flag: bool,
    pub obu_has_size_field: bool,
    pub temporal_id: u8,
    pub spatial_id: u8,
}

pub fn parse_obu_header(reader: &mut BitReader) -> CodecResult<ObuHeader> {
    let obu_forbidden_bit = reader.read_bit()?;
    if obu_forbidden_bit {
        return Err(CodecError::InvalidData);
    }

    let obu_type_val = reader.read_bits(4)?;
    let obu_type = match obu_type_val {
        1 => ObuType::SequenceHeader,
        2 => ObuType::TemporalDelimiter,
        3 => ObuType::FrameHeader,
        4 => ObuType::TileGroup,
        5 => ObuType::Metadata,
        6 => ObuType::Frame,
        7 => ObuType::RedundantFrameHeader,
        8 => ObuType::TileList,
        15 => ObuType::Padding,
        _ => ObuType::Reserved,
    };

    let obu_ext_flag = reader.read_bit()?;
    let obu_has_size_field = reader.read_bit()?;
    if reader.read_bit()? {
        return Err(CodecError::InvalidData);
    }

    let mut temporal_id = 0;
    let mut spatial_id = 0;

    if obu_ext_flag {
        temporal_id = reader.read_bits(3)? as u8;
        spatial_id = reader.read_bits(2)? as u8;
        if reader.read_bits(3)? != 0 {
            return Err(CodecError::InvalidData);
        }
    }

    Ok(ObuHeader {
        obu_type,
        obu_ext_flag,
        obu_has_size_field,
        temporal_id,
        spatial_id,
    })
}

/// Parse the length-delimited OBUs required by the AV1 ISO BMFF binding.
/// Unknown/reserved types are bounded and skipped, not interpreted.
pub fn validate_sample(data: &[u8]) -> CodecResult<usize> {
    if data.is_empty() {
        return Err(CodecError::InvalidData);
    }
    let mut reader = BitReader::new(data);
    let mut count = 0;
    while reader.bits_left() != 0 {
        let header = parse_obu_header(&mut reader)?;
        if !header.obu_has_size_field {
            return Err(CodecError::InvalidData);
        }
        let size = usize::try_from(reader.read_leb128()?).map_err(|_| CodecError::InvalidData)?;
        let payload = reader.read_bytes(size)?;
        if header.obu_type == ObuType::SequenceHeader {
            if header.temporal_id != 0 || header.spatial_id != 0 {
                return Err(CodecError::InvalidData);
            }
            crate::parse_sequence_header(&mut BitReader::new(payload))?;
        } else if header.obu_type == ObuType::TemporalDelimiter && !payload.is_empty() {
            return Err(CodecError::InvalidData);
        }
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reserved_types_are_skipped_within_bounds() {
        for kind in [0u8, 9, 10, 11, 12, 13, 14] {
            assert_eq!(validate_sample(&[(kind << 3) | 2, 1, 0x80]).unwrap(), 1);
        }
    }
    #[test]
    fn malformed_headers_and_boundaries() {
        for data in [
            &[0x83, 0][..],
            &[0x03, 0],
            &[0x06, 1, 0],
            &[0x0a, 8, 0],
            &[0x08],
            &[0x0a, 0],
            &[],
            &[0x4a, 0x80],
            &[0x12, 1, 0],
        ] {
            assert!(validate_sample(data).is_err(), "accepted {data:?}");
        }
        assert_eq!(validate_sample(&[0x12, 0, 0x4a, 1, 0x80]).unwrap(), 2);
    }
}

/// Fully parsed intra headers and validated tile byte boundaries. Tile contents
/// remain opaque; this result does not certify entropy syntax or decoded pixels.
#[derive(Debug, Clone)]
pub struct SampleValidation {
    pub obu_count: usize,
    pub frame_headers: Vec<crate::frame::FrameHeader>,
    /// Byte ranges into the original sample, one per tile in this call.
    pub tile_ranges: Vec<std::ops::Range<usize>>,
}

/// Transactional, strict header validation for the implemented intra subset.
/// The structural-only `validate_sample` API remains available and unchanged.
pub fn validate_sample_with_state(
    data: &[u8],
    state: &mut crate::DecoderState,
) -> CodecResult<SampleValidation> {
    use crate::frame::{parse_uncompressed_header, ParserState};
    if data.is_empty() {
        return Err(CodecError::InvalidData);
    }
    let mut next = state.clone();
    let mut reader = BitReader::new(data);
    let mut result = SampleValidation {
        obu_count: 0,
        frame_headers: Vec::new(),
        tile_ranges: Vec::new(),
    };
    while reader.bits_left() != 0 {
        let obu = parse_obu_header(&mut reader)?;
        if !obu.obu_has_size_field {
            return Err(CodecError::InvalidData);
        }
        if (obu.temporal_id != 0 || obu.spatial_id != 0)
            && !matches!(obu.obu_type, ObuType::Reserved | ObuType::Padding)
        {
            return Err(CodecError::Unsupported);
        }
        let size = usize::try_from(reader.read_leb128()?).map_err(|_| CodecError::InvalidData)?;
        let payload_offset = data.len() - reader.bits_left() / 8;
        let payload = reader.read_bytes(size)?;
        let mut r = BitReader::new(payload);
        match obu.obu_type {
            ObuType::SequenceHeader => {
                if !matches!(next.parser, ParserState::AwaitingHeader) {
                    return Err(CodecError::InvalidData);
                }
                let sequence = crate::parse_sequence_header(&mut r)?;
                if next.sequence.as_ref() != Some(&sequence) {
                    next.references.fill(None);
                    next.pictures.reset();
                    next.picture_pending = false;
                    next.last_header = None;
                }
                next.sequence = Some(sequence);
            }
            ObuType::TemporalDelimiter => {
                if !payload.is_empty() || !matches!(next.parser, ParserState::AwaitingHeader) {
                    return Err(CodecError::InvalidData);
                }
                next.temporal_unit = next
                    .temporal_unit
                    .checked_add(1)
                    .ok_or(CodecError::InvalidData)?;
            }
            ObuType::FrameHeader | ObuType::Frame | ObuType::RedundantFrameHeader => {
                let sequence = next.sequence.as_ref().ok_or(CodecError::InvalidData)?;
                let header = parse_uncompressed_header(&mut r, sequence)?;
                if obu.obu_type == ObuType::RedundantFrameHeader {
                    match &next.parser {
                        ParserState::AwaitingTiles {
                            header: saved,
                            header_payload,
                            ..
                        } if **saved == header
                            && same_header_bits(header_payload, payload, header.header_bits) => {}
                        _ => return Err(CodecError::InvalidData),
                    }
                    r.read_trailing_bits()?;
                } else {
                    if !matches!(next.parser, ParserState::AwaitingHeader) {
                        return Err(CodecError::InvalidData);
                    }
                    if let Some(previous) = &next.last_header {
                        if sequence.frame_id_length > 0
                            && !(header.frame_type == crate::frame::FrameType::Key
                                && header.show_frame)
                        {
                            let modulus = 1u32 << sequence.frame_id_length;
                            let difference =
                                (header.frame_id + modulus - previous.frame_id) % modulus;
                            if difference == 0 || difference >= modulus / 2 {
                                return Err(CodecError::InvalidData);
                            }
                        }
                    } else if header.frame_type != crate::frame::FrameType::Key
                        || !header.show_frame
                    {
                        return Err(CodecError::InvalidData);
                    }
                    next.begin(&header);
                    result.frame_headers.push(header.clone());
                    if obu.obu_type == ObuType::Frame {
                        r.read_byte_alignment()?;
                        let header_bytes = payload.len() - r.bits_left() / 8;
                        let (end, ranges) =
                            header.tiles.group(&payload[header_bytes..], 0, true)?;
                        if end != header.tiles.count() {
                            return Err(CodecError::InvalidData);
                        }
                        for range in ranges {
                            result.tile_ranges.push(
                                (payload_offset + header_bytes + range.start)
                                    ..(payload_offset + header_bytes + range.end),
                            );
                        }
                        next.complete(header);
                    } else {
                        r.read_trailing_bits()?;
                        next.parser = ParserState::AwaitingTiles {
                            header: Box::new(header.clone()),
                            next_tile: 0,
                            header_payload: payload[..header.header_bits.div_ceil(8)].to_vec(),
                        };
                    }
                }
            }
            ObuType::TileGroup => {
                let (header, next_tile, header_payload) = match &next.parser {
                    ParserState::AwaitingTiles {
                        header,
                        next_tile,
                        header_payload,
                    } => (header.clone(), *next_tile, header_payload.clone()),
                    _ => return Err(CodecError::InvalidData),
                };
                let (end, ranges) = header.tiles.group(payload, next_tile, false)?;
                for range in ranges {
                    result
                        .tile_ranges
                        .push((payload_offset + range.start)..(payload_offset + range.end));
                }
                if end == header.tiles.count() {
                    next.complete(*header);
                } else {
                    next.parser = ParserState::AwaitingTiles {
                        header,
                        next_tile: end,
                        header_payload,
                    };
                }
            }
            // Metadata semantics and large-scale tile lists are outside this subset.
            ObuType::Metadata | ObuType::TileList => return Err(CodecError::Unsupported),
            ObuType::Reserved | ObuType::Padding => (),
        }
        result.obu_count += 1;
    }
    *state = next;
    Ok(result)
}

fn same_header_bits(a: &[u8], b: &[u8], bits: usize) -> bool {
    let bytes = bits / 8;
    if a.get(..bytes) != b.get(..bytes) {
        return false;
    }
    let rem = bits % 8;
    rem == 0
        || match (a.get(bytes), b.get(bytes)) {
            (Some(a), Some(b)) => (a >> (8 - rem)) == (b >> (8 - rem)),
            _ => false,
        }
}
