use crate::time::Timestamp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Yuv420p,
    Yuv422p,
    Yuv444p,
    Yuv420p10,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromaSubsampling {
    Cs420,
    Cs422,
    Cs444,
    Monochrome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    Bt709,
    Bt2020,
    Srgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorRange {
    Limited,
    Full,
}

#[derive(Debug, Clone)]
pub struct HDRMetadata {
    pub max_cll: u16,
    pub max_fall: u16,
}

pub trait Frame: Send + Sync {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn format(&self) -> PixelFormat;
    fn pts(&self) -> Option<Timestamp>;
}
