//! Tile-local recursive partition traversal (AV1 decode_partition).
//!
//! `PartitionSyntax` supplies entropy symbols and decodes each leaf immediately:
//! block syntax is interleaved with partition syntax, never a second pass over a
//! precomputed tree. This module does not invent skip flags or prediction modes.
use super::partition::{BlockSize, PartitionType};
use codec_core::{CodecError, CodecResult};

/// Coded geometry, in 4x4 luma units. At the image edge the coded block may extend
/// beyond MiRows/MiCols; callers clip writes, never change the coded block size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockGeometry {
    pub mi_row: u32,
    pub mi_col: u32,
    pub bsize: BlockSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartitionRequest {
    pub block: BlockGeometry,
    /// left * 2 + above, with smaller neighboring height/width respectively.
    pub context: usize,
    pub has_rows: bool,
    pub has_cols: bool,
}

pub trait PartitionSyntax {
    fn read_partition(&mut self, request: PartitionRequest) -> CodecResult<PartitionType>;
    fn decode_leaf(&mut self, block: BlockGeometry) -> CodecResult<()>;
}

/// One tile's geometry and above/left contexts; no state crosses tile boundaries.
/// After a callback failure the tile is poisoned: discard it and its entropy/CDF
/// state. Callbacks may already have consumed syntax or modified private pixels.
pub struct PartitionTraversal {
    row_start: u32,
    col_start: u32,
    row_end: u32,
    col_end: u32,
    next_row: u32,
    next_col: u32,
    sb_mi: u32,
    above_width: Vec<u32>,
    left_height: Vec<u32>,
    failed: bool,
}

impl PartitionTraversal {
    /// MiRows/MiCols are AV1's padded dimensions (even numbers of 4x4 units).
    /// Interior tile boundaries must coincide with superblock boundaries.
    pub fn new(
        mi_rows: u32,
        mi_cols: u32,
        row_bounds: (u32, u32),
        col_bounds: (u32, u32),
        use_128x128: bool,
    ) -> CodecResult<Self> {
        let (row_start, row_end) = row_bounds;
        let (col_start, col_end) = col_bounds;
        let sb_mi = if use_128x128 { 32 } else { 16 };
        if mi_rows == 0
            || mi_cols == 0
            || mi_rows > 16384
            || mi_cols > 16384
            || !mi_rows.is_multiple_of(2)
            || !mi_cols.is_multiple_of(2)
            || row_start >= row_end
            || col_start >= col_end
            || row_end > mi_rows
            || col_end > mi_cols
            || row_start % sb_mi != 0
            || col_start % sb_mi != 0
            || (row_end != mi_rows && row_end % sb_mi != 0)
            || (col_end != mi_cols && col_end % sb_mi != 0)
        {
            return Err(CodecError::InvalidData);
        }
        Ok(Self {
            row_start,
            col_start,
            row_end,
            col_end,
            next_row: row_start,
            next_col: col_start,
            sb_mi,
            above_width: vec![0; (col_end - col_start) as usize],
            left_height: vec![0; (row_end - row_start) as usize],
            failed: false,
        })
    }

    /// Decode the next superblock in raster order. Returns false only when this
    /// tile is complete. A leaf callback must fully decode that block before return.
    pub fn decode_next(&mut self, syntax: &mut impl PartitionSyntax) -> CodecResult<bool> {
        if self.failed {
            return Err(CodecError::InvalidData);
        }
        if self.next_row >= self.row_end {
            return Ok(false);
        }
        if let Err(error) = self.walk(self.next_row, self.next_col, self.sb_mi, syntax) {
            self.failed = true;
            return Err(error);
        }
        self.next_col += self.sb_mi;
        if self.next_col >= self.col_end {
            self.next_col = self.col_start;
            self.next_row += self.sb_mi;
        }
        Ok(true)
    }

    fn leaf(
        &mut self,
        r: u32,
        c: u32,
        w: u32,
        h: u32,
        syntax: &mut impl PartitionSyntax,
    ) -> CodecResult<()> {
        if r >= self.row_end || c >= self.col_end {
            return Ok(());
        }
        let bsize = BlockSize::from_dimensions_px(w * 4, h * 4)?;
        syntax.decode_leaf(BlockGeometry {
            mi_row: r,
            mi_col: c,
            bsize,
        })?;
        self.above_width[(c - self.col_start) as usize
            ..(c + w).min(self.col_end).saturating_sub(self.col_start) as usize]
            .fill(w);
        self.left_height[(r - self.row_start) as usize
            ..(r + h).min(self.row_end).saturating_sub(self.row_start) as usize]
            .fill(h);
        Ok(())
    }

    fn walk(
        &mut self,
        r: u32,
        c: u32,
        size: u32,
        syntax: &mut impl PartitionSyntax,
    ) -> CodecResult<()> {
        if r >= self.row_end || c >= self.col_end {
            return Ok(());
        }
        // size strictly halves at each recursive call: at most six levels.
        if size == 1 {
            return self.leaf(r, c, size, size, syntax);
        }
        let half = size / 2;
        let has_rows = r + half < self.row_end;
        let has_cols = c + half < self.col_end;
        let above = r > self.row_start && self.above_width[(c - self.col_start) as usize] < size;
        let left = c > self.col_start && self.left_height[(r - self.row_start) as usize] < size;
        let request = PartitionRequest {
            block: BlockGeometry {
                mi_row: r,
                mi_col: c,
                bsize: BlockSize::from_dimensions_px(size * 4, size * 4)?,
            },
            context: usize::from(left) * 2 + usize::from(above),
            has_rows,
            has_cols,
        };
        let p = if !has_rows && !has_cols {
            PartitionType::Split
        } else {
            syntax.read_partition(request)?
        };
        if !request.allows(p) {
            return Err(CodecError::InvalidData);
        }
        use PartitionType::*;
        match p {
            None => self.leaf(r, c, size, size, syntax)?,
            Horizontal => {
                self.leaf(r, c, size, half, syntax)?;
                if has_rows {
                    self.leaf(r + half, c, size, half, syntax)?;
                }
            }
            Vertical => {
                self.leaf(r, c, half, size, syntax)?;
                if has_cols {
                    self.leaf(r, c + half, half, size, syntax)?;
                }
            }
            Split => {
                self.walk(r, c, half, syntax)?;
                self.walk(r, c + half, half, syntax)?;
                self.walk(r + half, c, half, syntax)?;
                self.walk(r + half, c + half, half, syntax)?;
            }
            HorzA => {
                self.leaf(r, c, half, half, syntax)?;
                self.leaf(r, c + half, half, half, syntax)?;
                self.leaf(r + half, c, size, half, syntax)?;
            }
            HorzB => {
                self.leaf(r, c, size, half, syntax)?;
                self.leaf(r + half, c, half, half, syntax)?;
                self.leaf(r + half, c + half, half, half, syntax)?;
            }
            VertA => {
                self.leaf(r, c, half, half, syntax)?;
                self.leaf(r + half, c, half, half, syntax)?;
                self.leaf(r, c + half, half, size, syntax)?;
            }
            VertB => {
                self.leaf(r, c, half, size, syntax)?;
                self.leaf(r, c + half, half, half, syntax)?;
                self.leaf(r + half, c + half, half, half, syntax)?;
            }
            Horz4 | Vert4 => {
                for i in 0..4 {
                    if p == Horz4 {
                        self.leaf(r + i * size / 4, c, size, size / 4, syntax)?;
                    } else {
                        self.leaf(r, c + i * size / 4, size / 4, size, syntax)?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl PartitionRequest {
    pub fn allows(self, p: PartitionType) -> bool {
        let (w, h) = self.block.bsize.dimensions_px();
        if w != h || w < 8 || self.context > 3 {
            return false;
        }
        if !self.has_rows && !self.has_cols {
            return p == PartitionType::Split;
        }
        if !self.has_rows {
            return matches!(p, PartitionType::Horizontal | PartitionType::Split);
        }
        if !self.has_cols {
            return matches!(p, PartitionType::Vertical | PartitionType::Split);
        }
        (w != 8 || (p as u8) < 4) && (w != 128 || (p as u8) < 8)
    }
}
