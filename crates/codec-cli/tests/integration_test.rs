use codec_av1::{obu::validate_sample, parse_obu_header, parse_sequence_header, ObuType};
use codec_bitstream::BitReader;
use codec_core::{CodecError, CodecId};
use container_mp4::Mp4Demuxer;
use std::process::Command;

const REAL: &[u8] = include_bytes!("../../../tests/real_av1.mp4");
const MULTI: &[u8] = include_bytes!("../../../tests/multi_av1.mp4");

#[test]
fn real_av1_mp4_all_samples() {
    for (bytes, expected) in [(REAL, 1), (MULTI, 12)] {
        let mut demux = Mp4Demuxer::new(bytes);
        demux.parse_moov().unwrap();
        assert_eq!(demux.streams[0].codec_id, CodecId::Av1);
        assert_eq!(
            (demux.streams[0].width, demux.streams[0].height),
            (320, 240)
        );
        let mut count = 0;
        loop {
            match demux.get_sample(0, count) {
                Ok(packet) => {
                    validate_sample(&packet.data).unwrap();
                    count += 1;
                }
                Err(CodecError::EndOfStream) => break,
                Err(error) => panic!("sample {count}: {error}"),
            }
        }
        assert_eq!(count, expected);
        let packet = demux.get_sample(0, 0).unwrap();
        assert_eq!(packet.dts.unwrap().pts, 0);
        let mut reader = BitReader::new(&packet.data);
        let mut found = false;
        while reader.bits_left() != 0 {
            let header = parse_obu_header(&mut reader).unwrap();
            let size = usize::try_from(reader.read_leb128().unwrap()).unwrap();
            let payload = reader.read_bytes(size).unwrap();
            if header.obu_type == ObuType::SequenceHeader {
                let seq = parse_sequence_header(&mut BitReader::new(payload)).unwrap();
                assert_eq!(
                    (seq.max_frame_width_minus_1, seq.max_frame_height_minus_1),
                    (319, 239)
                );
                found = true;
            }
        }
        assert!(found);
    }
}
#[test]
fn cli_reports_late_corruption() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ok = Command::new(env!("CARGO_BIN_EXE_codec-cli"))
        .args(["--input"])
        .arg(root.join("tests/multi_av1.mp4"))
        .output()
        .unwrap();
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    assert!(String::from_utf8_lossy(&ok.stdout).contains("12 AV1 samples"));
    let mut bytes = MULTI.to_vec();
    let mut demux = Mp4Demuxer::new(&bytes);
    demux.parse_moov().unwrap();
    let packet = demux.get_sample(0, 10).unwrap();
    let offset = bytes
        .windows(packet.data.len())
        .position(|w| w == packet.data)
        .unwrap();
    bytes[offset] |= 0x80; // forbidden OBU bit, after the old six-sample cutoff
    let path = std::env::temp_dir().join(format!("codec-malformed-{}.mp4", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    let bad = Command::new(env!("CARGO_BIN_EXE_codec-cli"))
        .arg("--input")
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("sample 10"));
}

#[test]
fn cli_strict_header_mode_reports_real_values_and_unsupported_interframes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(env!("CARGO_BIN_EXE_codec-cli"))
        .args(["--frame-headers", "--input"])
        .arg(root.join("tests/real_av1.mp4"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("Key 320x240"));
    assert!(text.contains("base Q 128, 1 tiles, 37 header bits"));
    let output = Command::new(env!("CARGO_BIN_EXE_codec-cli"))
        .args(["--frame-headers", "--input"])
        .arg(root.join("tests/multi_av1.mp4"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("sample 1"));
    assert!(error.contains("Feature not supported"));
}

#[test]
fn cli_md5_reports_compressed_sample_checksums() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(env!("CARGO_BIN_EXE_codec-cli"))
        .args(["--md5", "--input"])
        .arg(root.join("tests/multi_av1.mp4"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let digests: Vec<_> = stdout
        .lines()
        .filter_map(|line| line.split_once("compressed_sample_md5="))
        .map(|(_, digest)| digest)
        .collect();
    assert_eq!(digests.len(), 12);
    for digest in digests {
        assert_eq!(digest.len(), 32);
        assert!(digest.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}

#[test]
fn cli_y4m_refuses_to_write_placeholder_video() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path = std::env::temp_dir().join(format!("codec-y4m-{}.y4m", std::process::id()));
    std::fs::write(&path, b"keep existing output").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_codec-cli"))
        .arg("--input")
        .arg(root.join("tests/real_av1.mp4"))
        .arg("--y4m")
        .arg(&path)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("--y4m is unsupported: decoded AV1 pixel output is not implemented"));
    assert_eq!(std::fs::read(&path).unwrap(), b"keep existing output");
    std::fs::remove_file(&path).unwrap();
}
