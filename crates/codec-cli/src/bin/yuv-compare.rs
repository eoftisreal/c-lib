//! Pixel-exact comparison of two already-decoded raw planar YUV streams.
//!
//! Example:
//! cargo run -p codec-cli --bin yuv-compare -- --expected reference.yuv
//!     --actual candidate.yuv --width 320 --height 240 --bit-depth 8
//!     --subsampling 420

use clap::Parser;
use codec_cli::oracle::{compare_frame_bytes, raw_frame_bytes};
use codec_core::ChromaSubsampling;
use std::fs::File;
use std::io::{self, Read};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(about = "Compare independently decoded raw planar YUV frames pixel for pixel")]
struct Args {
    #[arg(long)]
    expected: PathBuf,
    #[arg(long)]
    actual: PathBuf,
    #[arg(long)]
    width: usize,
    #[arg(long)]
    height: usize,
    #[arg(long)]
    bit_depth: u8,
    #[arg(long, value_parser = ["420", "422", "444"])]
    subsampling: String,
}

fn compare(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let layout = match args.subsampling.as_str() {
        "420" => ChromaSubsampling::Cs420,
        "422" => ChromaSubsampling::Cs422,
        "444" => ChromaSubsampling::Cs444,
        _ => return Err("unsupported chroma subsampling".into()),
    };
    let frame_bytes = raw_frame_bytes(args.width, args.height, args.bit_depth, layout)?;
    let mut expected = File::open(&args.expected)?;
    let mut actual = File::open(&args.actual)?;
    let expected_bytes = expected.metadata()?.len();
    let actual_bytes = actual.metadata()?.len();
    let frame_bytes_u64 = u64::try_from(frame_bytes)?;
    if expected_bytes == 0
        || expected_bytes != actual_bytes
        || expected_bytes % frame_bytes_u64 != 0
    {
        return Err(format!(
            "raw stream lengths must match, be nonzero and contain whole frames: expected {} bytes, actual {} bytes, frame size {} bytes",
            expected_bytes, actual_bytes, frame_bytes
        ).into());
    }
    let count = expected_bytes / frame_bytes_u64;
    let allocate = || -> io::Result<Vec<u8>> {
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(frame_bytes)
            .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        buffer.resize(frame_bytes, 0);
        Ok(buffer)
    };
    let mut expected_frame = allocate()?;
    let mut actual_frame = allocate()?;
    for index in 0..count {
        expected.read_exact(&mut expected_frame)?;
        actual.read_exact(&mut actual_frame)?;
        if let Some(mismatch) = compare_frame_bytes(
            &expected_frame,
            &actual_frame,
            args.width,
            args.height,
            args.bit_depth,
            layout,
            usize::try_from(index)?,
        )? {
            return Err(format!(
                "pixel mismatch: frame={} plane={:?} x={} y={} expected={} actual={}",
                mismatch.frame_index,
                mismatch.pixel.plane,
                mismatch.pixel.x,
                mismatch.pixel.y,
                mismatch.pixel.expected,
                mismatch.pixel.actual
            )
            .into());
        }
    }
    // A changing file must not lead to a false "passed" verdict.
    let mut extra = [0u8; 1];
    if expected.read(&mut extra)? != 0 || actual.read(&mut extra)? != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "raw stream grew during comparison",
        )
        .into());
    }
    println!(
        "Pixel-exact parity: {} frames, {}x{}, {}-bit, {} YUV",
        count, args.width, args.height, args.bit_depth, args.subsampling
    );
    Ok(())
}

fn main() {
    let args = Args::parse();
    if let Err(error) = compare(&args) {
        eprintln!("YUV verification failed: {error}");
        std::process::exit(1);
    }
}
