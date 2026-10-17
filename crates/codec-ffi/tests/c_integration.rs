use std::{env, fs, path::PathBuf, process::Command};

#[test]
fn test_c_abi() {
    compile_and_run("crates/codec-ffi/tests/abi.c");
    compile_and_run("examples/api_version.c");
}

fn compile_and_run(source: &str) {
    // cargo test builds the cdylib in the test executable's dependency directory.
    let deps = env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = deps.join("c-abi-check");
    fs::create_dir_all(&out).unwrap();
    let exe = out.join(if cfg!(windows) { "abi.exe" } else { "abi" });
    let compiler = env::var("CC").unwrap_or_else(|_| {
        if cfg!(target_env = "msvc") {
            "cl"
        } else {
            "cc"
        }
        .into()
    });
    let mut command = Command::new(&compiler);
    command.current_dir(&out);
    if cfg!(target_env = "msvc") {
        command
            .arg("/nologo")
            .arg("/W4")
            .arg("/WX")
            .arg("/std:c11")
            .arg(format!("/I{}", root.join("include").display()))
            .arg(root.join(source))
            .arg(deps.join("codec_ffi.dll.lib"))
            .arg(format!("/Fe:{}", exe.display()));
    } else {
        let library = deps.join(if cfg!(target_os = "macos") {
            "libcodec_ffi.dylib"
        } else if cfg!(windows) {
            "libcodec_ffi.dll.a"
        } else {
            "libcodec_ffi.so"
        });
        command
            .args([
                "-std=c11",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-pedantic-errors",
            ])
            .arg(root.join(source))
            .arg("-I")
            .arg(root.join("include"))
            .arg(library)
            .arg("-o")
            .arg(&exe);
        if !cfg!(windows) {
            command.arg(format!("-Wl,-rpath,{}", deps.display()));
        }
    }
    let result = command
        .output()
        .unwrap_or_else(|error| panic!("Required C compiler {compiler} unavailable: {error}"));
    assert!(
        result.status.success(),
        "C compile/link failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut run = Command::new(exe);
    for variable in ["PATH", "LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH"] {
        let mut paths = vec![deps.clone()];
        if let Some(value) = env::var_os(variable) {
            paths.extend(env::split_paths(&value));
        }
        run.env(variable, env::join_paths(paths).unwrap());
    }
    assert!(run.status().expect("C ABI executable must run").success());
}
