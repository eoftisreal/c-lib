//! Native AV1 syntax and scalar decoding primitives.
//!
//! Decode one complete picture in the verified 8-bit lossless DC subset:
//! ```
//! use codec_av1::{decode_intra_frame, DecoderState};
//! let packet = include_bytes!("../../../tests/intra_lossless_core.obu");
//! let mut state = DecoderState::default();
//! let pixels = decode_intra_frame(packet, &mut state)?;
//! assert_eq!((pixels.width(), pixels.height()), (128, 96));
//! assert_eq!(pixels.planes()[0].row(0).unwrap()[0], 74);
//! # Ok::<(), codec_core::CodecError>(())
//! ```
//! See [`decode_intra_frame`] for scope and failure semantics.

pub mod obu;
pub mod sequence;
pub mod vulkan;

pub use obu::{parse_obu_header, ObuHeader, ObuType};
pub use sequence::{parse_sequence_header, SequenceHeader};

pub mod entropy;
pub mod frame;
pub use frame::{decode_intra_frame, DecoderState, ParserState};
