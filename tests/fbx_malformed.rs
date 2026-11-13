//! Malformed / hostile FBX input must produce an error - never a panic,
//! hang, stack overflow or giant allocation (any of which would take the
//! whole application down while live-syncing a broken export).

use lumencraft::fbx;
use lumencraft::import::TextureResolver;
use std::io::Write;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

fn data(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
}

/// Run `f` on a thread with a deadline: a hang fails the test.
fn within<T: Send + 'static>(secs: u64, what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let _ = tx.send(f());
        })
        .unwrap();
    rx.recv_timeout(Duration::from_secs(secs)).unwrap_or_else(|_| panic!("{what}: did not finish within {secs}s (hang)"))
}

/// Parse + build; returns whether it succeeded. Panics propagate (= test failure).
fn try_parse(bytes: &[u8]) -> bool {
    let dummy = data("box_binary.fbx");
    let nodes = if bytes.starts_with(b"Kaydara FBX Binary") {
        match fbx::parse_binary(bytes) {
            Ok((_, n)) => n,
            Err(_) => return false,
        }
    } else {
        match fbx::parse_ascii(&String::from_utf8_lossy(bytes)) {
            Ok(n) => n,
            Err(_) => return false,
        }
    };
    fbx::build_scene(&nodes, &mut TextureResolver::new(&dummy, &[])).is_ok()
}

fn header(version: u32) -> Vec<u8> {
    let mut h = b"Kaydara FBX Binary  \x00\x1a\x00".to_vec();
    h.extend_from_slice(&version.to_le_bytes());
    assert_eq!(h.len(), 27);
    h
}

/// One 7.4-format node record with raw property bytes and no children.
fn node(name: &str, nprops: u32, props: &[u8], end_override: Option<u32>, offset: usize) -> Vec<u8> {
    let len = 13 + name.len() + props.len();
    let end = end_override.unwrap_or((offset + len) as u32);
    let mut v = Vec::new();
    v.extend_from_slice(&end.to_le_bytes());
    v.extend_from_slice(&nprops.to_le_bytes());
    v.extend_from_slice(&(props.len() as u32).to_le_bytes());
    v.push(name.len() as u8);
    v.extend_from_slice(name.as_bytes());
    v.extend_from_slice(props);
    v
}

#[test]
fn truncated_files_fail_cleanly() {
    for f in ["phong_cube.fbx", "spider.fbx", "box_ascii.fbx", "cubes_with_names.fbx"] {
        let d = std::fs::read(data(f)).unwrap();
        within(60, f, move || {
            let step = (d.len() / 300).max(1);
            for len in (0..d.len()).step_by(step) {
                let _ = try_parse(&d[..len]);
            }
        });
    }
}

#[test]
fn corrupted_bytes_fail_cleanly() {
    for f in ["phong_cube.fbx", "spider.fbx", "global_settings.fbx", "box_ascii.fbx"] {
        let d = std::fs::read(data(f)).unwrap();
        within(120, f, move || {
            let mut seed = 0x2545F491u32;
            let mut rnd = move || {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed
            };
            for _ in 0..300 {
                let mut c = d.clone();
                for _ in 0..1 + rnd() % 8 {
                    let i = rnd() as usize % c.len();
                    // flip bits, or write an extreme value (lengths/offsets)
                    c[i] = match rnd() % 3 {
                        0 => c[i] ^ (1 << (rnd() % 8)),
                        1 => 0xFF,
                        _ => 0x00,
                    };
                }
                let _ = try_parse(&c);
            }
        });
    }
}

#[test]
fn hostile_binary_structures_are_rejected() {
    // Node whose end offset points backwards (used to loop forever).
    let mut f = header(7400);
    let off = f.len();
    f.extend(node("Objects", 0, &[], Some(5), off));
    f.extend([0u8; 13]);
    assert!(!within(10, "backwards end", move || try_parse(&f)));

    // Array claiming 4 billion floats (used to try a 16 GB allocation).
    let mut props = vec![b'f'];
    props.extend_from_slice(&u32::MAX.to_le_bytes()); // count
    props.extend_from_slice(&0u32.to_le_bytes()); // encoding
    props.extend_from_slice(&4u32.to_le_bytes()); // compressed length
    props.extend_from_slice(&[0, 0, 0, 0]);
    let mut f = header(7400);
    let off = f.len();
    f.extend(node("Vertices", 1, &props, None, off));
    f.extend([0u8; 13]);
    assert!(!within(10, "huge array", move || try_parse(&f)));

    // zlib bomb: 64 MB of zeros compressed to ~64 KB, declared as 1000 floats.
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(&vec![0u8; 64 << 20]).unwrap();
    let bomb = z.finish().unwrap();
    let mut props = vec![b'f'];
    props.extend_from_slice(&1000u32.to_le_bytes());
    props.extend_from_slice(&1u32.to_le_bytes());
    props.extend_from_slice(&(bomb.len() as u32).to_le_bytes());
    props.extend_from_slice(&bomb);
    let mut f = header(7400);
    let off = f.len();
    f.extend(node("Vertices", 1, &props, None, off));
    f.extend([0u8; 13]);
    let (ok, _) = within(10, "zlib bomb", move || {
        let r = fbx::parse_binary(&f);
        let len = match &r {
            Ok((_, n)) => match n.first().and_then(|n| n.props.first()) {
                Some(fbx::Prop::ArrF(v)) => v.len(),
                _ => 0,
            },
            Err(_) => 0,
        };
        (r.is_ok(), len)
    });
    // Either rejected or decoded to exactly the declared 1000 elements.
    let _ = ok;

    // Unknown array encoding.
    let mut props = vec![b'f'];
    props.extend_from_slice(&1u32.to_le_bytes());
    props.extend_from_slice(&7u32.to_le_bytes());
    props.extend_from_slice(&4u32.to_le_bytes());
    props.extend_from_slice(&[0, 0, 0, 0]);
    let mut f = header(7400);
    let off = f.len();
    f.extend(node("Vertices", 1, &props, None, off));
    f.extend([0u8; 13]);
    assert!(!within(10, "bad encoding", move || try_parse(&f)));

    // Property count far larger than the node.
    let mut f = header(7400);
    let off = f.len();
    f.extend(node("Objects", u32::MAX, &[b'C', 1], None, off));
    f.extend([0u8; 13]);
    assert!(!within(10, "huge property count", move || try_parse(&f)));
}

#[test]
fn zlib_bomb_decodes_only_the_declared_size() {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(&vec![0u8; 64 << 20]).unwrap();
    let bomb = z.finish().unwrap();
    let mut props = vec![b'f'];
    props.extend_from_slice(&1000u32.to_le_bytes());
    props.extend_from_slice(&1u32.to_le_bytes());
    props.extend_from_slice(&(bomb.len() as u32).to_le_bytes());
    props.extend_from_slice(&bomb);
    let mut f = header(7400);
    let off = f.len();
    f.extend(node("Vertices", 1, &props, None, off));
    f.extend([0u8; 13]);
    let (_, nodes) = fbx::parse_binary(&f).expect("well-formed node");
    match &nodes[0].props[0] {
        fbx::Prop::ArrF(v) => assert_eq!(v.len(), 1000),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn deep_nesting_is_rejected_without_stack_overflow() {
    // Binary: 1000 nested nodes via the writer.
    let mut n = fbx::Node { name: "Leaf".into(), ..Default::default() };
    for _ in 0..1000 {
        n = fbx::Node { name: "N".into(), props: vec![], children: vec![n] };
    }
    let bytes = within(30, "write nested", move || fbx::write_binary(&[n], 7400));
    assert!(!within(30, "binary nesting", move || try_parse(&bytes)));

    // ASCII: 200 000 unclosed levels.
    let text = "A: {".repeat(200_000);
    assert!(!within(30, "ascii nesting", move || try_parse(text.as_bytes())));
}

#[test]
fn importer_reports_corrupt_files_as_errors() {
    let dir = std::env::temp_dir().join(format!("lc-fbx-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut d = std::fs::read(data("spider.fbx")).unwrap();
    let n = d.len();
    d.truncate(n / 3);
    let p = dir.join("broken.fbx");
    std::fs::write(&p, &d).unwrap();
    let settings = lumencraft::import::ImportSettings::default_for(&p);
    let r = lumencraft::import::load(&p, &settings);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(r.is_err(), "a truncated file must be reported, not imported");
}
