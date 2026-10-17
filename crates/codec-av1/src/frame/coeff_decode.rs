//! Normative AV1 transform block coefficient entropy decoding (§5.11.39 - §5.11.45).
use super::partition::BlockSize;
use super::scan::scan_order;
use super::transform::{TxSize, TxType2D};
use crate::entropy::SymbolDecoder;
use codec_core::{CodecError, CodecResult};

const NZ_MAP_CTX_OFFSET_4X4: [usize; 16] = [0, 1, 6, 6, 1, 6, 6, 21, 6, 6, 21, 21, 6, 21, 21, 21];

const EOB_GROUP_START: [usize; 6] = [0, 1, 2, 3, 5, 9];
const EOB_OFFSET_BITS: [usize; 6] = [0, 0, 0, 1, 2, 3];

const SKIP_CONTEXTS: [[u8; 5]; 5] = [
    [1, 2, 2, 2, 3],
    [2, 4, 4, 4, 5],
    [2, 4, 4, 4, 5],
    [2, 4, 4, 4, 5],
    [3, 5, 5, 5, 6],
];

#[inline]
pub fn get_txb_skip_ctx_y(bsize: BlockSize, above: u8, left: u8) -> usize {
    if bsize == BlockSize::Block4x4 {
        0
    } else {
        let top = (above & 0x3F).min(4) as usize;
        let left = (left & 0x3F).min(4) as usize;
        SKIP_CONTEXTS[top][left] as usize
    }
}

#[inline]
pub fn get_txb_skip_ctx_uv(plane_bsize: BlockSize, above: u8, left: u8) -> usize {
    let ctx_base = usize::from(above != 0) + usize::from(left != 0);
    let ctx_offset = if plane_bsize != BlockSize::Block4x4 {
        10
    } else {
        7
    };
    ctx_base + ctx_offset
}

#[inline]
pub fn get_dc_sign_ctx(above: u8, left: u8) -> usize {
    let sign_above = (above >> 6) as usize;
    let sign_left = (left >> 6) as usize;
    let signs = [0i32, -1, 1];
    let dc_sign = signs[sign_above.min(2)] + signs[sign_left.min(2)];
    if dc_sign < 0 {
        1
    } else if dc_sign > 0 {
        2
    } else {
        0
    }
}

#[inline]
pub fn get_lower_levels_ctx_eob(bhl: usize, width: usize, scan_idx: usize) -> usize {
    if scan_idx == 0 {
        0
    } else if scan_idx <= (width << bhl) / 8 {
        1
    } else if scan_idx <= (width << bhl) / 4 {
        2
    } else {
        3
    }
}

#[inline]
pub fn get_br_ctx_eob(pos: usize, bhl: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let col = pos >> bhl;
    let row = pos - (col << bhl);
    if row < 2 && col < 2 {
        7
    } else {
        14
    }
}

#[inline]
pub fn get_lower_levels_ctx_2d(levels: &[u8; 64], coeff_idx: usize, bhl: usize) -> usize {
    if coeff_idx == 0 {
        return 0;
    }
    let p = (coeff_idx >> bhl) * 8 + (coeff_idx & ((1 << bhl) - 1));
    let stride = (1 << bhl) + 4;
    let mut mag = levels[p + stride].min(3) as usize;
    mag += levels[p + 1].min(3) as usize;
    mag += levels[p + stride + 1].min(3) as usize;
    mag += levels[p + 2 * stride].min(3) as usize;
    mag += levels[p + 2].min(3) as usize;
    let ctx = ((mag + 1) >> 1).min(4);
    ctx + NZ_MAP_CTX_OFFSET_4X4[coeff_idx]
}

#[inline]
pub fn get_br_ctx_2d(levels: &[u8; 64], coeff_idx: usize, bhl: usize) -> usize {
    let col = coeff_idx >> bhl;
    let row = coeff_idx - (col << bhl);
    let stride = (1 << bhl) + 4;
    let p = col * stride + row;
    let mut mag = levels[p + 1].min(15) as usize
        + levels[p + stride].min(15) as usize
        + levels[p + stride + 1].min(15) as usize;
    mag = ((mag + 1) >> 1).min(6);
    if coeff_idx == 0 {
        mag
    } else if row < 2 && col < 2 {
        mag + 7
    } else {
        mag + 14
    }
}

fn base_context(levels: &[u8; 64], pos: usize, kind: TxType2D) -> usize {
    if !matches!(kind, TxType2D::VIdtx | TxType2D::HIdtx) {
        return get_lower_levels_ctx_2d(levels, pos, 2);
    }
    let row = pos >> 2;
    let col = pos & 3;
    let p = row * 8 + col;
    let vertical = kind == TxType2D::VIdtx;
    let step = if vertical { 8 } else { 1 };
    // AV1 Sig_Ref_Diff_Offset for horizontal/vertical transform classes.
    let mag = [1, 8, 2 * step, 3 * step, 4 * step]
        .iter()
        .map(|&offset| usize::from(levels[p + offset].min(3)))
        .sum::<usize>();
    let axis = if vertical { row } else { col };
    ((mag + 1) >> 1).min(4) + [26, 31, 36][axis.min(2)]
}

fn br_context(levels: &[u8; 64], pos: usize, kind: TxType2D) -> usize {
    if !matches!(kind, TxType2D::VIdtx | TxType2D::HIdtx) {
        return get_br_ctx_2d(levels, pos, 2);
    }
    let row = pos >> 2;
    let col = pos & 3;
    let p = row * 8 + col;
    let vertical = kind == TxType2D::VIdtx;
    let third = if vertical { 16 } else { 2 };
    let mag = [1, 8, third]
        .iter()
        .map(|&offset| usize::from(levels[p + offset].min(15)))
        .sum::<usize>();
    let offset = if pos == 0 {
        0
    } else if (if vertical { row } else { col }) == 0 {
        7
    } else {
        14
    };
    ((mag + 1) >> 1).min(6) + offset
}

fn read_golomb(dec: &mut SymbolDecoder<'_>) -> CodecResult<u32> {
    let mut x = 1u32;
    let mut length = 0usize;
    loop {
        let bit = dec.read_bool()?;
        length += 1;
        if bit {
            break;
        }
        if length > 20 {
            return Err(CodecError::InvalidData);
        }
    }
    for _ in 0..length - 1 {
        x = (x << 1) | (dec.read_bool()? as u32);
    }
    Ok(x - 1)
}

#[derive(Debug)]
pub struct TileCoeffCdfs<'a> {
    pub txb_skip_cdfs: &'a mut [[[u16; 3]; 13]; 5],
    pub eob_multi16_cdfs: &'a mut [[[u16; 6]; 2]; 2],
    pub eob_extra_cdfs: &'a mut [[[[u16; 3]; 9]; 2]; 5],
    pub base_eob_cdfs: &'a mut [[[[u16; 4]; 4]; 2]; 5],
    pub base_cdfs: &'a mut [[[[u16; 5]; 42]; 2]; 5],
    pub br_cdfs: &'a mut [[[[u16; 5]; 21]; 2]; 5],
    pub dc_sign_cdfs: &'a mut [[[u16; 3]; 3]; 2],
}

/// Decode a 4x4 transform block into 16 quantized transform coefficients in raster order.
/// Returns (coefficients, is_all_zero).
pub fn decode_tx_block_4x4(
    dec: &mut SymbolDecoder<'_>,
    cdfs: &mut TileCoeffCdfs<'_>,
    plane: usize,
    plane_bsize: BlockSize,
    above_ec: u8,
    left_ec: u8,
    skip: bool,
) -> CodecResult<([i32; 16], bool)> {
    let (coefficients, zero, _) = decode_tx_block_4x4_with_type(
        dec,
        cdfs,
        plane,
        plane_bsize,
        above_ec,
        left_ec,
        skip,
        |_| Ok(TxType2D::DctDct),
    )?;
    Ok((coefficients, zero))
}

/// Read transform type after `all_zero` and before EOB, as required by AV1.
/// The caller supplies luma syntax or inferred chroma type. Lossless callers
/// retain the original entry point and do not consume a transform type symbol.
#[allow(clippy::too_many_arguments)]
pub fn decode_tx_block_4x4_with_type<F>(
    dec: &mut SymbolDecoder<'_>,
    cdfs: &mut TileCoeffCdfs<'_>,
    plane: usize,
    plane_bsize: BlockSize,
    above_ec: u8,
    left_ec: u8,
    skip: bool,
    read_type: F,
) -> CodecResult<([i32; 16], bool, TxType2D)>
where
    F: FnOnce(&mut SymbolDecoder<'_>) -> CodecResult<TxType2D>,
{
    let plane_type = if plane == 0 { 0 } else { 1 };
    let txs_ctx = 0; // TX_4X4 entropy context is 0
    let bhl = 2; // log2(height) = 2

    let txb_skip_ctx = if plane == 0 {
        get_txb_skip_ctx_y(plane_bsize, above_ec, left_ec)
    } else {
        get_txb_skip_ctx_uv(plane_bsize, above_ec, left_ec)
    };

    let all_zero = if skip {
        true
    } else {
        dec.read_symbol(&mut cdfs.txb_skip_cdfs[txs_ctx][txb_skip_ctx])? != 0
    };

    if all_zero {
        return Ok(([0i32; 16], true, TxType2D::DctDct));
    }

    let kind = read_type(dec)?;
    if !matches!(
        kind,
        TxType2D::DctDct
            | TxType2D::AdstDct
            | TxType2D::DctAdst
            | TxType2D::AdstAdst
            | TxType2D::Idtx
            | TxType2D::VIdtx
            | TxType2D::HIdtx
    ) {
        return Err(CodecError::Unsupported);
    }
    let scan = scan_order(TxSize::Tx4x4, kind)?;
    let class_1d = usize::from(matches!(kind, TxType2D::VIdtx | TxType2D::HIdtx));

    // EOB bin: 5 symbols (0..4), corresponding to eob_pt 1..5
    let eob_pt = dec.read_symbol(&mut cdfs.eob_multi16_cdfs[plane_type][class_1d])? + 1;
    let eob_offset_bits = EOB_OFFSET_BITS[eob_pt];
    let mut eob_extra = 0usize;
    if eob_offset_bits > 0 {
        let eob_ctx = eob_pt.saturating_sub(3);
        let bit = dec.read_symbol(&mut cdfs.eob_extra_cdfs[txs_ctx][plane_type][eob_ctx])?;
        if bit != 0 {
            eob_extra += 1 << (eob_offset_bits - 1);
        }
        for i in 1..eob_offset_bits {
            let bit = dec.read_bool()?;
            if bit {
                eob_extra += 1 << (eob_offset_bits - 1 - i);
            }
        }
    }

    let eob = EOB_GROUP_START[eob_pt]
        + if EOB_GROUP_START[eob_pt] > 2 {
            eob_extra
        } else {
            0
        };
    if eob == 0 || eob > 16 {
        return Err(CodecError::InvalidData);
    }

    let mut levels = [0u8; 64];

    // Decode non-zero coefficient with scan index eob - 1
    {
        let c = eob - 1;
        let pos = usize::from(scan[c]);
        let coeff_ctx = get_lower_levels_ctx_eob(bhl, 4, c);
        let mut level =
            dec.read_symbol(&mut cdfs.base_eob_cdfs[txs_ctx][plane_type][coeff_ctx])? + 1;
        if level > 2 {
            let br_ctx = br_context(&levels, pos, kind);
            for _ in 0..4 {
                let k = dec.read_symbol(&mut cdfs.br_cdfs[txs_ctx][plane_type][br_ctx])?;
                level += k;
                if k < 3 {
                    break;
                }
            }
        }
        let padded_idx = (pos >> bhl) * 8 + (pos & ((1 << bhl) - 1));
        levels[padded_idx] = level as u8;
    }

    // Decode coefficients in reverse scan order from eob - 2 down to 0
    if eob > 1 {
        for c in (0..eob - 1).rev() {
            let pos = usize::from(scan[c]);
            let coeff_ctx = base_context(&levels, pos, kind);
            let mut level = dec.read_symbol(&mut cdfs.base_cdfs[txs_ctx][plane_type][coeff_ctx])?;
            if level > 2 {
                let br_ctx = br_context(&levels, pos, kind);
                for _ in 0..4 {
                    let k = dec.read_symbol(&mut cdfs.br_cdfs[txs_ctx][plane_type][br_ctx])?;
                    level += k;
                    if k < 3 {
                        break;
                    }
                }
            }
            let padded_idx = (pos >> bhl) * 8 + (pos & ((1 << bhl) - 1));
            levels[padded_idx] = level as u8;
        }
    }

    // Signs and overflow
    let dc_sign_ctx = get_dc_sign_ctx(above_ec, left_ec);
    let mut coeffs = [0i32; 16];

    for (c, &scan_pos) in scan.iter().take(eob).enumerate() {
        let pos = usize::from(scan_pos);
        let padded_idx = (pos >> bhl) * 8 + (pos & ((1 << bhl) - 1));
        let mut level = u32::from(levels[padded_idx]);
        if level > 0 {
            let sign = if c == 0 {
                dec.read_symbol(&mut cdfs.dc_sign_cdfs[plane_type][dc_sign_ctx])? != 0
            } else {
                dec.read_bool()?
            };

            if level >= 15 {
                level += read_golomb(dec)?;
            }

            let val = level as i32;
            coeffs[pos] = if sign { -val } else { val };
        }
    }

    Ok((coeffs, false, kind))
}

#[inline]
pub fn update_entropy_context(coeffs: &[i32; 16]) -> u8 {
    let mut cul_level = 0u32;
    for &c in coeffs {
        cul_level = cul_level.saturating_add(c.unsigned_abs());
    }
    cul_level = cul_level.min(0x3F);
    let dc_val = coeffs[0];
    if dc_val < 0 {
        (cul_level | (1 << 6)) as u8
    } else if dc_val > 0 {
        (cul_level | (2 << 6)) as u8
    } else {
        cul_level as u8
    }
}

#[cfg(test)]
mod lossy_tests {
    use super::*;
    use crate::frame::lossy_tables::*;

    #[test]
    fn independent_libaom_vectors_cover_every_intra_4x4_transform_and_q_bank() {
        let kinds = [
            TxType2D::DctDct,
            TxType2D::AdstDct,
            TxType2D::DctAdst,
            TxType2D::AdstAdst,
            TxType2D::Idtx,
            TxType2D::VIdtx,
            TxType2D::HIdtx,
        ];
        let mut count = 0;
        for line in include_str!("../../../../tests/oracles/lossy_coeff_vectors.txt").lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let q: usize = fields[0].parse().unwrap();
            let kind = kinds[fields[1].parse::<usize>().unwrap()];
            let bytes: Vec<_> = fields[3]
                .as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|hex| u8::from_str_radix(std::str::from_utf8(hex).unwrap(), 16).unwrap())
                .collect();
            let expected: Vec<i32> = fields[4..].iter().map(|s| s.parse().unwrap()).collect();
            let mut skip = TXB_SKIP_BY_Q[q];
            let mut eob = EOB_MULTI16_BY_Q[q];
            let mut extra = EOB_EXTRA_BY_Q[q];
            let mut base_eob = BASE_EOB_BY_Q[q];
            let mut base = BASE_BY_Q[q];
            let mut br = BR_BY_Q[q];
            let mut sign = DC_SIGN_BY_Q[q];
            let mut cdfs = TileCoeffCdfs {
                txb_skip_cdfs: &mut skip,
                eob_multi16_cdfs: &mut eob,
                eob_extra_cdfs: &mut extra,
                base_eob_cdfs: &mut base_eob,
                base_cdfs: &mut base,
                br_cdfs: &mut br,
                dc_sign_cdfs: &mut sign,
            };
            let mut decoder = SymbolDecoder::new(&bytes, false).unwrap();
            let (actual, zero, actual_kind) = decode_tx_block_4x4_with_type(
                &mut decoder,
                &mut cdfs,
                0,
                BlockSize::Block4x4,
                0,
                0,
                false,
                |_| Ok(kind),
            )
            .unwrap();
            assert!(!zero);
            assert_eq!(actual_kind, kind);
            assert_eq!(
                actual.as_slice(),
                expected,
                "vector={count} q={q} kind={kind:?}"
            );
            let mut sentinel = 0u8;
            for _ in 0..8 {
                sentinel = (sentinel << 1) | u8::from(decoder.read_bool().unwrap());
            }
            assert_eq!(sentinel, 0xa5, "vector={count}");
            decoder.finish().unwrap();
            count += 1;
        }
        assert_eq!(count, 84);
    }
}
