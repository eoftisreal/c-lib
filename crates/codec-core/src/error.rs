use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    #[error("Invalid input data")]
    InvalidData,
    #[error("Malformed bitstream: {0}")]
    MalformedBitstream(&'static str),
    #[error("Feature not supported")]
    Unsupported,
    #[error("Allocation failed")]
    OutOfMemory,
    #[error("Buffer too small")]
    BufferTooSmall,
    #[error("Need more data")]
    NeedMoreData,
    #[error("End of stream")]
    EndOfStream,
    #[error("Hardware validation invariant violated: {0}")]
    HardwareValidation(&'static str),
    #[error("Internal decoder error: {0}")]
    Internal(String),
}

pub type CodecResult<T> = Result<T, CodecError>;
