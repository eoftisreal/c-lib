use super::dimensions::Dimensions;
use codec_bitstream::BitReader;
use codec_core::{CodecError, CodecResult, SpatialSubregion, SubregionDescriptor};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileInfo {
    pub col_starts: Vec<u32>,
    pub row_starts: Vec<u32>,
    pub cols_log2: u8,
    pub rows_log2: u8,
    pub context_update_tile_id: u32,
    pub tile_size_bytes: u8,
}
fn tile_log2(block: u32, target: u32) -> u8 {
    let mut k = 0;
    while (block << k) < target {
        k += 1;
    }
    k
}
impl TileInfo {
    pub fn count(&self) -> u32 {
        ((self.col_starts.len() - 1) * (self.row_starts.len() - 1)) as u32
    }
    pub(crate) fn parse(r: &mut BitReader, d: &Dimensions, sb128: bool) -> CodecResult<Self> {
        let shift = if sb128 { 5 } else { 4 };
        let cols = d.mi_cols().div_ceil(1 << shift);
        let rows = d.mi_rows().div_ceil(1 << shift);
        let max_width = 4096 >> (shift + 2);
        let max_area = (4096 * 2304) >> (2 * (shift + 2));
        let min_cols = tile_log2(max_width, cols);
        let max_cols = tile_log2(1, cols.min(64));
        let max_rows = tile_log2(1, rows.min(64));
        let min_tiles = min_cols.max(tile_log2(max_area, rows * cols));
        let (mut col_starts, mut row_starts) = (Vec::new(), Vec::new());
        let (cols_log2, rows_log2);
        if r.read_bit()? {
            let mut c = min_cols;
            while c < max_cols && r.read_bit()? {
                c += 1;
            }
            let width = cols.div_ceil(1 << c);
            for start in (0..cols).step_by(width as usize) {
                col_starts.push(start << shift);
            }
            let mut h = min_tiles.saturating_sub(c);
            while h < max_rows && r.read_bit()? {
                h += 1;
            }
            let height = rows.div_ceil(1 << h);
            for start in (0..rows).step_by(height as usize) {
                row_starts.push(start << shift);
            }
            cols_log2 = c;
            rows_log2 = h;
        } else {
            let mut start = 0;
            let mut widest = 0;
            while start < cols {
                if col_starts.len() == 64 {
                    return Err(CodecError::InvalidData);
                }
                col_starts.push(start << shift);
                let width = r.read_ns((cols - start).min(max_width))? + 1;
                widest = widest.max(width);
                start += width;
            }
            cols_log2 = tile_log2(1, col_starts.len() as u32);
            let area = if min_tiles > 0 {
                (rows * cols) >> (min_tiles + 1)
            } else {
                rows * cols
            };
            let max_height = (area / widest).max(1);
            start = 0;
            while start < rows {
                if row_starts.len() == 64 {
                    return Err(CodecError::InvalidData);
                }
                row_starts.push(start << shift);
                start += r.read_ns((rows - start).min(max_height))? + 1;
            }
            rows_log2 = tile_log2(1, row_starts.len() as u32);
        }
        col_starts.push(d.mi_cols());
        row_starts.push(d.mi_rows());
        let (context_update_tile_id, tile_size_bytes) = if cols_log2 + rows_log2 > 0 {
            (
                r.read_bits(cols_log2 + rows_log2)?,
                r.read_bits(2)? as u8 + 1,
            )
        } else {
            (0, 0)
        };
        let out = Self {
            col_starts,
            row_starts,
            cols_log2,
            rows_log2,
            context_update_tile_id,
            tile_size_bytes,
        };
        if context_update_tile_id >= out.count() {
            return Err(CodecError::InvalidData);
        }
        Ok(out)
    }

    /// Validate framing only. The ranges contain opaque entropy-coded tile data.
    pub(crate) fn group(
        &self,
        payload: &[u8],
        next_tile: u32,
        combined: bool,
    ) -> CodecResult<(u32, Vec<Range<usize>>)> {
        let mut r = BitReader::new(payload);
        let explicit = self.count() > 1 && r.read_bit()?;
        if combined && explicit {
            return Err(CodecError::InvalidData);
        }
        let (start, end) = if explicit {
            (
                r.read_bits(self.cols_log2 + self.rows_log2)?,
                r.read_bits(self.cols_log2 + self.rows_log2)?,
            )
        } else {
            (0, self.count() - 1)
        };
        if start != next_tile || end < start || end >= self.count() {
            return Err(CodecError::InvalidData);
        }
        r.read_byte_alignment()?;
        let mut ranges = Vec::new();
        for tile in start..=end {
            let size = if tile == end {
                r.bits_left() / 8
            } else {
                let mut size = 0u64;
                for i in 0..self.tile_size_bytes {
                    size |= u64::from(r.read_bits(8)?) << (8 * i);
                }
                usize::try_from(size + 1).map_err(|_| CodecError::InvalidData)?
            };
            if size == 0 {
                return Err(CodecError::InvalidData);
            }
            let offset = payload.len() - r.bits_left() / 8;
            r.read_bytes(size)?;
            ranges.push(offset..offset + size);
        }
        Ok((end + 1, ranges))
    }
}

impl SpatialSubregion for TileInfo {
    fn subregions(&self) -> Vec<SubregionDescriptor> {
        let mut out = Vec::with_capacity(self.count() as usize);
        let mut id = 0u32;
        for r in 0..self.row_starts.len().saturating_sub(1) {
            let y_px = self.row_starts[r] * 4;
            let height_px = (self.row_starts[r + 1] - self.row_starts[r]) * 4;
            for c in 0..self.col_starts.len().saturating_sub(1) {
                let x_px = self.col_starts[c] * 4;
                let width_px = (self.col_starts[c + 1] - self.col_starts[c]) * 4;
                out.push(SubregionDescriptor {
                    id,
                    x_px,
                    y_px,
                    width_px,
                    height_px,
                    independent_entropy: true,
                    independent_motion: false,
                });
                id += 1;
            }
        }
        out
    }
}
