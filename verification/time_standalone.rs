//! Standalone verification of clock arithmetic when the workspace manifest is incomplete.
//! Keep this minimal shim restricted to the error variants used by time.rs.

#[derive(Debug, PartialEq, Eq)]
pub enum CodecError {
    InvalidData,
}

pub type CodecResult<T> = Result<T, CodecError>;

#[path = "../crates/codec-core/src/time.rs"]
mod time;
