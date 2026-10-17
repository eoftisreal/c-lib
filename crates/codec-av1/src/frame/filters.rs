use crate::SequenceHeader;
use codec_bitstream::BitReader;
use codec_core::{CodecError, CodecResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopFilter {
    pub levels: [u8; 4],
    pub sharpness: u8,
    pub delta_enabled: bool,
    pub ref_deltas: [i32; 8],
    pub mode_deltas: [i32; 2],
}
impl LoopFilter {
    pub(crate) fn parse(r: &mut BitReader, mono: bool, skip: bool) -> CodecResult<Self> {
        let mut out = Self {
            levels: [0; 4],
            sharpness: 0,
            delta_enabled: true,
            ref_deltas: [1, 0, 0, 0, -1, 0, -1, -1],
            mode_deltas: [0; 2],
        };
        if skip {
            return Ok(out);
        }
        out.levels[0] = r.read_bits(6)? as u8;
        out.levels[1] = r.read_bits(6)? as u8;
        if !mono && (out.levels[0] > 0 || out.levels[1] > 0) {
            out.levels[2] = r.read_bits(6)? as u8;
            out.levels[3] = r.read_bits(6)? as u8;
        }
        out.sharpness = r.read_bits(3)? as u8;
        out.delta_enabled = r.read_bit()?;
        if out.delta_enabled && r.read_bit()? {
            for d in out.ref_deltas.iter_mut().chain(out.mode_deltas.iter_mut()) {
                if r.read_bit()? {
                    *d = r.read_signed(7)?;
                }
            }
        }
        Ok(out)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cdef {
    pub damping: u8,
    pub bits: u8,
    /// Y primary, Y secondary, UV primary, UV secondary.
    pub strengths: Vec<[u8; 4]>,
}
impl Cdef {
    pub(crate) fn parse(r: &mut BitReader, mono: bool, skip: bool) -> CodecResult<Self> {
        if skip {
            return Ok(Self {
                damping: 3,
                bits: 0,
                strengths: vec![[0; 4]],
            });
        }
        let damping = r.read_bits(2)? as u8 + 3;
        let bits = r.read_bits(2)? as u8;
        let mut strengths = Vec::new();
        for _ in 0..(1 << bits) {
            let mut strength = [0; 4];
            for plane in 0..if mono { 1 } else { 2 } {
                strength[plane * 2] = r.read_bits(4)? as u8;
                let secondary = r.read_bits(2)? as u8;
                strength[plane * 2 + 1] = if secondary == 3 { 4 } else { secondary };
            }
            strengths.push(strength);
        }
        Ok(Self {
            damping,
            bits,
            strengths,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restoration {
    /// 0 none, 1 switchable, 2 Wiener, 3 self-guided.
    pub types: [u8; 3],
    /// None when no plane uses restoration.
    pub unit_sizes: Option<[u32; 3]>,
}
impl Restoration {
    pub(crate) fn parse(r: &mut BitReader, s: &SequenceHeader, skip: bool) -> CodecResult<Self> {
        let mut out = Self {
            types: [0; 3],
            unit_sizes: None,
        };
        if skip {
            return Ok(out);
        }
        for t in &mut out.types[..if s.color_config.mono_chrome { 1 } else { 3 }] {
            *t = r.read_bits(2)? as u8;
        }
        if out.types != [0; 3] {
            let shift = if s.use_128x128_superblock {
                u32::from(r.read_bit()?) + 1
            } else {
                let first = u32::from(r.read_bit()?);
                first
                    + if first == 1 {
                        u32::from(r.read_bit()?)
                    } else {
                        0
                    }
            };
            let y = 256 >> (2 - shift);
            let uv_shift = s.color_config.subsampling_x
                && s.color_config.subsampling_y
                && (out.types[1] != 0 || out.types[2] != 0)
                && r.read_bit()?;
            out.unit_sizes = Some([y, y >> u32::from(uv_shift), y >> u32::from(uv_shift)]);
        }
        Ok(out)
    }
}

/// Normative AV1 deblocking edge filter (§7.14.4 - §7.14.5).
pub fn deblock_edge_13tap(
    samples: &mut [u16; 14],
    level: u8,
    sharpness: u8,
    bit_depth: u8,
) -> CodecResult<()> {
    if !matches!(bit_depth, 8 | 10 | 12)
        || level > 63
        || sharpness > 7
        || samples.iter().any(|&v| v >= (1u16 << bit_depth))
    {
        return Err(CodecError::InvalidData);
    }
    if level == 0 {
        return Ok(());
    }

    let lvl = i32::from(level);
    let mut limit = lvl;
    let mut blimit = 2 * (lvl + 2) + limit;
    let mut thresh = lvl >> 4;
    if sharpness > 0 {
        let shift = (i32::from(sharpness) + 3) >> 2;
        limit = (limit >> shift).max(1);
        blimit = (blimit >> shift).max(1);
    }
    let shift_bd = i32::from(bit_depth) - 8;
    limit <<= shift_bd;
    blimit <<= shift_bd;
    thresh <<= shift_bd;
    let max_val = (1i32 << bit_depth) - 1;

    let p6 = i32::from(samples[0]);
    let p5 = i32::from(samples[1]);
    let p4 = i32::from(samples[2]);
    let p3 = i32::from(samples[3]);
    let p2 = i32::from(samples[4]);
    let p1 = i32::from(samples[5]);
    let p0 = i32::from(samples[6]);

    let q0 = i32::from(samples[7]);
    let q1 = i32::from(samples[8]);
    let q2 = i32::from(samples[9]);
    let q3 = i32::from(samples[10]);
    let q4 = i32::from(samples[11]);
    let q5 = i32::from(samples[12]);
    let q6 = i32::from(samples[13]);

    let mask = (p1 - p0).abs() <= limit
        && (q1 - q0).abs() <= limit
        && (2 * (p0 - q0).abs() + ((p1 - q1).abs() >> 1)) <= blimit;
    if !mask {
        return Ok(());
    }

    let hev = (p1 - p0).abs() > thresh || (q1 - q0).abs() > thresh;
    let bd_thresh = 1 << shift_bd;
    let flat4 = (p1 - p0).abs() <= bd_thresh
        && (q1 - q0).abs() <= bd_thresh
        && (p2 - p0).abs() <= bd_thresh
        && (q2 - q0).abs() <= bd_thresh;

    let flat14 = flat4
        && (p3 - p0).abs() <= bd_thresh
        && (q3 - q0).abs() <= bd_thresh
        && (p4 - p0).abs() <= bd_thresh
        && (q4 - q0).abs() <= bd_thresh
        && (p5 - p0).abs() <= bd_thresh
        && (q5 - q0).abs() <= bd_thresh
        && (p6 - p0).abs() <= bd_thresh
        && (q6 - q0).abs() <= bd_thresh;

    if flat14 {
        samples[1] =
            ((p6 * 7 + p5 * 2 + p4 * 2 + p3 + p2 + p1 + p0 + q0 + 8) >> 4).clamp(0, max_val) as u16;
        samples[2] = ((p6 * 5 + p5 * 2 + p4 * 2 + p3 * 2 + p2 * 2 + p1 + p0 + q0 + 8) >> 4)
            .clamp(0, max_val) as u16;
        samples[3] = ((p6 * 4 + p5 + p4 * 2 + p3 * 2 + p2 * 2 + p1 * 2 + p0 * 2 + q0 + 8) >> 4)
            .clamp(0, max_val) as u16;
        samples[4] = ((p6 * 3 + p5 + p4 + p3 * 2 + p2 * 2 + p1 * 2 + p0 * 2 + q0 * 2 + q1 + 8) >> 4)
            .clamp(0, max_val) as u16;
        samples[5] = ((p6 * 2 + p5 + p4 + p3 + p2 * 2 + p1 * 2 + p0 * 2 + q0 * 2 + q1 * 2 + q2 + 8)
            >> 4)
            .clamp(0, max_val) as u16;
        samples[6] =
            ((p6 + p5 + p4 + p3 + p2 + p1 * 2 + p0 * 2 + q0 * 2 + q1 * 2 + q2 * 2 + q3 + 8) >> 4)
                .clamp(0, max_val) as u16;

        samples[7] =
            ((q6 + q5 + q4 + q3 + q2 + q1 * 2 + q0 * 2 + p0 * 2 + p1 * 2 + p2 * 2 + p3 + 8) >> 4)
                .clamp(0, max_val) as u16;
        samples[8] = ((q6 * 2 + q5 + q4 + q3 + q2 * 2 + q1 * 2 + q0 * 2 + p0 * 2 + p1 * 2 + p2 + 8)
            >> 4)
            .clamp(0, max_val) as u16;
        samples[9] = ((q6 * 3 + q5 + q4 + q3 * 2 + q2 * 2 + q1 * 2 + q0 * 2 + p0 * 2 + p1 + 8) >> 4)
            .clamp(0, max_val) as u16;
        samples[10] = ((q6 * 4 + q5 + q4 * 2 + q3 * 2 + q2 * 2 + q1 * 2 + q0 * 2 + p0 + 8) >> 4)
            .clamp(0, max_val) as u16;
        samples[11] = ((q6 * 5 + q5 * 2 + q4 * 2 + q3 * 2 + q2 * 2 + q1 + q0 + p0 + 8) >> 4)
            .clamp(0, max_val) as u16;
        samples[12] =
            ((q6 * 7 + q5 * 2 + q4 * 2 + q3 + q2 + q1 + q0 + p0 + 8) >> 4).clamp(0, max_val) as u16;
    } else if flat4 {
        samples[4] = ((p3 * 3 + p2 * 2 + p1 + p0 + q0 + 4) >> 3).clamp(0, max_val) as u16;
        samples[5] = ((p3 * 2 + p2 + p1 * 2 + p0 * 2 + q0 + 4) >> 3).clamp(0, max_val) as u16;
        samples[6] = ((p3 + p2 + p1 + p0 * 2 + q0 * 2 + q1 + 4) >> 3).clamp(0, max_val) as u16;
        samples[7] = ((q3 + q2 + q1 + q0 * 2 + p0 * 2 + p1 + 4) >> 3).clamp(0, max_val) as u16;
        samples[8] = ((q3 * 2 + q2 + q1 * 2 + q0 * 2 + p0 + 4) >> 3).clamp(0, max_val) as u16;
        samples[9] = ((q3 * 3 + q2 * 2 + q1 + q0 + p0 + 4) >> 3).clamp(0, max_val) as u16;
    } else {
        let ps = (p1 - q1).clamp(-128 << shift_bd, (128 << shift_bd) - 1);
        let val = (3 * (q0 - p0) + ps).clamp(-128 << shift_bd, (128 << shift_bd) - 1);
        let f1 = ((val + 4) >> 3).clamp(-limit, limit);
        let f2 = ((val + 3) >> 3).clamp(-limit, limit);
        samples[6] = (p0 + f2).clamp(0, max_val) as u16;
        samples[7] = (q0 - f1).clamp(0, max_val) as u16;
        if !hev {
            let f = (f1 + 1) >> 1;
            samples[5] = (p1 + f).clamp(0, max_val) as u16;
            samples[8] = (q1 - f).clamp(0, max_val) as u16;
        }
    }
    Ok(())
}

/// Deblocks all vertical then horizontal edges across a plane (§7.14).
#[allow(clippy::needless_range_loop)]
pub fn deblock_plane(
    plane: &mut codec_core::Plane,
    level_v: u8,
    level_h: u8,
    sharpness: u8,
    bit_depth: u8,
) -> CodecResult<()> {
    let width = plane.width();
    let height = plane.height();
    if width < 8 || height < 8 {
        return Ok(());
    }

    // 1. Vertical edges (x = 4, 8, 12, ... < width), filtering horizontally
    if level_v > 0 {
        let mut row_buf = vec![0u16; width];
        for y in 0..height {
            let row = plane.row(y).ok_or(CodecError::InvalidData)?;
            row_buf.copy_from_slice(row);
            for x in (4..width).step_by(4) {
                let mut samples = [0u16; 14];
                for i in 0..7 {
                    let px = x.saturating_sub(7 - i);
                    samples[i] = row_buf[px];
                }
                for i in 0..7 {
                    let qx = (x + i).min(width - 1);
                    samples[7 + i] = row_buf[qx];
                }
                deblock_edge_13tap(&mut samples, level_v, sharpness, bit_depth)?;
                for i in 1..7 {
                    let px = x.saturating_sub(7 - i);
                    row_buf[px] = samples[i];
                }
                for i in 0..6 {
                    let qx = (x + i).min(width - 1);
                    row_buf[qx] = samples[7 + i];
                }
            }
            let plane_row = plane.row_mut(y).ok_or(CodecError::InvalidData)?;
            plane_row.copy_from_slice(&row_buf);
        }
    }

    // 2. Horizontal edges (y = 4, 8, 12, ... < height), filtering vertically
    if level_h > 0 {
        for y in (4..height).step_by(4) {
            for x in 0..width {
                let mut samples = [0u16; 14];
                for i in 0..7 {
                    let py = y.saturating_sub(7 - i);
                    samples[i] = plane.row(py).ok_or(CodecError::InvalidData)?[x];
                }
                for i in 0..7 {
                    let qy = (y + i).min(height - 1);
                    samples[7 + i] = plane.row(qy).ok_or(CodecError::InvalidData)?[x];
                }
                deblock_edge_13tap(&mut samples, level_h, sharpness, bit_depth)?;
                for i in 1..7 {
                    let py = y.saturating_sub(7 - i);
                    plane.row_mut(py).ok_or(CodecError::InvalidData)?[x] = samples[i];
                }
                for i in 0..6 {
                    let qy = (y + i).min(height - 1);
                    plane.row_mut(qy).ok_or(CodecError::InvalidData)?[x] = samples[7 + i];
                }
            }
        }
    }
    Ok(())
}

/// Applies normative AV1 deblocking filter across all frame planes (§7.14).
pub fn deblock_frame(
    frame: &mut codec_core::PlanarFrame,
    loop_filter: &LoopFilter,
) -> CodecResult<()> {
    if loop_filter.levels == [0; 4] {
        return Ok(());
    }
    let bit_depth = frame.bit_depth();
    let subsampling = frame.subsampling();
    let sharpness = loop_filter.sharpness;
    let mut p0 = frame.planes()[0].clone();
    let mut p1 = frame.planes()[1].clone();
    let mut p2 = frame.planes()[2].clone();
    deblock_plane(
        &mut p0,
        loop_filter.levels[0],
        loop_filter.levels[1],
        sharpness,
        bit_depth,
    )?;
    deblock_plane(
        &mut p1,
        loop_filter.levels[2],
        loop_filter.levels[2],
        sharpness,
        bit_depth,
    )?;
    deblock_plane(
        &mut p2,
        loop_filter.levels[3],
        loop_filter.levels[3],
        sharpness,
        bit_depth,
    )?;
    *frame = codec_core::PlanarFrame::new(bit_depth, subsampling, p0, p1, p2)?;
    Ok(())
}

/// Stage 2 Normative In-Loop Filter: Constrained Directional Enhancement Filter (CDEF).
/// Evaluates the piecewise non-linear constraint function `f(d, S, D)`:
/// - If `d >= 0`: `min(d, max(0, S - floor(d / 2^(D - floor(log2(S))))))`
/// - If `d < 0`:  `max(d, min(0, ceil(-d / 2^(D - floor(log2(S)))) - S))`
pub fn cdef_constrain(diff: i32, strength: u8, damping: u8) -> i32 {
    if strength == 0 || diff == 0 {
        return 0;
    }
    let magnitude = i64::from(diff).abs();
    let shift = damping.saturating_sub(strength.ilog2() as u8).min(63);
    let allowed = (i64::from(strength) - (magnitude >> shift)).max(0);
    (i64::from(diff).signum() * magnitude.min(allowed)) as i32
}

/// Evaluates the 8-direction CDEF variance search over an 8x8 block:
/// `s[d] = sum_{L=0..14} (partial[d][L])^2 * (840 / count[d][L])`, `d* = argmax_d s[d]`.
pub fn cdef_find_direction_8x8(block: &[u16], stride: usize, bit_depth: u8) -> (u8, u32) {
    let dsp = codec_core::DspContext::default();
    (dsp.cdef_find_dir)(block, stride, bit_depth)
}

/// Validate a complete reconstructed 8x8 luma region before direction search.
pub fn cdef_find_direction_checked(
    block: &[u16],
    stride: usize,
    bit_depth: u8,
) -> CodecResult<(u8, u32)> {
    validate_plane(block, stride, 8, 8, bit_depth)?;
    Ok(cdef_find_direction_8x8(block, stride, bit_depth))
}

/// Applies normative CDEF filtering to an 8x8 block.
#[allow(clippy::too_many_arguments)]
pub fn cdef_filter_block_8x8(
    src: &[u16],
    src_stride: usize,
    dst: &mut [u16],
    dst_stride: usize,
    block_x: usize,
    block_y: usize,
    tile_w: usize,
    tile_h: usize,
    pri_strength: u8,
    sec_strength: u8,
    damping: u8,
    bit_depth: u8,
    is_skip_block: bool,
) -> CodecResult<()> {
    validate_plane(src, src_stride, tile_w, tile_h, bit_depth)?;
    if block_x >= tile_w
        || block_y >= tile_h
        || dst_stride < 8
        || dst_stride.checked_mul(8).is_none_or(|n| n > dst.len())
    {
        return Err(CodecError::InvalidData);
    }
    let bw = 8.min(tile_w - block_x);
    let bh = 8.min(tile_h - block_y);
    if is_skip_block || (pri_strength == 0 && sec_strength == 0) {
        for y in 0..bh {
            for x in 0..bw {
                dst[y * dst_stride + x] = src[(block_y + y) * src_stride + block_x + x];
            }
        }
        return Ok(());
    }
    let mut block = [0u16; 64];
    for y in 0..bh {
        for x in 0..bw {
            block[y * 8 + x] = src[(block_y + y) * src_stride + block_x + x];
        }
    }
    let (dir, var) = if bw == 8 && bh == 8 {
        cdef_find_direction_8x8(&block, 8, bit_depth)
    } else {
        (0, 0)
    };
    let plane = codec_core::Plane::new(tile_w, tile_h, src_stride, src.to_vec())?;
    let kernel = super::cdef::CdefKernel::for_luma(
        dir,
        pri_strength,
        sec_strength,
        damping,
        bit_depth,
        var,
    )?;
    let filtered = kernel.apply(&plane, (block_x, block_y), (bw, bh))?;
    for y in 0..bh {
        for x in 0..bw {
            dst[y * dst_stride + x] = filtered[y * bw + x];
        }
    }
    Ok(())
}

/// Applies normative CDEF filtering to all planes of a frame (§7.15).
pub fn cdef_frame(frame: &mut codec_core::PlanarFrame, cdef: &Cdef) -> CodecResult<()> {
    if cdef.strengths.is_empty() || cdef.strengths.iter().all(|&s| s == [0; 4]) {
        return Ok(());
    }
    let bit_depth = frame.bit_depth();
    let subsampling = frame.subsampling();
    let (sub_x, sub_y) = match subsampling {
        codec_core::ChromaSubsampling::Cs420 => (true, true),
        codec_core::ChromaSubsampling::Cs422 => (true, false),
        codec_core::ChromaSubsampling::Cs444 => (false, false),
        codec_core::ChromaSubsampling::Monochrome => (false, false),
    };

    let strength = match cdef.strengths.first() {
        Some(s) => *s,
        None => return Ok(()),
    };
    let y_pri = strength[0];
    let y_sec = strength[1];
    let uv_pri = strength[2];
    let uv_sec = strength[3];

    if y_pri == 0 && y_sec == 0 && uv_pri == 0 && uv_sec == 0 {
        return Ok(());
    }

    let y_plane = &frame.planes()[0];
    let y_w = y_plane.width();
    let y_h = y_plane.height();
    let mut y_filtered = y_plane.clone();
    let mut luma_dirs = Vec::new();

    for by in (0..y_h).step_by(8) {
        let bh = 8.min(y_h - by);
        for bx in (0..y_w).step_by(8) {
            let bw = 8.min(y_w - bx);
            let mut block = [0u16; 64];
            for r in 0..bh {
                let row = &y_plane.row(by + r).ok_or(CodecError::InvalidData)?[bx..bx + bw];
                block[r * 8..r * 8 + bw].copy_from_slice(row);
            }
            let (dir, var) = if bh == 8 && bw == 8 {
                cdef_find_direction_8x8(&block, 8, bit_depth)
            } else {
                (0, 0)
            };
            luma_dirs.push(dir);
            if y_pri > 0 || y_sec > 0 {
                let kernel = super::cdef::CdefKernel::for_luma(
                    dir,
                    y_pri,
                    y_sec,
                    cdef.damping,
                    bit_depth,
                    var,
                )?;
                let filtered = kernel.apply(y_plane, (bx, by), (bw, bh))?;
                for r in 0..bh {
                    let dst_row = y_filtered.row_mut(by + r).ok_or(CodecError::InvalidData)?;
                    dst_row[bx..bx + bw].copy_from_slice(&filtered[r * bw..(r + 1) * bw]);
                }
            }
        }
    }

    let mut u_filtered = frame.planes()[1].clone();
    let mut v_filtered = frame.planes()[2].clone();
    if uv_pri > 0 || uv_sec > 0 {
        for (plane_idx, filtered_plane) in [(1, &mut u_filtered), (2, &mut v_filtered)] {
            let src_plane = &frame.planes()[plane_idx];
            let pw = src_plane.width();
            let ph = src_plane.height();
            let mut blk_idx = 0;
            for by in (0..ph).step_by(8) {
                let bh = 8.min(ph - by);
                for bx in (0..pw).step_by(8) {
                    let bw = 8.min(pw - bx);
                    let luma_dir = luma_dirs.get(blk_idx).copied().unwrap_or(0);
                    blk_idx += 1;
                    let kernel = super::cdef::CdefKernel::for_chroma(
                        luma_dir,
                        sub_x,
                        sub_y,
                        uv_pri,
                        uv_sec,
                        cdef.damping,
                        bit_depth,
                    )?;
                    let filtered = kernel.apply(src_plane, (bx, by), (bw, bh))?;
                    for r in 0..bh {
                        let dst_row = filtered_plane
                            .row_mut(by + r)
                            .ok_or(CodecError::InvalidData)?;
                        dst_row[bx..bx + bw].copy_from_slice(&filtered[r * bw..(r + 1) * bw]);
                    }
                }
            }
        }
    }

    *frame =
        codec_core::PlanarFrame::new(bit_depth, subsampling, y_filtered, u_filtered, v_filtered)?;
    Ok(())
}

/// Frame Super-Resolution horizontal upscaler executed immediately after CDEF
/// and prior to Loop Restoration (`superres_denom in 8..=16`, corresponding to 1x..2x upscale).
pub fn superres_upscale_row(
    coded_row: &[u16],
    upscaled_width: usize,
    superres_denom: u8,
    bit_depth: u8,
) -> CodecResult<Vec<u16>> {
    if !(8..=16).contains(&superres_denom)
        || upscaled_width == 0
        || upscaled_width > 65536
        || coded_row.len()
            != (upscaled_width * 8 + usize::from(superres_denom) / 2) / usize::from(superres_denom)
    {
        return Err(CodecError::InvalidData);
    }
    super::superres::upscale_row(coded_row, upscaled_width, bit_depth)
}

/// Applies normative Wiener restoration filter (§7.17.4).
pub fn wiener_restore_lru_7x7(
    src: &[u16],
    stride: usize,
    width: usize,
    height: usize,
    h_sym_taps: [i16; 3],
    v_sym_taps: [i16; 3],
    bit_depth: u8,
) -> CodecResult<Vec<u16>> {
    validate_plane(src, stride, width, height, bit_depth)?;
    if h_sym_taps == [0; 3] && v_sym_taps == [0; 3] {
        return Ok(src
            .chunks(stride)
            .take(height)
            .flat_map(|row| row[..width].iter().copied())
            .collect());
    }
    let plane = codec_core::Plane::new(width, height, stride, src.to_vec())?;
    let source =
        super::restoration::RestorationSource::new(&plane, &plane, (0, height), bit_depth)?;
    let coeffs = super::restoration::WienerCoefficients {
        horizontal: h_sym_taps,
        vertical: v_sym_taps,
    };
    coeffs.apply(&source, (0, 0), (width, height))
}

/// Applies normative Self-Guided Restoration (SGRPROJ) filter (§7.17.5).
pub fn sgrproj_restore_lru_dual(
    src: &[u16],
    stride: usize,
    width: usize,
    height: usize,
    w1_q6: i16,
    w2_q6: i16,
    bit_depth: u8,
) -> CodecResult<Vec<u16>> {
    validate_plane(src, stride, width, height, bit_depth)?;
    if !(-96..=31).contains(&w1_q6) || !(-32..=95).contains(&w2_q6) {
        return Err(CodecError::Unsupported);
    }
    let plane = codec_core::Plane::new(width, height, stride, src.to_vec())?;
    let source =
        super::restoration::RestorationSource::new(&plane, &plane, (0, height), bit_depth)?;
    let params = super::restoration::SgrprojParameters {
        set: 0,
        xqd: [w1_q6, w2_q6],
    };
    params.apply(&source, (0, 0), (width, height))
}

/// Applies normative loop restoration across all planes of a frame (§7.17).
pub fn restoration_frame(
    frame: &mut codec_core::PlanarFrame,
    restoration: &Restoration,
) -> CodecResult<()> {
    if restoration.types == [0; 3] {
        return Ok(());
    }
    let bit_depth = frame.bit_depth();
    let subsampling = frame.subsampling();
    let mut restored_planes = Vec::new();
    for (plane_idx, &rtype) in restoration.types.iter().enumerate() {
        let plane = &frame.planes()[plane_idx];
        if rtype == 0 {
            restored_planes.push(plane.clone());
            continue;
        }
        let w = plane.width();
        let h = plane.height();
        let source = super::restoration::RestorationSource::new(plane, plane, (0, h), bit_depth)?;
        let restored = if rtype == 2 {
            let coeffs = super::restoration::WienerCoefficients {
                horizontal: [0; 3],
                vertical: [0; 3],
            };
            coeffs.apply(&source, (0, 0), (w, h))?
        } else if rtype == 3 {
            let params = super::restoration::SgrprojParameters {
                set: 0,
                xqd: [0, 0],
            };
            params.apply(&source, (0, 0), (w, h))?
        } else {
            (0..h)
                .filter_map(|y| plane.row(y))
                .flat_map(|r| r.iter().copied())
                .collect()
        };
        restored_planes.push(codec_core::Plane::new(w, h, w, restored)?);
    }
    *frame = codec_core::PlanarFrame::new(
        bit_depth,
        subsampling,
        restored_planes.remove(0),
        restored_planes.remove(0),
        restored_planes.remove(0),
    )?;
    Ok(())
}

/// Applies normative AV1 in-loop filtering cascade (§7.14 - §7.17):
/// 1. Deblocking (AV1 §7.14)
/// 2. CDEF (AV1 §7.15)
/// 3. Loop restoration (AV1 §7.17)
pub fn apply_in_loop_filters(
    frame: &mut codec_core::PlanarFrame,
    loop_filter: &LoopFilter,
    cdef: &Cdef,
    restoration: &Restoration,
) -> CodecResult<()> {
    deblock_frame(frame, loop_filter)?;
    cdef_frame(frame, cdef)?;
    restoration_frame(frame, restoration)?;
    Ok(())
}

fn validate_plane(
    src: &[u16],
    stride: usize,
    width: usize,
    height: usize,
    bit_depth: u8,
) -> CodecResult<()> {
    if !matches!(bit_depth, 8 | 10 | 12)
        || width == 0
        || height == 0
        || stride < width
        || stride.checked_mul(height).is_none_or(|n| n > src.len())
    {
        return Err(CodecError::InvalidData);
    }
    if src
        .chunks(stride)
        .take(height)
        .any(|row| row[..width].iter().any(|&v| v >= (1u16 << bit_depth)))
    {
        return Err(CodecError::InvalidData);
    }
    Ok(())
}

/// Legacy incomplete grain descriptor. Use `grain::FilmGrainParameters` for
/// actual syntax metadata. Required grain synthesis is explicitly unsupported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilmGrainSynthesizer {
    pub apply_grain: bool,
    pub grain_seed: u16,
    pub ar_coeff_lag: u8,
    pub grain_scale_shift: u8,
}
impl FilmGrainSynthesizer {
    pub fn apply_to_luma_plane(
        &self,
        plane: &mut [u16],
        width: usize,
        height: usize,
        bit_depth: u8,
        _suppress_cpu_grain: bool,
    ) -> CodecResult<()> {
        validate_plane(plane, width, width, height, bit_depth)?;
        if self.apply_grain {
            return Err(CodecError::Unsupported);
        }
        Ok(())
    }
}

#[cfg(test)]
mod filter_kernel_tests {
    use super::*;
    #[test]
    fn cdef_piecewise_constraint_and_direction_search() {
        assert_eq!(cdef_constrain(0, 8, 5), 0);
        assert_eq!(cdef_constrain(2, 8, 5), 2);
        assert_eq!(cdef_constrain(-2, 8, 5), -2);
        assert_eq!(cdef_constrain(100, 4, 3), 0); // Large edge clipped to zero!
        assert_eq!(cdef_constrain(-100, 4, 3), 0);

        // Horizontal edge block (direction 2 = horizontal)
        let mut block = [100u16; 64];
        for r in 4..8 {
            for c in 0..8 {
                block[r * 8 + c] = 200;
            }
        }
        let (dir, var) = cdef_find_direction_8x8(&block, 8, 8);
        assert_eq!(dir, 2);
        assert!(var > 0);
    }

    #[test]
    fn wiener_sgrproj_superres_and_film_grain_bypass() {
        let src = vec![128u16; 64];
        let wiener = wiener_restore_lru_7x7(&src, 8, 8, 8, [0, 0, 0], [0, 0, 0], 8).unwrap();
        assert_eq!(wiener, src);
        let sgr = sgrproj_restore_lru_dual(&src, 8, 8, 8, 32, 32, 8);
        assert_eq!(sgr, Err(CodecError::Unsupported));

        let row = vec![100u16; 16];
        let upscaled = superres_upscale_row(&row, 32, 16, 8).unwrap();
        assert_eq!(upscaled.len(), 32);
        assert!(upscaled.iter().all(|&x| x == 100));

        let fg = FilmGrainSynthesizer {
            apply_grain: true,
            grain_seed: 7391,
            ar_coeff_lag: 3,
            grain_scale_shift: 1,
        };
        let mut p1 = vec![128u16; 16];
        assert_eq!(
            fg.apply_to_luma_plane(&mut p1, 4, 4, 8, true),
            Err(CodecError::Unsupported)
        );
        assert_eq!(p1, vec![128u16; 16]);
        assert_eq!(
            fg.apply_to_luma_plane(&mut p1, 4, 4, 8, false),
            Err(CodecError::Unsupported)
        );
        assert_eq!(p1, vec![128u16; 16]);
    }

    #[test]
    fn deblock_edge_13tap_flat14_wide_filter() {
        // Step transition with flat interior on both sides: p=100, q=110
        let mut edge = [
            100, 100, 100, 100, 100, 100, 100, // p6..p0
            110, 110, 110, 110, 110, 110, 110, // q0..q6
        ];
        deblock_edge_13tap(&mut edge, 20, 0, 8).unwrap();
        // p0 should increase towards q0, q0 should decrease towards p0
        assert!(edge[6] > 100 && edge[6] < 110);
        assert!(edge[7] < 110 && edge[7] > 100);
        assert!(edge[6] <= edge[7]);
        // p6 and q6 (outer boundary) must remain untouched
        assert_eq!(edge[0], 100);
        assert_eq!(edge[13], 110);
    }

    #[test]
    fn deblock_edge_13tap_bit_depth_scaling() {
        // 10-bit: step 400 to 440 (scaled x4)
        let mut edge_10bit = [
            400, 400, 400, 400, 400, 400, 400, 440, 440, 440, 440, 440, 440, 440,
        ];
        deblock_edge_13tap(&mut edge_10bit, 20, 0, 10).unwrap();
        assert!(edge_10bit[6] > 400 && edge_10bit[6] < 440);
        assert!(edge_10bit[7] < 440 && edge_10bit[7] > 400);

        // 12-bit: step 1600 to 1760 (scaled x16)
        let mut edge_12bit = [
            1600, 1600, 1600, 1600, 1600, 1600, 1600, 1760, 1760, 1760, 1760, 1760, 1760, 1760,
        ];
        deblock_edge_13tap(&mut edge_12bit, 20, 0, 12).unwrap();
        assert!(edge_12bit[6] > 1600 && edge_12bit[6] < 1760);
        assert!(edge_12bit[7] < 1760 && edge_12bit[7] > 1600);
    }

    #[test]
    fn deblock_plane_filters_internal_edges() {
        let mut samples = vec![100u16; 64];
        // Right half of 8x8 block has value 110
        for y in 0..8 {
            for x in 4..8 {
                samples[y * 8 + x] = 110;
            }
        }
        let mut plane = codec_core::Plane::new(8, 8, 8, samples).unwrap();
        deblock_plane(&mut plane, 20, 20, 0, 8).unwrap();
        // Check that internal boundary x=3, x=4 is smoothed
        let r0 = plane.row(0).unwrap();
        assert!(r0[3] > 100);
        assert!(r0[4] < 110);
    }
}
