//! Validate every WGSL shader offline with naga (the compiler wgpu uses),
//! so shader errors fail `cargo test` even on machines without a GPU.

fn validate(name: &str, src: &str) {
    let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{name}: parse error:\n{}", e.emit_to_string(src)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: validation error: {e:?}"));
}

#[test]
fn all_shaders_validate() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shaders");
    let mut n = 0;
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("wgsl") {
            validate(&p.file_name().unwrap().to_string_lossy(), &std::fs::read_to_string(&p).unwrap());
            n += 1;
        }
    }
    assert!(n >= 8, "expected all shaders, found {n}");
}
