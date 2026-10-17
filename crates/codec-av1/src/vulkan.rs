//! Normative AV1 frame header and tile-group mapping to Khronos `VK_KHR_video_decode_av1`.

use crate::frame::FrameHeader;
use crate::SequenceHeader;
use codec_core::{
    CodecError, CodecResult, VkVideoDecodeAV1PictureInfoKHR, VkVideoDecodeAV1ProfileInfoKHR,
    VkVideoDecodeInfoKHR,
};
use std::ops::Range;

pub fn build_vulkan_av1_profile(
    seq: &SequenceHeader,
) -> CodecResult<VkVideoDecodeAV1ProfileInfoKHR> {
    let profile = VkVideoDecodeAV1ProfileInfoKHR {
        std_profile: seq.seq_profile,
        film_grain_support: seq.film_grain_params_present,
    };
    profile.validate()?;
    Ok(profile)
}

pub fn build_vulkan_av1_decode_info(
    header: &FrameHeader,
    tile_ranges: &[Range<usize>],
    bitstream_len: usize,
    dpb_slot_index: u8,
    dst_image_view: u64,
) -> CodecResult<VkVideoDecodeInfoKHR> {
    if tile_ranges.is_empty() {
        return Err(CodecError::InvalidData);
    }
    let mut tile_offsets = Vec::with_capacity(tile_ranges.len());
    let mut tile_sizes = Vec::with_capacity(tile_ranges.len());
    for range in tile_ranges {
        let offset = u32::try_from(range.start).map_err(|_| CodecError::InvalidData)?;
        let size = u32::try_from(range.len()).map_err(|_| CodecError::InvalidData)?;
        tile_offsets.push(offset);
        tile_sizes.push(size);
    }
    let pic_info = VkVideoDecodeAV1PictureInfoKHR {
        frame_header_offset: 0,
        tile_count: u32::try_from(tile_ranges.len()).map_err(|_| CodecError::InvalidData)?,
        tile_offsets,
        tile_sizes,
        dpb_slot_index,
        reference_dpb_slots: [None; 7],
        film_grain_target_slot: None,
    };
    let info = VkVideoDecodeInfoKHR {
        src_buffer_offset: 0,
        src_buffer_range: bitstream_len as u64,
        dst_image_view,
        setup_reference_slot: dpb_slot_index,
        av1_picture_info: pic_info,
    };
    let _ = header.frame_id;
    info.validate()?;
    Ok(info)
}
