//! Decode the verified intra OBU subset to native raw planar YUV.
use clap::Parser;
use std::{
    fs::File,
    io::{BufWriter, Read, Write},
    path::PathBuf,
};

#[derive(Parser)]
#[command(about = "Decode one supported AV1 key picture to native raw YUV (constrained subset)")]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    const MAX_INPUT: usize = 16 * 1024 * 1024;
    let mut input = Vec::new();
    File::open(&args.input)?
        .take((MAX_INPUT + 1) as u64)
        .read_to_end(&mut input)?;
    if input.len() > MAX_INPUT {
        return Err("input exceeds 16 MiB".into());
    }
    let frame = codec_av1::decode_intra_frame(&input, &mut codec_av1::DecoderState::default())?;
    // Do not create/truncate an output when compressed-picture decoding fails.
    let mut output = BufWriter::new(File::create(&args.output)?);
    codec_cli::y4m::write_raw_yuv(&mut output, &frame)?;
    output.flush()?;
    Ok(())
}

fn main() {
    if let Err(error) = run(Args::parse()) {
        eprintln!("AV1 picture decode failed: {error}");
        std::process::exit(1);
    }
}
