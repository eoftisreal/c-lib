//! YUV4MPEG2 and planar raw-YUV output of verified reconstructed pictures.
//!
//! This module does not decode AV1. Callers must provide real decoded pixels.

use codec_core::{ChromaSubsampling, PlanarFrame};
use std::io::{self, Write};

fn invalid_input(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn chroma_tag(frame: &PlanarFrame) -> &'static str {
    match (frame.subsampling(), frame.bit_depth()) {
        (ChromaSubsampling::Cs420, 8) => "420jpeg",
        (ChromaSubsampling::Cs420, 10) => "420p10",
        (ChromaSubsampling::Cs420, 12) => "420p12",
        (ChromaSubsampling::Cs422, 8) => "422",
        (ChromaSubsampling::Cs422, 10) => "422p10",
        (ChromaSubsampling::Cs422, 12) => "422p12",
        (ChromaSubsampling::Cs444, 8) => "444",
        (ChromaSubsampling::Cs444, 10) => "444p10",
        (ChromaSubsampling::Cs444, 12) => "444p12",
        _ => unreachable!("PlanarFrame validates supported bit depths and layouts"),
    }
}

fn write_active_planes<W: Write>(writer: &mut W, frame: &PlanarFrame) -> io::Result<()> {
    for plane in frame.planes() {
        for y in 0..plane.height() {
            let row = plane
                .row(y)
                .ok_or_else(|| invalid_input("invalid plane row"))?;
            if frame.bit_depth() == 8 {
                // Avoid allocating or serializing stride padding.
                for &sample in row {
                    writer.write_all(&[sample as u8])?;
                }
            } else {
                for &sample in row {
                    writer.write_all(&sample.to_le_bytes())?;
                }
            }
        }
    }
    Ok(())
}

/// Write raw planar YUV samples in Y, U, V order with no stride padding.
/// For 10/12-bit sources, samples are little-endian 16-bit words.
pub fn write_raw_yuv<W: Write>(writer: &mut W, frame: &PlanarFrame) -> io::Result<()> {
    write_active_planes(writer, frame)
}

pub struct Y4mWriter<W: Write> {
    inner: W,
    width: usize,
    height: usize,
    depth: u8,
    layout: ChromaSubsampling,
}

impl<W: Write> Y4mWriter<W> {
    /// Begin a Y4M stream with the dimensions/layout of the first real frame.
    pub fn new(mut inner: W, frame: &PlanarFrame, fps_num: u32, fps_den: u32) -> io::Result<Self> {
        if fps_num == 0 || fps_den == 0 {
            return Err(invalid_input("Y4M frame rate must be positive"));
        }
        writeln!(
            inner,
            "YUV4MPEG2 W{} H{} F{}:{} Ip A0:0 C{}",
            frame.width(),
            frame.height(),
            fps_num,
            fps_den,
            chroma_tag(frame)
        )?;
        Ok(Self {
            inner,
            width: frame.width(),
            height: frame.height(),
            depth: frame.bit_depth(),
            layout: frame.subsampling(),
        })
    }

    pub fn write_frame(&mut self, frame: &PlanarFrame) -> io::Result<()> {
        if frame.width() != self.width
            || frame.height() != self.height
            || frame.bit_depth() != self.depth
            || frame.subsampling() != self.layout
        {
            return Err(invalid_input("Y4M frame format changed after header"));
        }
        self.inner.write_all(b"FRAME\n")?;
        write_active_planes(&mut self.inner, frame)
    }

    pub fn into_inner(self) -> W {
        self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codec_core::Plane;

    fn image(depth: u8) -> PlanarFrame {
        let y = Plane::new(
            3,
            3,
            4,
            vec![1, 2, 3, 65535, 4, 5, 6, 65535, 7, 8, 9, 65535],
        )
        .unwrap();
        let u = Plane::new(2, 2, 3, vec![10, 11, 65535, 12, 13, 65535]).unwrap();
        let v = Plane::new(2, 2, 2, vec![14, 15, 16, 17]).unwrap();
        PlanarFrame::new(depth, ChromaSubsampling::Cs420, y, u, v).unwrap()
    }

    #[test]
    fn y4m_8bit_odd_dimensions_without_stride_padding() {
        let f = image(8);
        let mut stream = Y4mWriter::new(Vec::new(), &f, 24000, 1001).unwrap();
        stream.write_frame(&f).unwrap();
        let bytes = stream.into_inner();
        let mut expected = b"YUV4MPEG2 W3 H3 F24000:1001 Ip A0:0 C420jpeg\nFRAME\n".to_vec();
        expected.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17]);
        assert_eq!(bytes, expected);
    }

    #[test]
    fn y4m_10bit_le_serialization_and_raw_yuv() {
        let f = image(10);
        let mut raw = Vec::new();
        write_raw_yuv(&mut raw, &f).unwrap();
        assert_eq!(raw.len(), 34);
        assert_eq!(&raw[..6], &[1, 0, 2, 0, 3, 0]);

        let mut stream = Y4mWriter::new(Vec::new(), &f, 24, 1).unwrap();
        stream.write_frame(&f).unwrap();
        let bytes = stream.into_inner();
        let start = b"YUV4MPEG2 W3 H3 F24:1 Ip A0:0 C420p10\nFRAME\n";
        assert!(bytes.starts_with(start));
        assert_eq!(&bytes[start.len()..], raw);
    }

    #[test]
    fn invalid_rate_or_format_change_is_rejected() {
        let f = image(8);
        assert!(Y4mWriter::new(Vec::new(), &f, 0, 1).is_err());
        let mut stream = Y4mWriter::new(Vec::new(), &f, 30, 1).unwrap();
        assert!(stream.write_frame(&image(10)).is_err());
        assert_eq!(
            stream.into_inner(),
            b"YUV4MPEG2 W3 H3 F30:1 Ip A0:0 C420jpeg\n"
        );
    }
}
