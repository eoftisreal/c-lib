//! Validated, owned planar pixels and decoder infrastructure.
//!
//! This example assembles supplied pixels; it does not decode compressed AV1.
//! Rows contain only active samples; a plane can also own stride padding.
//!
//! ```
//! use codec_core::{ChromaSubsampling, PlanarFrame, Plane};
//! let y = Plane::new(2, 2, 2, vec![16; 4])?;
//! let u = Plane::new(1, 1, 1, vec![128])?;
//! let v = Plane::new(1, 1, 1, vec![128])?;
//! let frame = PlanarFrame::new(8, ChromaSubsampling::Cs420, y, u, v)?;
//! assert_eq!(frame.width(), 2);
//! # Ok::<(), codec_core::CodecError>(())
//! ```

pub mod decoder;
pub mod error;
pub mod frame;
pub mod hwaccel;
pub mod packet;
pub mod picture;
pub mod simd;
pub mod stream;
pub mod threading;
pub mod time;

pub use decoder::{
    Active, AwaitingKeyframe, CodecId, CodecParams, Decoder, DecoderConfig, Discontinuous,
    FilterStageKind, InLoopFilterStage, SpatialSubregion, SubregionDescriptor, TypestateDecoder,
};
pub use error::{CodecError, CodecResult};
pub use frame::{ChromaSubsampling, ColorRange, ColorSpace, Frame, HDRMetadata, PixelFormat};
pub use hwaccel::{
    VkVideoDecodeAV1PictureInfoKHR, VkVideoDecodeAV1ProfileInfoKHR, VkVideoDecodeInfoKHR,
    VK_VIDEO_CODEC_OPERATION_DECODE_AV1_BIT_KHR,
};
pub use packet::Packet;
pub use picture::{first_pixel_mismatch, PixelMismatch, PlanarFrame, Plane, PlaneId};
pub use simd::{CpuFlags, DspContext};
pub use stream::StreamInfo;
pub use threading::SuperblockWavefrontGrid;
pub use time::{
    KalmanVsyncEstimator, PidClockController, Rational128, SyncStepOutput, Timestamp,
    VsyncCadenceStep,
};
