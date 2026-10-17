//! Coefficient scans from AV1 get_scan, in specification raster order.
use super::{
    scan_tables::*,
    transform::{TxSize, TxType2D},
};
use codec_core::{CodecError, CodecResult};
pub fn scan_order(size: TxSize, kind: TxType2D) -> CodecResult<&'static [u16]> {
    let (w, h) = size.dimensions();
    if w.max(h) == 64 && kind != TxType2D::DctDct
        || w.max(h) == 32 && !matches!(kind, TxType2D::DctDct | TxType2D::Idtx)
    {
        return Err(CodecError::Unsupported);
    }
    let (w, h) = (w.min(32), h.min(32));
    let row = matches!(
        kind,
        TxType2D::VIdtx | TxType2D::AdstIdtx | TxType2D::FlipAdstIdtx
    );
    let col = matches!(
        kind,
        TxType2D::HIdtx | TxType2D::IdtxAdst | TxType2D::IdtxFlipAdst
    );
    let result: &[u16] = if row {
        match (w, h) {
            (4, 4) => &MROW_SCAN_4X4,
            (4, 8) => &MROW_SCAN_4X8,
            (8, 4) => &MROW_SCAN_8X4,
            (8, 8) => &MROW_SCAN_8X8,
            (8, 16) => &MROW_SCAN_8X16,
            (16, 8) => &MROW_SCAN_16X8,
            (16, 16) => &MROW_SCAN_16X16,
            (4, 16) => &MROW_SCAN_4X16,
            (16, 4) => &MROW_SCAN_16X4,
            _ => return Err(CodecError::Unsupported),
        }
    } else if col {
        match (w, h) {
            (4, 4) => &MCOL_SCAN_4X4,
            (4, 8) => &MCOL_SCAN_4X8,
            (8, 4) => &MCOL_SCAN_8X4,
            (8, 8) => &MCOL_SCAN_8X8,
            (8, 16) => &MCOL_SCAN_8X16,
            (16, 8) => &MCOL_SCAN_16X8,
            (16, 16) => &MCOL_SCAN_16X16,
            (4, 16) => &MCOL_SCAN_4X16,
            (16, 4) => &MCOL_SCAN_16X4,
            _ => return Err(CodecError::Unsupported),
        }
    } else {
        match (w, h) {
            (4, 4) => &DEFAULT_SCAN_4X4,
            (4, 8) => &DEFAULT_SCAN_4X8,
            (8, 4) => &DEFAULT_SCAN_8X4,
            (8, 8) => &DEFAULT_SCAN_8X8,
            (8, 16) => &DEFAULT_SCAN_8X16,
            (16, 8) => &DEFAULT_SCAN_16X8,
            (16, 16) => &DEFAULT_SCAN_16X16,
            (4, 16) => &DEFAULT_SCAN_4X16,
            (16, 4) => &DEFAULT_SCAN_16X4,
            (16, 32) => &DEFAULT_SCAN_16X32,
            (32, 16) => &DEFAULT_SCAN_32X16,
            (8, 32) => &DEFAULT_SCAN_8X32,
            (32, 8) => &DEFAULT_SCAN_32X8,
            (32, 32) => &DEFAULT_SCAN_32X32,
            _ => return Err(CodecError::Unsupported),
        }
    };
    Ok(result)
}
