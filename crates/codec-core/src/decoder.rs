use crate::error::{CodecError, CodecResult};
use crate::frame::{ChromaSubsampling, Frame};
use crate::packet::Packet;
use std::marker::PhantomData;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecId {
    Av1,
    Av2,
    H264,
    Hevc,
    Vvc,
    Aac,
    Opus,
    Flac,
    Webvtt,
    Tx3g,
}

#[derive(Debug, Clone)]
pub struct DecoderConfig {
    pub codec: CodecId,
    pub extradata: Vec<u8>,
    pub threads: usize,
    pub suppress_cpu_grain: bool,
    pub operating_point: u8,
    pub output_invisible_frames: bool,
}

impl Default for DecoderConfig {
    fn default() -> Self {
        Self {
            codec: CodecId::Av1,
            extradata: Vec::new(),
            threads: 1,
            suppress_cpu_grain: false,
            operating_point: 0,
            output_invisible_frames: false,
        }
    }
}

/// Symphonia-aligned codec-agnostic parameter descriptor extracted by container demuxers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecParams {
    pub codec_id: CodecId,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub subsampling: ChromaSubsampling,
    pub extradata: Vec<u8>,
    pub time_base: Option<(u32, u32)>,
}

/// Spatial subregion descriptor abstracting AV1 tile grids and H.266/VVC independent
/// subpictures for selective viewport/frustum decoding in 8K360/VR pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubregionDescriptor {
    pub id: u32,
    pub x_px: u32,
    pub y_px: u32,
    pub width_px: u32,
    pub height_px: u32,
    pub independent_entropy: bool,
    pub independent_motion: bool,
}

pub trait SpatialSubregion {
    fn subregions(&self) -> Vec<SubregionDescriptor>;
    fn select_viewport_subregions(
        &self,
        viewport_x: u32,
        viewport_y: u32,
        viewport_w: u32,
        viewport_h: u32,
    ) -> Vec<u32> {
        self.subregions()
            .into_iter()
            .filter(|r| {
                let r_end_x = r.x_px.saturating_add(r.width_px);
                let r_end_y = r.y_px.saturating_add(r.height_px);
                let v_end_x = viewport_x.saturating_add(viewport_w);
                let v_end_y = viewport_y.saturating_add(viewport_h);
                r.x_px < v_end_x && r_end_x > viewport_x && r.y_px < v_end_y && r_end_y > viewport_y
            })
            .map(|r| r.id)
            .collect()
    }
}

/// Composable post-reconstruction normative in-loop filter stage abstraction
/// across AV1 (Deblock -> CDEF -> Wiener/SGRPROJ), AV2 (GDF, PC-Wiener, CCSO), and VVC (SAO, ALF).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterStageKind {
    Deblock,
    DirectionalEnhance,
    Restoration,
    CrossComponentOffset,
}

pub trait InLoopFilterStage: Send + Sync {
    fn stage_kind(&self) -> FilterStageKind;
    fn halo_rows_required(&self) -> usize;
    #[allow(clippy::too_many_arguments)]
    fn apply_superblock_row(
        &self,
        plane: &mut [u16],
        stride: usize,
        width: usize,
        height: usize,
        sb_row_y: usize,
        sb_height: usize,
        bit_depth: u8,
    ) -> CodecResult<()>;
}

pub trait Decoder {
    fn configure(&mut self, config: &DecoderConfig) -> CodecResult<()>;
    fn submit_packet(&mut self, packet: &Packet) -> CodecResult<()>;
    fn receive_frame(&mut self) -> CodecResult<Option<Box<dyn Frame>>>;
    fn flush(&mut self) -> CodecResult<()>;
    fn reset(&mut self) -> CodecResult<()> {
        self.flush()
    }
}

/// Typestate markers enforcing discontinuous seek reset invariants at compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Discontinuous;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AwaitingKeyframe;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Active;

/// Compile-time typestate wrapper around any `Decoder` enforcing:
/// `TypestateDecoder<D, Active>` --seek--> `TypestateDecoder<D, Discontinuous>`
/// --reset--> `TypestateDecoder<D, AwaitingKeyframe>` --keyframe--> `TypestateDecoder<D, Active>`.
#[derive(Debug)]
pub struct TypestateDecoder<D, State = AwaitingKeyframe> {
    inner: D,
    _state: PhantomData<State>,
}

impl<D: Decoder> TypestateDecoder<D, AwaitingKeyframe> {
    pub fn new(inner: D) -> Self {
        Self {
            inner,
            _state: PhantomData,
        }
    }

    pub fn submit_keyframe(
        mut self,
        packet: &Packet,
    ) -> Result<TypestateDecoder<D, Active>, (Self, CodecError)> {
        if !packet.is_keyframe {
            return Err((self, CodecError::InvalidData));
        }
        if let Err(err) = self.inner.submit_packet(packet) {
            let _ = self.inner.reset();
            return Err((self, err));
        }
        Ok(TypestateDecoder {
            inner: self.inner,
            _state: PhantomData,
        })
    }
}

impl<D: Decoder> TypestateDecoder<D, Active> {
    pub fn submit_packet(&mut self, packet: &Packet) -> CodecResult<()> {
        if let Err(err) = self.inner.submit_packet(packet) {
            let _ = self.inner.reset();
            return Err(err);
        }
        Ok(())
    }

    pub fn receive_frame(&mut self) -> CodecResult<Option<Box<dyn Frame>>> {
        self.inner.receive_frame()
    }

    pub fn begin_seek(self) -> TypestateDecoder<D, Discontinuous> {
        TypestateDecoder {
            inner: self.inner,
            _state: PhantomData,
        }
    }

    pub fn into_inner(self) -> D {
        self.inner
    }
}

impl<D: Decoder> TypestateDecoder<D, Discontinuous> {
    pub fn reset_for_keyframe(mut self) -> CodecResult<TypestateDecoder<D, AwaitingKeyframe>> {
        self.inner.reset()?;
        Ok(TypestateDecoder {
            inner: self.inner,
            _state: PhantomData,
        })
    }
}
