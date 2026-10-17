//! AV1 intra-frame spatial prediction modes (DC, Directional 56-angle, Smooth, Paeth,
//! recursive Markov FilterIntra, CfL, Palette, and IntraBC).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntraMode {
    DcPred = 0,
    VPred = 1,
    HPred = 2,
    D45Pred = 3,
    D135Pred = 4,
    D113Pred = 5,
    D157Pred = 6,
    D203Pred = 7,
    D67Pred = 8,
    SmoothPred = 9,
    SmoothVPred = 10,
    SmoothHPred = 11,
    PaethPred = 12,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterIntraMode {
    FilterDcPred = 0,
    FilterVPred = 1,
    FilterHPred = 2,
    FilterD157Pred = 3,
    FilterPaethPred = 4,
}

pub fn predict_paeth_sample(left: u16, above: u16, above_left: u16) -> u16 {
    let l = i32::from(left);
    let a = i32::from(above);
    let al = i32::from(above_left);
    let base = l + a - al;
    let dl = (base - l).abs();
    let da = (base - a).abs();
    let dal = (base - al).abs();
    if dl <= da && dl <= dal {
        left
    } else if da <= dal {
        above
    } else {
        above_left
    }
}

/// Scalar DC, V, H, Paeth and smooth predictors for reconstructed neighboring edges.
/// Smooth modes accept AV1 transform shapes with 4..64 samples per side.
/// Callers supply prepared edges; unavailable required edges return Unsupported.
/// Directional and filter-intra modes are not implemented here.
#[allow(clippy::too_many_arguments)]
pub fn predict_basic_block(
    mode: IntraMode,
    dst: &mut [u16],
    stride: usize,
    width: usize,
    height: usize,
    bit_depth: u8,
    above: Option<&[u16]>,
    left: Option<&[u16]>,
    above_left: Option<u16>,
) -> codec_core::CodecResult<()> {
    use codec_core::CodecError;
    if !matches!(bit_depth, 8 | 10 | 12)
        || width == 0
        || height == 0
        || width > 128
        || height > 128
        || stride < width
        || stride.checked_mul(height).is_none_or(|n| n > dst.len())
        || above.is_some_and(|v| v.len() < width)
        || left.is_some_and(|v| v.len() < height)
    {
        return Err(CodecError::InvalidData);
    }
    let max_sample = (1u16 << bit_depth) - 1;
    if above.is_some_and(|v| v[..width].iter().any(|&x| x > max_sample))
        || left.is_some_and(|v| v[..height].iter().any(|&x| x > max_sample))
        || above_left.is_some_and(|v| v > max_sample)
    {
        return Err(CodecError::InvalidData);
    }
    let smooth = matches!(
        mode,
        IntraMode::SmoothPred | IntraMode::SmoothVPred | IntraMode::SmoothHPred
    );
    let (weights_x, weights_y) = if smooth {
        if width.max(height) > 4 * width.min(height) {
            return Err(CodecError::Unsupported);
        }
        (smooth_weights(width)?, smooth_weights(height)?)
    } else {
        (&[][..], &[][..])
    };

    let mut above_storage = [0u16; 128];
    let mut left_storage = [0u16; 128];
    let (eff_above, eff_left, eff_above_left) = match (above, left) {
        (Some(a), Some(l)) => (a, l, above_left.unwrap_or(a[0])),
        (Some(a), None) => {
            let fill = a[0];
            for item in left_storage[..height].iter_mut() {
                *item = fill;
            }
            (a, &left_storage[..height], fill)
        }
        (None, Some(l)) => {
            let fill = l[0];
            for item in above_storage[..width].iter_mut() {
                *item = fill;
            }
            (&above_storage[..width], l, fill)
        }
        (None, None) => {
            let mid_corner = 1u16 << (bit_depth - 1);
            let mid_above = mid_corner - 1;
            let mid_left = mid_corner + 1;
            for item in above_storage[..width].iter_mut() {
                *item = mid_above;
            }
            for item in left_storage[..height].iter_mut() {
                *item = mid_left;
            }
            (&above_storage[..width], &left_storage[..height], mid_corner)
        }
    };

    let dc = match mode {
        IntraMode::DcPred => {
            let mut sum = 0u32;
            let mut count = 0u32;
            if let Some(a) = above {
                sum += a[..width].iter().map(|&v| u32::from(v)).sum::<u32>();
                count += width as u32;
            }
            if let Some(l) = left {
                sum += l[..height].iter().map(|&v| u32::from(v)).sum::<u32>();
                count += height as u32;
            }
            (sum + count / 2)
                .checked_div(count)
                .map(|value| value as u16)
                .unwrap_or(1u16 << (bit_depth - 1))
        }
        IntraMode::SmoothPred | IntraMode::SmoothVPred | IntraMode::SmoothHPred => 0,
        IntraMode::VPred => 0,
        IntraMode::HPred => 0,
        IntraMode::PaethPred => 0,
        _ => return Err(CodecError::Unsupported),
    };
    for y in 0..height {
        for x in 0..width {
            dst[y * stride + x] = match mode {
                IntraMode::DcPred => dc,
                IntraMode::VPred => eff_above[x],
                IntraMode::HPred => eff_left[y],
                IntraMode::SmoothPred | IntraMode::SmoothVPred | IntraMode::SmoothHPred => {
                    let wx = u32::from(weights_x[x]);
                    let wy = u32::from(weights_y[y]);
                    let vertical =
                        wy * u32::from(eff_above[x]) + (256 - wy) * u32::from(eff_left[height - 1]);
                    let horizontal =
                        wx * u32::from(eff_left[y]) + (256 - wx) * u32::from(eff_above[width - 1]);
                    match mode {
                        IntraMode::SmoothVPred => ((vertical + 128) >> 8) as u16,
                        IntraMode::SmoothHPred => ((horizontal + 128) >> 8) as u16,
                        _ => ((vertical + horizontal + 256) >> 9) as u16,
                    }
                }
                IntraMode::PaethPred => {
                    predict_paeth_sample(eff_left[y], eff_above[x], eff_above_left)
                }
                _ => return Err(CodecError::Unsupported),
            };
        }
    }
    Ok(())
}

fn smooth_weights(size: usize) -> codec_core::CodecResult<&'static [u16]> {
    use super::smooth_tables::*;
    match size {
        4 => Ok(&SMOOTH_4),
        8 => Ok(&SMOOTH_8),
        16 => Ok(&SMOOTH_16),
        32 => Ok(&SMOOTH_32),
        64 => Ok(&SMOOTH_64),
        _ => Err(codec_core::CodecError::Unsupported),
    }
}

pub const DR_INTRA_DERIVATIVE: [u16; 90] = [
    1, 14666, 7330, 4884, 3660, 2926, 2435, 2084, 1821, 1616, 1451, 1317, 1204, 1108, 1026, 955,
    892, 837, 787, 743, 703, 666, 633, 603, 574, 548, 524, 502, 481, 461, 443, 426, 409, 394, 379,
    365, 352, 339, 327, 316, 305, 294, 284, 274, 265, 256, 247, 238, 230, 222, 214, 207, 200, 192,
    185, 179, 172, 166, 159, 153, 147, 141, 136, 130, 124, 119, 113, 108, 103, 98, 93, 88, 83, 78,
    73, 68, 63, 59, 54, 49, 45, 40, 35, 31, 26, 22, 17, 13, 8, 4,
];

pub const INTRA_EDGE_KERNEL: [[u16; 5]; 3] = [[0, 4, 8, 4, 0], [0, 5, 6, 5, 0], [2, 4, 4, 4, 2]];

pub fn mode_to_nominal_angle(mode: IntraMode) -> Option<usize> {
    match mode {
        IntraMode::VPred => Some(90),
        IntraMode::HPred => Some(180),
        IntraMode::D45Pred => Some(45),
        IntraMode::D135Pred => Some(135),
        IntraMode::D113Pred => Some(113),
        IntraMode::D157Pred => Some(157),
        IntraMode::D203Pred => Some(203),
        IntraMode::D67Pred => Some(67),
        _ => None,
    }
}

pub fn get_intra_edge_filter_strength(blk_wh: usize, delta: i32, filter_type: u8) -> u8 {
    let d = delta.abs();
    if filter_type == 0 {
        if blk_wh <= 8 {
            if d >= 56 {
                1
            } else {
                0
            }
        } else if blk_wh <= 16 {
            if d >= 40 {
                1
            } else {
                0
            }
        } else if blk_wh <= 24 {
            if d >= 32 {
                3
            } else if d >= 16 {
                2
            } else if d >= 8 {
                1
            } else {
                0
            }
        } else if blk_wh <= 32 {
            if d >= 32 {
                3
            } else if d >= 4 {
                2
            } else {
                1
            }
        } else {
            3
        }
    } else {
        if blk_wh <= 8 {
            if d >= 64 {
                2
            } else if d >= 40 {
                1
            } else {
                0
            }
        } else if blk_wh <= 16 {
            if d >= 48 {
                2
            } else if d >= 20 {
                1
            } else {
                0
            }
        } else if blk_wh <= 24 {
            if d >= 4 {
                3
            } else {
                0
            }
        } else {
            3
        }
    }
}

pub fn get_intra_edge_upsample(blk_wh: usize, delta: i32, filter_type: u8) -> bool {
    let d = delta.abs();
    if d <= 0 || d >= 40 {
        false
    } else if filter_type == 0 {
        blk_wh <= 16
    } else {
        blk_wh <= 8
    }
}

#[allow(clippy::too_many_arguments)]
pub fn predict_directional(
    p_angle: usize,
    dst: &mut [u16],
    stride: usize,
    width: usize,
    height: usize,
    bit_depth: u8,
    above_raw: &[u16], // above_raw[0] is -1, above_raw[1..] is 0..
    left_raw: &[u16],  // left_raw[0] is -1, left_raw[1..] is 0..
    enable_intra_edge_filter: bool,
    filter_type: u8,
) -> codec_core::CodecResult<()> {
    use codec_core::CodecError;
    if !matches!(bit_depth, 8 | 10 | 12)
        || width == 0
        || height == 0
        || width > 128
        || height > 128
        || stride < width
        || stride.checked_mul(height).is_none_or(|n| n > dst.len())
    {
        return Err(CodecError::InvalidData);
    }

    if p_angle == 90 {
        for y in 0..height {
            for x in 0..width {
                dst[y * stride + x] = above_raw[1 + x];
            }
        }
        return Ok(());
    }
    if p_angle == 180 {
        for y in 0..height {
            for x in 0..width {
                dst[y * stride + x] = left_raw[1 + y];
            }
        }
        return Ok(());
    }

    let max_sample = (1u16 << bit_depth) - 1;
    let mut above = [0u16; 64];
    let mut left = [0u16; 64];
    above[..above_raw.len().min(64)].copy_from_slice(&above_raw[..above_raw.len().min(64)]);
    left[..left_raw.len().min(64)].copy_from_slice(&left_raw[..left_raw.len().min(64)]);

    // 1. Filter corner process (7.11.2.7)
    if enable_intra_edge_filter && p_angle > 90 && p_angle < 180 && width + height >= 24 {
        let s = left[1] as u32 * 5 + above[0] as u32 * 6 + above[1] as u32 * 5;
        let corner = ((s + 8) >> 4) as u16;
        above[0] = corner;
        left[0] = corner;
    }

    let blk_wh = width + height;

    // 2. Intra edge filter & upsample for Above edge
    let mut upsample_above = false;
    let num_px_above = width + if p_angle < 90 { height } else { 0 };
    if enable_intra_edge_filter {
        let delta = p_angle as i32 - 90;
        let strength = get_intra_edge_filter_strength(blk_wh, delta, filter_type);
        if strength > 0 {
            let mut filtered = [0u16; 64];
            let sz = num_px_above + 1;
            let kernel = &INTRA_EDGE_KERNEL[strength as usize - 1];
            for (i, value) in filtered.iter_mut().enumerate().take(sz) {
                let mut s = 0u32;
                for (j, &k_wt) in kernel.iter().enumerate() {
                    let k = (i as i32 - 2 + j as i32).clamp(0, sz as i32 - 1) as usize;
                    s += k_wt as u32 * above[k] as u32;
                }
                *value = ((s + 8) >> 4) as u16;
            }
            above[..sz].copy_from_slice(&filtered[..sz]);
        }
        upsample_above = get_intra_edge_upsample(blk_wh, delta, filter_type);
    }

    let mut up_above = [0u16; 128];
    if upsample_above {
        let mut dup = [0u16; 64];
        dup[0] = above[0];
        dup[1..num_px_above + 2].copy_from_slice(&above[..=num_px_above]);
        dup[num_px_above + 2] = above[num_px_above];

        up_above[0] = dup[0];
        for i in 0..num_px_above {
            let s = -(dup[i] as i32) + 9 * dup[i + 1] as i32 + 9 * dup[i + 2] as i32
                - dup[i + 3] as i32;
            let s_clamped = ((s + 8) >> 4).clamp(0, max_sample as i32) as u16;
            up_above[2 * i + 1] = s_clamped;
            up_above[2 * i + 2] = dup[i + 2];
        }
    }

    // 3. Intra edge filter & upsample for Left edge
    let mut upsample_left = false;
    let num_px_left = height + if p_angle > 180 { width } else { 0 };
    if enable_intra_edge_filter {
        let delta = p_angle as i32 - 180;
        let strength = get_intra_edge_filter_strength(blk_wh, delta, filter_type);
        if strength > 0 {
            let mut filtered = [0u16; 64];
            let sz = num_px_left + 1;
            let kernel = &INTRA_EDGE_KERNEL[strength as usize - 1];
            for (i, value) in filtered.iter_mut().enumerate().take(sz) {
                let mut s = 0u32;
                for (j, &k_wt) in kernel.iter().enumerate() {
                    let k = (i as i32 - 2 + j as i32).clamp(0, sz as i32 - 1) as usize;
                    s += k_wt as u32 * left[k] as u32;
                }
                *value = ((s + 8) >> 4) as u16;
            }
            left[..sz].copy_from_slice(&filtered[..sz]);
        }
        upsample_left = get_intra_edge_upsample(blk_wh, delta, filter_type);
    }

    let mut up_left = [0u16; 128];
    if upsample_left {
        let mut dup = [0u16; 64];
        dup[0] = left[0];
        dup[1..num_px_left + 2].copy_from_slice(&left[..=num_px_left]);
        dup[num_px_left + 2] = left[num_px_left];

        up_left[0] = dup[0];
        for i in 0..num_px_left {
            let s = -(dup[i] as i32) + 9 * dup[i + 1] as i32 + 9 * dup[i + 2] as i32
                - dup[i + 3] as i32;
            let s_clamped = ((s + 8) >> 4).clamp(0, max_sample as i32) as u16;
            up_left[2 * i + 1] = s_clamped;
            up_left[2 * i + 2] = dup[i + 2];
        }
    }

    let get_above = |k: i32| -> u16 {
        if upsample_above {
            let max_idx = 2 * num_px_above;
            let idx = (k + 2).clamp(0, max_idx as i32) as usize;
            up_above[idx]
        } else {
            let max_idx = num_px_above;
            let idx = (k + 1).clamp(0, max_idx as i32) as usize;
            above[idx]
        }
    };

    let get_left = |k: i32| -> u16 {
        if upsample_left {
            let max_idx = 2 * num_px_left;
            let idx = (k + 2).clamp(0, max_idx as i32) as usize;
            up_left[idx]
        } else {
            let max_idx = num_px_left;
            let idx = (k + 1).clamp(0, max_idx as i32) as usize;
            left[idx]
        }
    };

    let ups_a = upsample_above as u32;
    let ups_l = upsample_left as u32;

    if p_angle < 90 {
        let dx = DR_INTRA_DERIVATIVE[p_angle] as i32;
        let max_base_x = ((width + height - 1) << ups_a) as i32;
        for i in 0..height {
            for j in 0..width {
                let idx = (i as i32 + 1) * dx;
                let base = (idx >> (6 - ups_a)) + ((j as i32) << ups_a);
                if base >= max_base_x {
                    dst[i * stride + j] = get_above(max_base_x);
                } else {
                    let shift = ((idx << ups_a) >> 1) & 0x1F;
                    let v0 = get_above(base) as i32;
                    let v1 = get_above(base + 1) as i32;
                    dst[i * stride + j] = ((v0 * (32 - shift) + v1 * shift + 16) >> 5) as u16;
                }
            }
        }
    } else if p_angle > 180 {
        let dy = DR_INTRA_DERIVATIVE[270 - p_angle] as i32;
        let max_base_y = ((width + height - 1) << ups_l) as i32;
        for i in 0..height {
            for j in 0..width {
                let idx = (j as i32 + 1) * dy;
                let base = (idx >> (6 - ups_l)) + ((i as i32) << ups_l);
                if base >= max_base_y {
                    dst[i * stride + j] = get_left(max_base_y);
                } else {
                    let shift = ((idx << ups_l) >> 1) & 0x1F;
                    let v0 = get_left(base) as i32;
                    let v1 = get_left(base + 1) as i32;
                    dst[i * stride + j] = ((v0 * (32 - shift) + v1 * shift + 16) >> 5) as u16;
                }
            }
        }
    } else {
        // 90 < p_angle < 180
        let dx = DR_INTRA_DERIVATIVE[180 - p_angle] as i32;
        let dy = DR_INTRA_DERIVATIVE[p_angle - 90] as i32;
        for i in 0..height {
            for j in 0..width {
                let idx = ((j as i32) << 6) - (i as i32 + 1) * dx;
                let base = idx >> (6 - ups_a);
                if base >= -(1 << ups_a) {
                    let shift = ((idx << ups_a) >> 1) & 0x1F;
                    let v0 = get_above(base) as i32;
                    let v1 = get_above(base + 1) as i32;
                    dst[i * stride + j] = ((v0 * (32 - shift) + v1 * shift + 16) >> 5) as u16;
                } else {
                    let idx_y = ((i as i32) << 6) - (j as i32 + 1) * dy;
                    let base_y = idx_y >> (6 - ups_l);
                    let shift_y = ((idx_y << ups_l) >> 1) & 0x1F;
                    let v0 = get_left(base_y) as i32;
                    let v1 = get_left(base_y + 1) as i32;
                    dst[i * stride + j] = ((v0 * (32 - shift_y) + v1 * shift_y + 16) >> 5) as u16;
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod predictor_tests {
    use super::*;

    #[test]
    fn dc_with_edges_and_without_edges() {
        let mut out = [999u16; 6];
        predict_basic_block(
            IntraMode::DcPred,
            &mut out,
            3,
            2,
            2,
            8,
            Some(&[10, 20]),
            Some(&[30, 40]),
            None,
        )
        .unwrap();
        assert_eq!(out, [25, 25, 999, 25, 25, 999]);
        predict_basic_block(IntraMode::DcPred, &mut out, 3, 2, 2, 10, None, None, None).unwrap();
        assert_eq!(out, [512, 512, 999, 512, 512, 999]);
    }

    #[test]
    fn edge_predictors_and_rejection() {
        let mut out = [7u16; 4];
        predict_basic_block(
            IntraMode::VPred,
            &mut out,
            2,
            2,
            2,
            8,
            Some(&[11, 13]),
            None,
            None,
        )
        .unwrap();
        assert_eq!(out, [11, 13, 11, 13]);
        predict_basic_block(
            IntraMode::HPred,
            &mut out,
            2,
            2,
            2,
            8,
            None,
            Some(&[31, 47]),
            None,
        )
        .unwrap();
        assert_eq!(out, [31, 31, 47, 47]);
        predict_basic_block(
            IntraMode::PaethPred,
            &mut out,
            2,
            2,
            2,
            8,
            Some(&[11, 13]),
            Some(&[31, 47]),
            Some(3),
        )
        .unwrap();
        assert_eq!(out[0], predict_paeth_sample(31, 11, 3));
        let saved = out;
        assert!(predict_basic_block(
            IntraMode::SmoothPred,
            &mut out,
            2,
            2,
            2,
            8,
            None,
            None,
            None
        )
        .is_err());
        assert!(predict_basic_block(
            IntraMode::DcPred,
            &mut out,
            2,
            2,
            2,
            8,
            Some(&[256, 3]),
            None,
            None
        )
        .is_err());
        assert_eq!(out, saved);
    }
}
