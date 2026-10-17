//! Lock-free diagonal wave-front superblock scheduler (`SuperblockWavefrontGrid`).

use std::sync::atomic::{AtomicU32, Ordering};

/// Coordinates multi-threaded diagonal wave-front decoding of superblock rows.
/// Superblock `(sb_row, sb_col)` is ready when:
/// - `sb_row == 0 && sb_col == 0`, OR
/// - `sb_row == 0 && completed_cols[0] >= sb_col`, OR
/// - `sb_row > 0 && completed_cols[sb_row] >= sb_col && completed_cols[sb_row - 1] >= (sb_col + 2).min(sb_cols)`.
#[derive(Debug)]
pub struct SuperblockWavefrontGrid {
    pub threads: usize,
    pub sb_cols: u32,
    pub sb_rows: u32,
    completed_cols: Vec<AtomicU32>,
}

impl SuperblockWavefrontGrid {
    pub fn new(threads: usize, sb_cols: u32, sb_rows: u32) -> Self {
        let rows = sb_rows.max(1) as usize;
        let mut completed_cols = Vec::with_capacity(rows);
        for _ in 0..rows {
            completed_cols.push(AtomicU32::new(0));
        }
        Self {
            threads: threads.max(1),
            sb_cols: sb_cols.max(1),
            sb_rows: sb_rows.max(1),
            completed_cols,
        }
    }

    pub fn is_sb_ready(&self, _tile_id: u32, sb_row: u32, sb_col: u32) -> bool {
        if sb_row >= self.sb_rows || sb_col >= self.sb_cols {
            return false;
        }
        let row_idx = sb_row as usize;
        let cur_done = self.completed_cols[row_idx].load(Ordering::Acquire);
        if cur_done < sb_col {
            return false;
        }
        if sb_row == 0 {
            return true;
        }
        let above_done = self.completed_cols[row_idx - 1].load(Ordering::Acquire);
        let required_above = (sb_col + 2).min(self.sb_cols);
        above_done >= required_above
    }

    pub fn mark_sb_completed(&self, _tile_id: u32, sb_row: u32, sb_col: u32) {
        if let Some(slot) = self.completed_cols.get(sb_row as usize) {
            slot.fetch_max(sb_col + 1, Ordering::AcqRel);
        }
    }
}
