use std::process::Command;

#[test]
fn yuv_compare_reports_frame_plane_and_coordinates() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "c-lib-yuv-compare-{}-{}",
        std::process::id(),
        unique
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let expected_path = dir.join("expected.yuv");
    let actual_path = dir.join("actual.yuv");
    // Two 3x3 YUV420p 8-bit frames: 9Y + 4U + 4V = 17 samples.
    let expected = vec![42u8; 34];
    std::fs::write(&expected_path, &expected).unwrap();
    std::fs::write(&actual_path, &expected).unwrap();

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_yuv-compare"))
            .args(["--expected"])
            .arg(&expected_path)
            .args(["--actual"])
            .arg(&actual_path)
            .args([
                "--width",
                "3",
                "--height",
                "3",
                "--bit-depth",
                "8",
                "--subsampling",
                "420",
            ])
            .output()
            .unwrap()
    };
    let ok = run();
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    assert!(String::from_utf8_lossy(&ok.stdout).contains("Pixel-exact parity: 2 frames"));

    let mut actual = expected.clone();
    // The last U sample in the second frame: 17 + 9 + 3.
    actual[29] = 43;
    std::fs::write(&actual_path, &actual).unwrap();
    let bad = run();
    assert!(!bad.status.success());
    let message = String::from_utf8_lossy(&bad.stderr);
    assert!(message.contains("frame=1"), "{message}");
    assert!(message.contains("plane=U"), "{message}");
    assert!(
        message.contains("x=1 y=1 expected=42 actual=43"),
        "{message}"
    );

    // Truncated candidate must never be reported as a successful match.
    std::fs::write(&actual_path, &actual[..33]).unwrap();
    let truncated = run();
    assert!(!truncated.status.success());
    assert!(String::from_utf8_lossy(&truncated.stderr).contains("raw stream lengths"));

    std::fs::remove_dir_all(dir).unwrap();
}
