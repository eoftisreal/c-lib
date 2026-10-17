//! Tile-local AV1 palette colors, cache and diagonal index-map decoding.
use super::palette_tables::*;
use crate::entropy::SymbolDecoder;
use codec_core::{CodecError, CodecResult};

#[derive(Clone, Copy, Default)]
pub(super) struct Colors {
    pub values: [u16; 8],
    pub len: usize,
}

pub(super) struct PaletteCdfs {
    pub sizes: [[[u16; 8]; 7]; 2],
    colors: [[[[u16; 9]; 5]; 7]; 2],
}
impl Default for PaletteCdfs {
    fn default() -> Self {
        fn padded<const N: usize>(table: [[u16; N]; 5]) -> [[u16; 9]; 5] {
            let mut out = [[0; 9]; 5];
            for (dst, src) in out.iter_mut().zip(table) {
                dst[..N].copy_from_slice(&src);
            }
            out
        }
        let colors = [
            [
                padded(PALETTE_Y_COLOR_2),
                padded(PALETTE_Y_COLOR_3),
                padded(PALETTE_Y_COLOR_4),
                padded(PALETTE_Y_COLOR_5),
                padded(PALETTE_Y_COLOR_6),
                padded(PALETTE_Y_COLOR_7),
                padded(PALETTE_Y_COLOR_8),
            ],
            [
                padded(PALETTE_UV_COLOR_2),
                padded(PALETTE_UV_COLOR_3),
                padded(PALETTE_UV_COLOR_4),
                padded(PALETTE_UV_COLOR_5),
                padded(PALETTE_UV_COLOR_6),
                padded(PALETTE_UV_COLOR_7),
                padded(PALETTE_UV_COLOR_8),
            ],
        ];
        Self {
            sizes: [PALETTE_Y_SIZE, PALETTE_UV_SIZE],
            colors,
        }
    }
}

pub(super) fn read_colors(
    dec: &mut SymbolDecoder<'_>,
    size: usize,
    bit_depth: u8,
    above: Colors,
    left: Colors,
    luma: bool,
) -> CodecResult<Colors> {
    let mut cache = Vec::with_capacity(16);
    cache.extend_from_slice(&above.values[..above.len]);
    cache.extend_from_slice(&left.values[..left.len]);
    cache.sort_unstable();
    cache.dedup();
    let mut result = Colors {
        len: size,
        ..Colors::default()
    };
    let mut idx = 0;
    for value in cache {
        if idx == size {
            break;
        }
        if dec.read_bool()? {
            result.values[idx] = value;
            idx += 1;
        }
    }
    if idx < size {
        result.values[idx] = dec.read_literal(bit_depth)? as u16;
        idx += 1;
    }
    let max = (1u32 << bit_depth) - 1;
    let mut bits = if idx < size {
        bit_depth - 3 + dec.read_literal(2)? as u8
    } else {
        0
    };
    while idx < size {
        let delta = dec.read_literal(bits)? + u32::from(luma);
        let value = (u32::from(result.values[idx - 1]) + delta).min(max);
        result.values[idx] = value as u16;
        let range = max + u32::from(!luma) - value;
        let ceil_log2 = if range <= 1 {
            0
        } else {
            (32 - (range - 1).leading_zeros()) as u8
        };
        bits = bits.min(ceil_log2);
        idx += 1;
    }
    result.values[..size].sort_unstable();
    Ok(result)
}

pub(super) fn read_v_colors(
    dec: &mut SymbolDecoder<'_>,
    size: usize,
    bit_depth: u8,
) -> CodecResult<Colors> {
    let mut result = Colors {
        len: size,
        ..Colors::default()
    };
    if dec.read_bool()? {
        let bits = bit_depth - 4 + dec.read_literal(2)? as u8;
        result.values[0] = dec.read_literal(bit_depth)? as u16;
        for i in 1..size {
            let mut delta = dec.read_literal(bits)? as i32;
            if delta != 0 && dec.read_bool()? {
                delta = -delta;
            }
            result.values[i] =
                (i32::from(result.values[i - 1]) + delta).rem_euclid(1 << bit_depth) as u16;
        }
    } else {
        for value in &mut result.values[..size] {
            *value = dec.read_literal(bit_depth)? as u16;
        }
    }
    Ok(result)
}

fn color_context(
    map: &[u8],
    stride: usize,
    y: usize,
    x: usize,
    size: usize,
) -> CodecResult<(usize, [usize; 8])> {
    let mut scores = [0u8; 8];
    let mut order = [0, 1, 2, 3, 4, 5, 6, 7];
    if x > 0 {
        scores[usize::from(map[y * stride + x - 1])] += 2;
    }
    if x > 0 && y > 0 {
        scores[usize::from(map[(y - 1) * stride + x - 1])] += 1;
    }
    if y > 0 {
        scores[usize::from(map[(y - 1) * stride + x])] += 2;
    }
    // Stable selection of the three highest scores; ties retain color order.
    for i in 0..3 {
        let mut best = i;
        for j in i + 1..size {
            if scores[j] > scores[best] {
                best = j;
            }
        }
        scores[i..=best].rotate_right(1);
        order[i..=best].rotate_right(1);
    }
    let hash = usize::from(scores[0] + 2 * scores[1] + 2 * scores[2]);
    let ctx = match hash {
        2 => 0,
        5 => 4,
        6 => 3,
        7 => 2,
        8 => 1,
        _ => return Err(CodecError::InvalidData),
    };
    Ok((ctx, order))
}

impl PaletteCdfs {
    pub fn read_map(
        &mut self,
        dec: &mut SymbolDecoder<'_>,
        plane: usize,
        size: usize,
        coded: (usize, usize),
        visible: (usize, usize),
    ) -> CodecResult<Vec<u8>> {
        let (width, height) = coded;
        let (visible_width, visible_height) = visible;
        let mut map = vec![0; width * height];
        let bits = (usize::BITS - size.leading_zeros()) as u8;
        let m = (1 << bits) - size;
        let v = dec.read_literal(bits - 1)? as usize;
        map[0] = if v < m {
            v
        } else {
            (v << 1) - m + usize::from(dec.read_bool()?)
        } as u8;
        for diagonal in 1..visible_width + visible_height - 1 {
            let first = diagonal.saturating_sub(visible_height - 1);
            let last = diagonal.min(visible_width - 1);
            for x in (first..=last).rev() {
                let y = diagonal - x;
                let (ctx, order) = color_context(&map, width, y, x, size)?;
                let sym = dec.read_symbol(&mut self.colors[plane][size - 2][ctx][..size + 1])?;
                map[y * width + x] = order[sym] as u8;
            }
        }
        for y in 0..visible_height {
            let last = map[y * width + visible_width - 1];
            map[y * width + visible_width..(y + 1) * width].fill(last);
        }
        for y in visible_height..height {
            map.copy_within(
                (visible_height - 1) * width..visible_height * width,
                y * width,
            );
        }
        Ok(map)
    }
}

pub(super) fn predict(colors: Colors, map: &[u8], stride: usize, x: usize, y: usize) -> [u16; 16] {
    let mut out = [0; 16];
    for dy in 0..4 {
        for dx in 0..4 {
            out[dy * 4 + dx] = colors.values[usize::from(map[(y + dy) * stride + x + dx])];
        }
    }
    out
}
