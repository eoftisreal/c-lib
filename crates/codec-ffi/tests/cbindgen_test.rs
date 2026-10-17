use std::env;
use std::fs;
use std::path::PathBuf;

#[test]
fn test_cbindgen_consistency() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let manifest_path = PathBuf::from(&manifest_dir);
    let root = manifest_path.parent().unwrap().parent().unwrap();

    let include_dir = root.join("include");
    let existing_header_path = include_dir.join("codec.h");
    let mut generated = Vec::new();

    cbindgen::Builder::new()
        .with_crate(&manifest_dir)
        .with_config(cbindgen::Config::from_file(root.join("cbindgen.toml")).unwrap())
        .generate()
        .expect("Unable to generate bindings")
        .write(&mut generated);

    let generated_header_content = String::from_utf8(generated).expect("UTF-8 bindings");
    let existing_header_content =
        fs::read_to_string(&existing_header_path).expect("checked-in header");

    let normalized_generated = generated_header_content.replace("\r\n", "\n");
    let normalized_existing = existing_header_content.replace("\r\n", "\n");

    if normalized_generated != normalized_existing {
        panic!(
            "The generated C header does not match include/codec.h.\n\
             Please check the differences, and run cbindgen to update include/codec.h."
        );
    }
}
