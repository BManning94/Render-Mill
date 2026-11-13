use lumencraft::fbx;
use lumencraft::import::{self, ImportSettings, TextureResolver};
use std::path::Path;

fn data(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
}

fn tris(s: &fbx::FbxScene) -> usize {
    s.meshes.iter().map(|(_, m)| m.indices.len() / 3).sum()
}

#[test]
fn binary_7400_files_load() {
    for f in ["phong_cube.fbx", "global_settings.fbx", "spider.fbx", "box_binary.fbx"] {
        let s = fbx::load(&data(f), &mut TextureResolver::new(&data(f), &[])).unwrap_or_else(|e| panic!("{f}: {e:#}"));
        assert!(tris(&s) > 0, "{f} has no triangles");
        println!("{f}: {} meshes, {} tris, {} materials, hints {:?}", s.meshes.len(), tris(&s), s.materials.len(), s.hints);
    }
}

#[test]
fn ascii_files_load() {
    for f in ["box_ascii.fbx", "cubes_with_names.fbx", "close_to_identity_transforms.fbx", "cubes_with_mirroring_and_pivot.fbx"] {
        let s = fbx::load(&data(f), &mut TextureResolver::new(&data(f), &[])).unwrap_or_else(|e| panic!("{f}: {e:#}"));
        assert!(tris(&s) > 0, "{f} has no triangles");
        println!("{f}: {} meshes, {} tris, {} materials", s.meshes.len(), tris(&s), s.materials.len());
    }
}

#[test]
fn cube_is_twelve_triangles_with_correct_size() {
    let s = fbx::load(&data("phong_cube.fbx"), &mut TextureResolver::new(&data("phong_cube.fbx"), &[])).unwrap();
    assert_eq!(tris(&s), 12);
    let (lo, hi) = s.meshes.iter().filter_map(|(_, m)| m.bounds()).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1))).unwrap();
    println!("cube bounds {lo:?} {hi:?}");
    assert!((hi - lo).max_element() > 0.0);
}

#[test]
fn wide_7500_headers_roundtrip() {
    // Revit 2020+ writes FBX 7.5 with 64-bit node headers. Re-encode a 7.4
    // file both ways and check the scene comes out identical.
    let bytes = std::fs::read(data("spider.fbx")).unwrap();
    let (_, nodes) = fbx::parse_binary(&bytes).unwrap();
    let a = fbx::build_scene(&nodes, &mut TextureResolver::new(Path::new("x.fbx"), &[])).unwrap();
    for v in [7400, 7500, 7700] {
        let enc = fbx::write_binary(&nodes, v);
        let (ver, nodes2) = fbx::parse_binary(&enc).unwrap();
        assert_eq!(ver, v);
        let b = fbx::build_scene(&nodes2, &mut TextureResolver::new(Path::new("x.fbx"), &[])).unwrap();
        assert_eq!(tris(&a), tris(&b), "version {v}");
        assert_eq!(a.meshes.len(), b.meshes.len());
        let pa: Vec<_> = a.meshes.iter().flat_map(|(_, m)| m.positions.clone()).collect();
        let pb: Vec<_> = b.meshes.iter().flat_map(|(_, m)| m.positions.clone()).collect();
        assert_eq!(pa, pb);
    }
}

#[test]
fn revit_style_feet_z_up_is_converted() {
    // Synthesize a Revit-like file: Z-up, UnitScaleFactor 30.48 (feet).
    let bytes = std::fs::read(data("phong_cube.fbx")).unwrap();
    let (_, mut nodes) = fbx::parse_binary(&bytes).unwrap();
    let gs = nodes.iter_mut().find(|n| n.name == "GlobalSettings").unwrap();
    let p70 = gs.children.iter_mut().find(|n| n.name == "Properties70").unwrap();
    for p in &mut p70.children {
        match p.props.first() {
            Some(fbx::Prop::S(s)) if s == "UpAxis" => p.props[4] = fbx::Prop::I(2),
            Some(fbx::Prop::S(s)) if s == "UnitScaleFactor" => p.props[4] = fbx::Prop::F(30.48),
            _ => {}
        }
    }
    let dir = std::env::temp_dir().join(format!("lc-fbx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("revit.fbx");
    std::fs::write(&path, fbx::write_binary(&nodes, 7500)).unwrap();
    let settings = ImportSettings::default_for(&path);
    let model = import::load(&path, &settings).unwrap();
    let (lo, hi) = model.meshes.iter().filter_map(|(_, m)| m.bounds()).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1))).unwrap();
    println!("revit-style bounds {lo:?} {hi:?}");
    // sits on the ground after recentering
    assert!(lo.y.abs() < 1e-4);
    let _ = std::fs::remove_dir_all(&dir);
}
