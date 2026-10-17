//! Reusable verification and output primitives for decoded YUV pictures.
//!
//! `av1-decode` reconstructs the verified lossless OBU subset.
//! The primary `codec-cli` binary retains its syntax-validation behavior.
pub mod oracle;
pub mod y4m;
