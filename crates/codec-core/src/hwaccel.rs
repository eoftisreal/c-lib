//! Khronos `VK_KHR_video_decode_av1` hardware acceleration descriptors and validation.

use crate::error::{CodecError, CodecResult};

pub const VK_VIDEO_CODEC_OPERATION_DECODE_AV1_BIT_KHR: u32 = 0x0000_0004;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VkVideoDecodeAV1ProfileInfoKHR {
    pub std_profile: u8,
    pub film_grain_support: bool,
}

impl VkVideoDecodeAV1ProfileInfoKHR {
    pub fn validate(&self) -> CodecResult<()> {
        if self.std_profile > 2 {
            return Err(CodecError::HardwareValidation(
                "VUID-09256: std_profile must be 0, 1, or 2",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VkVideoDecodeAV1PictureInfoKHR {
    pub frame_header_offset: u32,
    pub tile_count: u32,
    pub tile_offsets: Vec<u32>,
    pub tile_sizes: Vec<u32>,
    pub dpb_slot_index: u8,
    pub reference_dpb_slots: [Option<u8>; 7],
    pub film_grain_target_slot: Option<u8>,
}

impl VkVideoDecodeAV1PictureInfoKHR {
    /// Enforces Khronos `VK_KHR_video_decode_av1` validation rules:
    /// - `VUID-09256`: `tile_offsets.len() == tile_count && tile_sizes.len() == tile_count`
    /// - `VUID-10801`: `dpb_slot_index < 16` and reference slots `< 16`
    /// - `VUID-07175`: When film grain is enabled, `film_grain_target_slot` must be distinct from `dpb_slot_index`.
    pub fn validate(&self, bitstream_len: usize) -> CodecResult<()> {
        if self.tile_count == 0
            || self.tile_offsets.len() != self.tile_count as usize
            || self.tile_sizes.len() != self.tile_count as usize
        {
            return Err(CodecError::HardwareValidation(
                "VUID-09256: tile_offsets and tile_sizes lengths must match non-zero tile_count",
            ));
        }
        if self.dpb_slot_index >= 16
            || self
                .reference_dpb_slots
                .iter()
                .flatten()
                .any(|&slot| slot >= 16)
        {
            return Err(CodecError::HardwareValidation(
                "VUID-10801: DPB slot index out of valid range [0..16)",
            ));
        }
        if let Some(fg_slot) = self.film_grain_target_slot {
            if fg_slot >= 16 || fg_slot == self.dpb_slot_index {
                return Err(CodecError::HardwareValidation(
                    "VUID-07175: film_grain_target_slot must be valid and distinct from setup dpb_slot_index",
                ));
            }
        }
        for (&offset, &size) in self.tile_offsets.iter().zip(self.tile_sizes.iter()) {
            let end = (offset as usize).checked_add(size as usize).ok_or(
                CodecError::HardwareValidation("VUID-09256: tile range overflow"),
            )?;
            if size == 0 || end > bitstream_len {
                return Err(CodecError::HardwareValidation(
                    "VUID-09256: tile byte range exceeds bitstream buffer bounds",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VkVideoDecodeInfoKHR {
    pub src_buffer_offset: u64,
    pub src_buffer_range: u64,
    pub dst_image_view: u64,
    pub setup_reference_slot: u8,
    pub av1_picture_info: VkVideoDecodeAV1PictureInfoKHR,
}

impl VkVideoDecodeInfoKHR {
    pub fn validate(&self) -> CodecResult<()> {
        if !self.src_buffer_offset.is_multiple_of(4)
            || self.src_buffer_range == 0
            || self.dst_image_view == 0
        {
            return Err(CodecError::HardwareValidation(
                "VUID-07175: invalid Vulkan buffer alignment or null dst_image_view",
            ));
        }
        let len = usize::try_from(self.src_buffer_range).map_err(|_| {
            CodecError::HardwareValidation("VUID-09256: src_buffer_range exceeds usize")
        })?;
        self.av1_picture_info.validate(len)
    }
}
