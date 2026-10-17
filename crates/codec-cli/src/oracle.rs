//! Strict raw-planar YUV deserialization and decoded-pixel comparison.
//!
//! This accepts outputs **already decoded** by an external reference decoder
//! or by a future c-lib decoder; it never substitutes for AV1 decoding.

use codec_core::{ChromaSubsampling, PixelMismatch, PlanarFrame, Plane, PlaneId};
use std::io::{self, ErrorKind};

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, message)
}

const MAX_FRAME_BYTES: usize = 512 * 1024 * 1024;

/// Compute a single frame's exact raw planar YUV size without stride padding.
/// 10/12-bit samples use 16-bit little-endian words.
pub fn raw_frame_bytes(
    width: usize,
    height: usize,
    depth: u8,
    layout: ChromaSubsampling,
) -> io::Result<usize> {
    if width == 0 || height == 0 || !matches!(depth, 8 | 10 | 12) {
        return Err(invalid("invalid dimensions or bit depth"));
    }
    let (sx, sy) = match layout {
        ChromaSubsampling::Cs420 => (1, 1),
        ChromaSubsampling::Cs422 => (1, 0),
        ChromaSubsampling::Cs444 => (0, 0),
        ChromaSubsampling::Monochrome => return Err(invalid("monochrome is not supported")),
    };
    let y = width
        .checked_mul(height)
        .ok_or_else(|| invalid("frame size overflow"))?;
    let cw = width.div_ceil(1usize << sx);
    let ch = height.div_ceil(1usize << sy);
    let c = cw
        .checked_mul(ch)
        .ok_or_else(|| invalid("frame size overflow"))?;
    let n = y
        .checked_add(
            c.checked_mul(2)
                .ok_or_else(|| invalid("frame size overflow"))?,
        )
        .and_then(|samples| samples.checked_mul(if depth == 8 { 1 } else { 2 }))
        .ok_or_else(|| invalid("frame size overflow"))?;
    if n > MAX_FRAME_BYTES {
        return Err(invalid("frame exceeds verification memory limit"));
    }
    Ok(n)
}

/// Decode exactly one YUV frame from raw, tightly packed plane bytes.
///
/// For 10/12-bit pixels, two little-endian bytes represent one u16.
/// Illegal sample values, extra bytes, and truncated planes are rejected.
pub fn parse_raw_yuv(
    bytes: &[u8],
    width: usize,
    height: usize,
    depth: u8,
    layout: ChromaSubsampling,
) -> io::Result<PlanarFrame> {
    let expected = raw_frame_bytes(width, height, depth, layout)?;
    if bytes.len() != expected {
        return Err(invalid("raw frame length is not exact"));
    }
    let (sx, sy) = match layout {
        ChromaSubsampling::Cs420 => (1, 1),
        ChromaSubsampling::Cs422 => (1, 0),
        ChromaSubsampling::Cs444 => (0, 0),
        ChromaSubsampling::Monochrome => return Err(invalid("monochrome is not supported")),
    };
    let cw = width.div_ceil(1usize << sx);
    let ch = height.div_ceil(1usize << sy);
    let mut offset = 0;
    let mut plane = |pw: usize, ph: usize| -> io::Result<Plane> {
        let n = pw * ph; // already bounded by raw_frame_bytes
        let sz = n * if depth == 8 { 1 } else { 2 };
        let chunk = &bytes[offset..offset + sz];
        offset += sz;
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(n)
            .map_err(|_| io::Error::from(ErrorKind::OutOfMemory))?;
        if depth == 8 {
            samples.extend(chunk.iter().map(|&v| u16::from(v)));
        } else {
            samples.extend(
                chunk
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|v| u16::from_le_bytes([v[0], v[1]])),
            );
        }
        Plane::new(pw, ph, pw, samples).map_err(|_| invalid("invalid raw plane"))
    };
    let y = plane(width, height)?;
    let u = plane(cw, ch)?;
    let v = plane(cw, ch)?;
    PlanarFrame::new(depth, layout, y, u, v)
        .map_err(|_| invalid("sample value exceeds declared bit depth"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameMismatch {
    pub frame_index: usize,
    pub pixel: PixelMismatch,
}

/// Compare two frames. Pass source inputs from independent decoders, not
/// two copies of the same decoder's pixels.
pub fn compare_frame_bytes(
    expected: &[u8],
    actual: &[u8],
    width: usize,
    height: usize,
    depth: u8,
    layout: ChromaSubsampling,
    index: usize,
) -> io::Result<Option<FrameMismatch>> {
    let size = raw_frame_bytes(width, height, depth, layout)?;
    if expected.len() != size || actual.len() != size {
        return Err(invalid("raw frame length is not exact"));
    }
    let (sx, sy) = match layout {
        ChromaSubsampling::Cs420 => (1, 1),
        ChromaSubsampling::Cs422 => (1, 0),
        ChromaSubsampling::Cs444 => (0, 0),
        ChromaSubsampling::Monochrome => return Err(invalid("monochrome is not supported")),
    };
    let cw = width.div_ceil(1 << sx);
    let ch = height.div_ceil(1 << sy);
    let word = if depth == 8 { 1 } else { 2 };
    let sample = |bytes: &[u8], i: usize| -> u16 {
        if word == 1 {
            u16::from(bytes[i])
        } else {
            u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]])
        }
    };
    let mut offset = 0;
    let mut first = None;
    for (plane, w, h) in [
        (PlaneId::Y, width, height),
        (PlaneId::U, cw, ch),
        (PlaneId::V, cw, ch),
    ] {
        for i in 0..w * h {
            let a = sample(expected, offset + i);
            let b = sample(actual, offset + i);
            // Validate every sample even after a mismatch; malformed output is
            // not a pixel-conformance result. No decoded-plane copies needed.
            if a >= 1 << depth || b >= 1 << depth {
                return Err(invalid("sample value exceeds declared bit depth"));
            }
            if a != b && first.is_none() {
                first = Some(FrameMismatch {
                    frame_index: index,
                    pixel: PixelMismatch {
                        plane,
                        x: i % w,
                        y: i / w,
                        expected: a,
                        actual: b,
                    },
                });
            }
        }
        offset += w * h;
    }
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::y4m::write_raw_yuv;

    #[test]
    fn roundtrip_odd_sized_planar_frames_and_first_mismatch() {
        let p = |w, h, fill: u16| Plane::new(w, h, w, vec![fill; w * h]).unwrap();
        let picture = PlanarFrame::new(
            8,
            ChromaSubsampling::Cs420,
            p(3, 3, 2),
            p(2, 2, 60),
            p(2, 2, 90),
        )
        .unwrap();
        let mut reference = Vec::new();
        write_raw_yuv(&mut reference, &picture).unwrap();
        assert_eq!(reference.len(), 17);
        assert_eq!(
            parse_raw_yuv(&reference, 3, 3, 8, ChromaSubsampling::Cs420).unwrap(),
            picture
        );
        let mut observed = reference.clone();
        observed[9 + 3] = 150; // last U sample
        let mismatch =
            compare_frame_bytes(&reference, &observed, 3, 3, 8, ChromaSubsampling::Cs420, 12)
                .unwrap()
                .unwrap();
        assert_eq!(mismatch.frame_index, 12);
        assert_eq!(mismatch.pixel.plane, codec_core::PlaneId::U);
        assert_eq!((mismatch.pixel.x, mismatch.pixel.y), (1, 1));
        assert_eq!((mismatch.pixel.expected, mismatch.pixel.actual), (60, 150));
    }

    #[test]
    fn ten_and_twelve_bit_little_endian_are_checked() {
        for depth in [10u8, 12] {
            let high = (1u16 << depth) - 1;
            let p = |w, h| Plane::new(w, h, w, vec![high; w * h]).unwrap();
            let picture =
                PlanarFrame::new(depth, ChromaSubsampling::Cs444, p(4, 4), p(4, 4), p(4, 4))
                    .unwrap();
            let mut raw = Vec::new();
            write_raw_yuv(&mut raw, &picture).unwrap();
            assert_eq!(
                raw_frame_bytes(4, 4, depth, ChromaSubsampling::Cs444).unwrap(),
                96
            );
            assert_eq!(&raw[..2], &high.to_le_bytes());
            assert_eq!(
                parse_raw_yuv(&raw, 4, 4, depth, ChromaSubsampling::Cs444).unwrap(),
                picture
            );
            raw[0] = 255;
            raw[1] = 255;
            assert!(parse_raw_yuv(&raw, 4, 4, depth, ChromaSubsampling::Cs444).is_err());
        }
    }

    #[test]
    fn every_plane_and_depth_reports_native_coordinates() {
        for depth in [8, 10, 12] {
            for layout in [
                ChromaSubsampling::Cs420,
                ChromaSubsampling::Cs422,
                ChromaSubsampling::Cs444,
            ] {
                let cw = if layout == ChromaSubsampling::Cs444 {
                    3
                } else {
                    2
                };
                let ch = if layout == ChromaSubsampling::Cs420 {
                    2
                } else {
                    3
                };
                let bytes = if depth == 8 { 1 } else { 2 };
                let expected = vec![0; raw_frame_bytes(3, 3, depth, layout).unwrap()];
                for (plane, start, w, h) in [
                    (PlaneId::Y, 0, 3, 3),
                    (PlaneId::U, 9, cw, ch),
                    (PlaneId::V, 9 + cw * ch, cw, ch),
                ] {
                    let mut actual = expected.clone();
                    actual[(start + w * h - 1) * bytes] = 1;
                    let mismatch = compare_frame_bytes(&expected, &actual, 3, 3, depth, layout, 7)
                        .unwrap()
                        .unwrap();
                    assert_eq!(mismatch.frame_index, 7);
                    assert_eq!(mismatch.pixel.plane, plane);
                    assert_eq!((mismatch.pixel.x, mismatch.pixel.y), (w - 1, h - 1));
                }
            }
        }
    }

    #[test]
    fn invalid_late_sample_is_not_hidden_by_earlier_mismatch() {
        let expected = vec![0; 12]; // 2x2 10-bit 420
        let mut actual = expected.clone();
        actual[0] = 1;
        actual[11] = 255;
        assert!(
            compare_frame_bytes(&expected, &actual, 2, 2, 10, ChromaSubsampling::Cs420, 0).is_err()
        );
    }

    #[test]
    fn invalid_geometry_size_and_extra_bytes_are_rejected() {
        let layout = ChromaSubsampling::Cs420;
        assert!(raw_frame_bytes(0, 1, 8, layout).is_err());
        assert!(raw_frame_bytes(usize::MAX, 2, 8, layout).is_err());
        assert!(raw_frame_bytes(100_000, 100_000, 12, layout).is_err());
        assert!(parse_raw_yuv(&[0; 16], 3, 3, 8, layout).is_err());
        assert!(parse_raw_yuv(&[0; 18], 3, 3, 8, layout).is_err());
    }
}
