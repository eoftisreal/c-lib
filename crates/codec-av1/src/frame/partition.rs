//! AV1 10-way recursive superblock partition tree (`PartitionType`, `BlockSize`, `CodingLeafBlock`).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionType {
    None = 0,
    Horizontal = 1,
    Vertical = 2,
    Split = 3,
    HorzA = 4,
    HorzB = 5,
    VertA = 6,
    VertB = 7,
    Horz4 = 8,
    Vert4 = 9,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockSize {
    Block4x4,
    Block4x8,
    Block8x4,
    Block8x8,
    Block8x16,
    Block16x8,
    Block16x16,
    Block16x32,
    Block32x16,
    Block32x32,
    Block32x64,
    Block64x32,
    Block64x64,
    Block64x128,
    Block128x64,
    Block128x128,
    Block4x16,
    Block16x4,
    Block8x32,
    Block32x8,
    Block16x64,
    Block64x16,
}

impl BlockSize {
    pub const fn dimensions_px(self) -> (u32, u32) {
        match self {
            Self::Block4x4 => (4, 4),
            Self::Block4x8 => (4, 8),
            Self::Block8x4 => (8, 4),
            Self::Block8x8 => (8, 8),
            Self::Block8x16 => (8, 16),
            Self::Block16x8 => (16, 8),
            Self::Block16x16 => (16, 16),
            Self::Block16x32 => (16, 32),
            Self::Block32x16 => (32, 16),
            Self::Block32x32 => (32, 32),
            Self::Block32x64 => (32, 64),
            Self::Block64x32 => (64, 32),
            Self::Block64x64 => (64, 64),
            Self::Block64x128 => (64, 128),
            Self::Block128x64 => (128, 64),
            Self::Block128x128 => (128, 128),
            Self::Block4x16 => (4, 16),
            Self::Block16x4 => (16, 4),
            Self::Block8x32 => (8, 32),
            Self::Block32x8 => (32, 8),
            Self::Block16x64 => (16, 64),
            Self::Block64x16 => (64, 16),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodingLeafBlock {
    pub mi_row: u32,
    pub mi_col: u32,
    pub bsize: BlockSize,
    pub segment_id: u8,
    pub skip: bool,
}

impl BlockSize {
    pub fn from_dimensions_px(width: u32, height: u32) -> codec_core::CodecResult<Self> {
        use BlockSize::*;
        const ALL: [BlockSize; 22] = [
            Block4x4,
            Block4x8,
            Block8x4,
            Block8x8,
            Block8x16,
            Block16x8,
            Block16x16,
            Block16x32,
            Block32x16,
            Block32x32,
            Block32x64,
            Block64x32,
            Block64x64,
            Block64x128,
            Block128x64,
            Block128x128,
            Block4x16,
            Block16x4,
            Block8x32,
            Block32x8,
            Block16x64,
            Block64x16,
        ];
        ALL.into_iter()
            .find(|b| b.dimensions_px() == (width, height))
            .ok_or(codec_core::CodecError::InvalidData)
    }
}

impl TryFrom<usize> for PartitionType {
    type Error = codec_core::CodecError;
    fn try_from(value: usize) -> Result<Self, Self::Error> {
        use PartitionType::*;
        [
            None, Horizontal, Vertical, Split, HorzA, HorzB, VertA, VertB, Horz4, Vert4,
        ]
        .get(value)
        .copied()
        .ok_or(codec_core::CodecError::InvalidData)
    }
}

/// Adaptive tile-local partition CDF banks. Clone the selected frame context for
/// each tile; only the designated context-update tile may be saved for reuse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartitionCdfs {
    w8: [[u16; 5]; 4],
    w16: [[u16; 11]; 4],
    w32: [[u16; 11]; 4],
    w64: [[u16; 11]; 4],
    w128: [[u16; 9]; 4],
}
impl Default for PartitionCdfs {
    fn default() -> Self {
        use super::partition_tables::*;
        Self {
            w8: PARTITION_W8,
            w16: PARTITION_W16,
            w32: PARTITION_W32,
            w64: PARTITION_W64,
            w128: PARTITION_W128,
        }
    }
}
impl PartitionCdfs {
    pub fn read_partition(
        &mut self,
        decoder: &mut crate::entropy::SymbolDecoder<'_>,
        request: super::partition_decode::PartitionRequest,
    ) -> codec_core::CodecResult<PartitionType> {
        use codec_core::CodecError;
        let (width, height) = request.block.bsize.dimensions_px();
        if width != height || request.context > 3 {
            return Err(CodecError::InvalidData);
        }
        let cdf: &mut [u16] = match width {
            8 => &mut self.w8[request.context],
            16 => &mut self.w16[request.context],
            32 => &mut self.w32[request.context],
            64 => &mut self.w64[request.context],
            128 => &mut self.w128[request.context],
            _ => return Err(CodecError::InvalidData),
        };
        if !request.has_rows && !request.has_cols {
            return Ok(PartitionType::Split);
        }
        // AV1 MiRows/MiCols are even: partial 8x8 nodes cannot occur.
        if width == 8 && (!request.has_rows || !request.has_cols) {
            return Err(CodecError::InvalidData);
        }
        if request.has_rows && request.has_cols {
            return PartitionType::try_from(decoder.read_symbol(cdf)?);
        }
        let indices: &[usize] = if !request.has_rows {
            &[2, 3, 4, 6, 7, 9]
        } else {
            &[1, 3, 4, 5, 6, 8]
        };
        let psum: u16 = indices
            .iter()
            .filter(|&&i| i < cdf.len() - 1)
            .map(|&i| cdf[i] - cdf[i - 1])
            .sum();
        // This temporary binary CDF is not adapted back into the partition bank.
        let split = decoder.read_symbol(&mut [32768 - psum, 32768, 0])? != 0;
        Ok(if split {
            PartitionType::Split
        } else if !request.has_rows {
            PartitionType::Horizontal
        } else {
            PartitionType::Vertical
        })
    }
}
