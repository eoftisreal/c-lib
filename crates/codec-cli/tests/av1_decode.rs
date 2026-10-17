use std::{fs, process::Command};

#[test]
fn actual_pixel_output_and_unsupported_stream_preserves_existing_output() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::env::temp_dir().join(format!("codec-core-pixels-{}.yuv", std::process::id()));
    let run = |fixture: &str| {
        Command::new(env!("CARGO_BIN_EXE_av1-decode"))
            .arg("--input")
            .arg(root.join("tests").join(fixture))
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap()
    };
    let decoded = run("intra_lossless_core.obu");
    assert!(decoded.status.success(), "{:?}", decoded);
    assert!(decoded.stdout.is_empty());
    let expected = fs::read(root.join("tests/oracles/intra_lossless_core.yuv")).unwrap();
    assert_eq!(fs::read(&output).unwrap(), expected);
    let palette = run("intra_lossless.obu");
    assert!(palette.status.success(), "{:?}", palette);
    assert_eq!(fs::read(&output).unwrap(), expected);
    let rejected = run("intra_10bit.obu");
    assert!(!rejected.status.success());
    assert_eq!(fs::read(&output).unwrap(), expected);
    fs::remove_file(output).unwrap();
}

#[test]
fn lossy_and_12bit_chroma_outputs_are_native_and_failure_preserves_output() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output =
        std::env::temp_dir().join(format!("codec-intra-extended-{}.yuv", std::process::id()));
    let run = |fixture: &str| {
        Command::new(env!("CARGO_BIN_EXE_av1-decode"))
            .arg("--input")
            .arg(root.join("tests").join(fixture))
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap()
    };
    for fixture in [
        "lossy_4x4_full_q24",
        "intra_4x4_12_444_lossy",
        "intra_4x4_10_422_lossless",
    ] {
        let result = run(&format!("{fixture}.obu"));
        assert!(result.status.success(), "{fixture}: {result:?}");
        let expected = fs::read(root.join(format!("tests/oracles/{fixture}.yuv"))).unwrap();
        assert_eq!(fs::read(&output).unwrap(), expected);
    }
    let before = fs::read(&output).unwrap();
    assert!(!run("intra_10bit.obu").status.success());
    assert_eq!(fs::read(&output).unwrap(), before);
    fs::remove_file(&output).unwrap();
    assert!(!run("intra_10bit.obu").status.success());
    assert!(!output.exists());
}
