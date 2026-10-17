//! Owned, stride-aware reconstructed YUV planes.
//!
//! A decoded picture is constructed explicitly from reconstructed samples.
//! No bitstream parser or CLI validation path may synthesize placeholder pixels.

use crate::{ChromaSubsampling, CodecError, CodecResult};

/// Maximum number of bytes occupied by active/padded planar u16 sample storage.
const MAX_STORAGE_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plane {
    width: usize,
    height: usize,
    stride: usize,
    samples: Vec<u16>,
}

impl Plane {
    pub fn new(width: usize, height: usize, stride: usize, samples: Vec<u16>) -> CodecResult<Self> {
        if width == 0 || height == 0 || stride < width {
            return Err(CodecError::InvalidData);
        }
        let count = stride.checked_mul(height).ok_or(CodecError::InvalidData)?;
        if count != samples.len() || count > MAX_STORAGE_BYTES / 2 {
            return Err(CodecError::InvalidData);
        }
        Ok(Self {
            width,
            height,
            stride,
            samples,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn stride(&self) -> usize {
        self.stride
    }

    pub fn row(&self, y: usize) -> Option<&[u16]> {
        if y >= self.height {
            return None;
        }
        let start = y * self.stride;
        Some(&self.samples[start..start + self.width])
    }

    pub fn row_mut(&mut self, y: usize) -> Option<&mut [u16]> {
        if y >= self.height {
            return None;
        }
        let start = y * self.stride;
        Some(&mut self.samples[start..start + self.width])
    }
}

/// Exactly one reconstructed planar YUV frame, not a compressed sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanarFrame {
    bit_depth: u8,
    subsampling: ChromaSubsampling,
    planes: [Plane; 3],
}

impl PlanarFrame {
    pub fn new(
        bit_depth: u8,
        subsampling: ChromaSubsampling,
        y: Plane,
        u: Plane,
        v: Plane,
    ) -> CodecResult<Self> {
        if !matches!(bit_depth, 8 | 10 | 12) {
            return Err(CodecError::Unsupported);
        }
        let (x_shift, y_shift) = match subsampling {
            ChromaSubsampling::Cs420 => (1, 1),
            ChromaSubsampling::Cs422 => (1, 0),
            ChromaSubsampling::Cs444 => (0, 0),
            ChromaSubsampling::Monochrome => return Err(CodecError::Unsupported),
        };
        let chroma_w = y.width.div_ceil(1 << x_shift);
        let chroma_h = y.height.div_ceil(1 << y_shift);
        if u.width != chroma_w
            || u.height != chroma_h
            || v.width != chroma_w
            || v.height != chroma_h
        {
            return Err(CodecError::InvalidData);
        }
        let used = [y.samples.len(), u.samples.len(), v.samples.len()]
            .into_iter()
            .try_fold(0usize, |sum, n| sum.checked_add(n.checked_mul(2)?))
            .ok_or(CodecError::InvalidData)?;
        if used > MAX_STORAGE_BYTES {
            return Err(CodecError::InvalidData);
        }
        let max_sample = (1u16 << bit_depth) - 1;
        for plane in [&y, &u, &v] {
            for row in 0..plane.height {
                if plane
                    .row(row)
                    .ok_or(CodecError::InvalidData)?
                    .iter()
                    .any(|&sample| sample > max_sample)
                {
                    return Err(CodecError::InvalidData);
                }
            }
        }
        Ok(Self {
            bit_depth,
            subsampling,
            planes: [y, u, v],
        })
    }

    pub fn bit_depth(&self) -> u8 {
        self.bit_depth
    }
    pub fn subsampling(&self) -> ChromaSubsampling {
        self.subsampling
    }
    pub fn width(&self) -> usize {
        self.planes[0].width
    }
    pub fn height(&self) -> usize {
        self.planes[0].height
    }
    pub fn planes(&self) -> &[Plane; 3] {
        &self.planes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaneId {
    Y,
    U,
    V,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelMismatch {
    pub plane: PlaneId,
    pub x: usize,
    pub y: usize,
    pub expected: u16,
    pub actual: u16,
}

/// Find the first mismatch in active pixel raster order (Y, then U, then V).
/// Padding is intentionally excluded. Returns InvalidData for incompatible layouts.
pub fn first_pixel_mismatch(
    expected: &PlanarFrame,
    actual: &PlanarFrame,
) -> CodecResult<Option<PixelMismatch>> {
    if expected.bit_depth != actual.bit_depth
        || expected.subsampling != actual.subsampling
        || expected.width() != actual.width()
        || expected.height() != actual.height()
    {
        return Err(CodecError::InvalidData);
    }
    for (index, (a, b)) in expected.planes.iter().zip(actual.planes.iter()).enumerate() {
        let plane = [PlaneId::Y, PlaneId::U, PlaneId::V][index];
        for y in 0..a.height {
            let expected_row = a.row(y).ok_or(CodecError::InvalidData)?;
            let actual_row = b.row(y).ok_or(CodecError::InvalidData)?;
            for (x, (&expected, &actual)) in expected_row.iter().zip(actual_row).enumerate() {
                if expected != actual {
                    return Ok(Some(PixelMismatch {
                        plane,
                        x,
                        y,
                        expected,
                        actual,
                    }));
                }
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_frame(y_stride: usize) -> PlanarFrame {
        let y = Plane::new(3, 3, y_stride, vec![7; 3 * y_stride]).unwrap();
        let u = Plane::new(2, 2, 2, vec![10; 4]).unwrap();
        let v = Plane::new(2, 2, 2, vec![11; 4]).unwrap();
        PlanarFrame::new(8, ChromaSubsampling::Cs420, y, u, v).unwrap()
    }

    #[test]
    fn odd_dimensions_with_padding_are_identical() {
        let a = sample_frame(3);
        let mut b = sample_frame(5);
        assert_eq!(first_pixel_mismatch(&a, &b).unwrap(), None);
        b.planes[1].row_mut(1).unwrap()[1] = 200;
        assert_eq!(
            first_pixel_mismatch(&a, &b).unwrap(),
            Some(PixelMismatch {
                plane: PlaneId::U,
                x: 1,
                y: 1,
                expected: 10,
                actual: 200
            })
        );
    }

    #[test]
    fn invalid_stride_layout_depth_and_value_are_rejected() {
        assert!(Plane::new(3, 2, 2, vec![0; 4]).is_err());
        assert!(Plane::new(3, 2, usize::MAX, vec![]).is_err());
        assert!(Plane::new(3, 2, 3, vec![0; 5]).is_err());
        let a = sample_frame(3);
        assert!(PlanarFrame::new(
            9,
            ChromaSubsampling::Cs420,
            a.planes[0].clone(),
            a.planes[1].clone(),
            a.planes[2].clone()
        )
        .is_err());
        assert!(PlanarFrame::new(
            8,
            ChromaSubsampling::Cs444,
            a.planes[0].clone(),
            a.planes[1].clone(),
            a.planes[2].clone()
        )
        .is_err());
        let mut bad_y = a.planes[0].clone();
        bad_y.row_mut(0).unwrap()[0] = 256;
        assert!(PlanarFrame::new(
            8,
            ChromaSubsampling::Cs420,
            bad_y,
            a.planes[1].clone(),
            a.planes[2].clone()
        )
        .is_err());
    }

    #[test]
    fn sample_validation_ignores_stride_padding() {
        let y = Plane::new(1, 1, 2, vec![255, 65535]).unwrap();
        let u = Plane::new(1, 1, 1, vec![0]).unwrap();
        let v = Plane::new(1, 1, 1, vec![0]).unwrap();
        assert!(PlanarFrame::new(8, ChromaSubsampling::Cs420, y, u, v).is_ok());
    }
}
