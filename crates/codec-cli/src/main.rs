mod digest;

use clap::Parser;
use codec_bitstream::BitReader;
use codec_core::{CodecError, CodecId};
use codec_dsl::{CompiledPipelineDag, FrameEvalContext};
use container_mp4::Mp4Demuxer;
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Validate MP4 sample tables and AV1 syntax, inspect headers, and measure parsing throughput"
)]
struct Args {
    #[arg(short, long)]
    input: String,
    /// Strict supported intra-frame header parsing; unsupported features fail explicitly.
    #[arg(long)]
    frame_headers: bool,
    /// Extended inter/switch/show-existing frame header diagnostics across multi-frame streams.
    #[arg(long)]
    inter_headers: bool,
    /// Reserved: Y4M export requires a pixel decoder and currently returns an error.
    #[arg(long)]
    y4m: Option<String>,
    /// Print real MD5 checksums of compressed AV1 sample bytes (not decoded planes).
    #[arg(long)]
    md5: bool,
    /// Measure container and AV1 syntax-validation throughput (not decoding speed).
    #[arg(long)]
    benchmark: bool,
    /// Compile and evaluate a declarative playback DSL pipeline model.
    #[arg(long)]
    dsl: Option<String>,
}

fn run(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    if args.y4m.is_some() {
        return Err("--y4m is unsupported: decoded AV1 pixel output is not implemented".into());
    }
    let start_time = Instant::now();
    if let Some(ref dsl_src) = args.dsl {
        let dag = CompiledPipelineDag::compile_str(dsl_src)?;
        let plan = dag.evaluate(&FrameEvalContext::default());
        println!(
            "Compiled DSL pipeline: {} active stages, suppress_cpu_grain={}, neural_lookahead_frames={}",
            plan.ordered_stages.len(),
            plan.suppress_cpu_grain,
            plan.neural_lookahead_frames
        );
    }

    let data = std::fs::read(&args.input)?;
    let mut demuxer = Mp4Demuxer::new(&data);
    demuxer.parse_moov()?;
    let mut total = 0;
    let mut av1_tracks = 0;
    for (index, stream) in demuxer.streams.iter().enumerate() {
        if stream.codec_id != CodecId::Av1 {
            continue;
        }
        av1_tracks += 1;
        let mut state = codec_av1::DecoderState::default();
        let mut seq_header = None;
        if stream.extradata.len() > 4 {
            if args.frame_headers {
                codec_av1::obu::validate_sample_with_state(&stream.extradata[4..], &mut state)?;
                seq_header = state.sequence().cloned();
            } else {
                codec_av1::obu::validate_sample(&stream.extradata[4..])?;
                let mut r = BitReader::new(&stream.extradata[4..]);
                while r.bits_left() != 0 {
                    let obu = codec_av1::parse_obu_header(&mut r)?;
                    let sz = usize::try_from(r.read_leb128()?)?;
                    let payload = r.read_bytes(sz)?;
                    if obu.obu_type == codec_av1::ObuType::SequenceHeader {
                        seq_header = Some(codec_av1::parse_sequence_header(&mut BitReader::new(
                            payload,
                        ))?);
                    }
                }
            }
        }
        let mut sample = 0;
        loop {
            let packet = match demuxer.get_sample(index, sample) {
                Ok(packet) => packet,
                Err(CodecError::EndOfStream) => break,
                Err(error) => return Err(format!("track {index}, sample {sample}: {error}").into()),
            };
            codec_av1::obu::validate_sample(&packet.data)
                .map_err(|error| format!("track {index}, sample {sample}: {error}"))?;
            if args.frame_headers {
                let result = codec_av1::obu::validate_sample_with_state(&packet.data, &mut state)
                    .map_err(|error| {
                    format!("track {index}, sample {sample}: frame-header validation: {error}")
                })?;
                for h in result.frame_headers {
                    println!(
                        "track {index}, sample {sample}: {:?} {}x{} render {}x{}, order hint {}, base Q {}, {} tiles, {} header bits",
                        h.frame_type,
                        h.dimensions.width,
                        h.dimensions.height,
                        h.dimensions.render_width,
                        h.dimensions.render_height,
                        h.order_hint,
                        h.quantization.base_q_idx,
                        h.tiles.count(),
                        h.header_bits
                    );
                }
            }
            if args.inter_headers {
                let mut r = BitReader::new(&packet.data);
                while r.bits_left() != 0 {
                    let obu = codec_av1::parse_obu_header(&mut r)?;
                    let sz = usize::try_from(r.read_leb128()?)?;
                    let payload = r.read_bytes(sz)?;
                    if obu.obu_type == codec_av1::ObuType::SequenceHeader {
                        seq_header = Some(codec_av1::parse_sequence_header(&mut BitReader::new(
                            payload,
                        ))?);
                    } else if matches!(
                        obu.obu_type,
                        codec_av1::ObuType::Frame | codec_av1::ObuType::FrameHeader
                    ) {
                        if let Some(ref seq) = seq_header {
                            let ext = codec_av1::frame::ExtendedFrameHeader::parse_prefix(
                                &mut BitReader::new(payload),
                                seq,
                            )?;
                            println!(
                                "track {index}, sample {sample}: type={:?} show={} show_existing={} order_hint={} refresh={:#04x}",
                                ext.frame_type,
                                ext.show_frame,
                                ext.show_existing_frame,
                                ext.order_hint,
                                ext.refresh_frame_flags
                            );
                        }
                    }
                }
            }
            if args.md5 {
                println!(
                    "track {index}, sample {sample}: compressed_sample_md5={}",
                    digest::md5_hex(&packet.data)
                );
            }
            sample += 1;
        }
        if args.frame_headers && !matches!(state.parser(), codec_av1::ParserState::AwaitingHeader) {
            return Err(format!("track {index}: missing tile groups at end of stream").into());
        }
        total += sample;
    }
    if av1_tracks == 0 || total == 0 {
        return Err("No nonempty AV1 track found".into());
    }
    let scope = if args.frame_headers {
        "supported intra headers and tile framing; no pixel decoding"
    } else {
        "OBU framing and sequence syntax only"
    };
    println!("Validated {total} AV1 samples across {av1_tracks} tracks ({scope}).");
    if args.benchmark {
        let elapsed = start_time.elapsed();
        println!(
            "Benchmark: processed {total} samples in {:.3} ms ({:.1} samples/sec)",
            elapsed.as_secs_f64() * 1000.0,
            (total as f64) / elapsed.as_secs_f64().max(1e-9)
        );
    }
    Ok(())
}

fn main() {
    let args = Args::parse();
    if let Err(error) = run(&args) {
        eprintln!("Validation failed: {error}");
        std::process::exit(1);
    }
}
