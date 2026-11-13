use lumencraft::fbx::{self, Prop};
use lumencraft::import::{self, ImportSettings, TextureResolver};
use std::path::{Path, PathBuf};

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
}

#[test]
fn fbx_textures_found_even_with_foreign_absolute_paths() {
    // spider.fbx points at D:\projects\assimp_int\...\SpiderTex.jpg (someone
    // else's machine). The files live in tests/data/textures/.
    let path = data("spider.fbx");
    let model = import::load(&path, &ImportSettings::default_for(&path)).unwrap();
    let textured: Vec<_> = model.materials.iter().filter(|m| m.texture.is_some()).map(|m| m.name.clone()).collect();
    println!("textured materials: {textured:?}; missing: {:?}", model.missing_textures);
    assert!(textured.len() >= 3, "expected the spider's textures to resolve");
    assert!(model.missing_textures.is_empty(), "missing: {:?}", model.missing_textures);
}

#[test]
fn extra_texture_folder_is_searched() {
    // Copy the model somewhere without its textures, then point at the folder.
    let dir = std::env::temp_dir().join(format!("lc-texdir-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let model_path = dir.join("spider.fbx");
    std::fs::copy(data("spider.fbx"), &model_path).unwrap();
    let mut s = ImportSettings::default_for(&model_path);
    let without = import::load(&model_path, &s).unwrap();
    assert!(!without.missing_textures.is_empty());
    s.texture_dirs.push(data("textures"));
    let with = import::load(&model_path, &s).unwrap();
    assert!(with.missing_textures.is_empty(), "{:?}", with.missing_textures);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fbm_folder_is_searched() {
    let dir = std::env::temp_dir().join(format!("lc-fbm-{}", std::process::id()));
    let fbm = dir.join("house.fbm");
    std::fs::create_dir_all(&fbm).unwrap();
    std::fs::copy(data("textures/SpiderTex.jpg"), fbm.join("SpiderTex.jpg")).unwrap();
    let mut r = TextureResolver::new(&dir.join("house.fbx"), &[]);
    assert!(r.load("C:\\Users\\someone\\Desktop\\SpiderTex.jpg").is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn embedded_texture_ascii() {
    // 3ds Max export with the PNG embedded as base64; the original file path
    // (C:\Users\HTC Vive 1\Desktop\uv_test.png) does not exist here.
    let path = data("embedded_texture_ascii.fbx");
    let model = import::load(&path, &ImportSettings::default_for(&path)).unwrap();
    let t = model.materials.iter().find_map(|m| m.texture.clone()).expect("embedded texture");
    assert_eq!((t.width, t.height), (512, 512));
    assert!(model.missing_textures.is_empty(), "{:?}", model.missing_textures);
}

#[test]
fn embedded_texture_binary_7500() {
    // Same file re-encoded as binary FBX 7.5 with raw embedded bytes (what
    // Revit/Max write when "Embed Media" is ticked).
    use base64::Engine;
    let text = std::fs::read_to_string(data("embedded_texture_ascii.fbx")).unwrap();
    let mut nodes = fbx::parse_ascii(&text).unwrap();
    fn fix(n: &mut fbx::Node) {
        if n.name == "Content" {
            for p in &mut n.props {
                if let Prop::S(s) = p {
                    if !s.is_empty() {
                        let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
                        *p = Prop::Raw(base64::engine::general_purpose::STANDARD.decode(clean).unwrap());
                    }
                }
            }
        }
        for c in &mut n.children {
            fix(c);
        }
    }
    for n in &mut nodes {
        fix(n);
    }
    let dir = std::env::temp_dir().join(format!("lc-emb-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("embedded.fbx");
    std::fs::write(&path, fbx::write_binary(&nodes, 7500)).unwrap();
    let model = import::load(&path, &ImportSettings::default_for(&path)).unwrap();
    assert!(model.materials.iter().any(|m| m.texture.is_some()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bump_maps_become_normal_maps() {
    use lumencraft::scene::TextureData;
    // A grey ramp: height rises to the right.
    let (w, h) = (16u32, 16u32);
    let mut px = Vec::new();
    for _y in 0..h {
        for x in 0..w {
            let v = (x * 16) as u8;
            px.extend_from_slice(&[v, v, v, 255]);
        }
    }
    let bump = TextureData::new(w, h, px);
    assert!(import::is_grayscale(&bump));
    let n = import::as_normal_map(bump);
    let mid = &n.rgba[((8 * w + 8) * 4) as usize..][..4];
    assert!(mid[0] < 128, "normal should tilt away from rising height: {mid:?}");
    assert!(mid[2] > 128);
}

#[test]
fn opacity_maps_cut_out() {
    use lumencraft::scene::TextureData;
    let color = TextureData::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]);
    let mask = TextureData::new(2, 1, vec![255, 255, 255, 255, 0, 0, 0, 255]);
    let merged = import::merge_opacity(Some(color), &mask, false);
    assert_eq!(merged.rgba[3], 255);
    assert_eq!(merged.rgba[7], 0);
    assert!(import::transparent_fraction(&merged) > 0.4);
}

#[test]
fn obj_mtl_map_options_and_formats() {
    let dir = std::env::temp_dir().join(format!("lc-objtex-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("maps")).unwrap();
    // Write the texture as TGA to exercise the extra formats.
    let img = image::open(data("textures/wal67ar_small.jpg")).unwrap();
    img.save(dir.join("maps/wall.tga")).unwrap();
    std::fs::write(dir.join("m.mtl"), "newmtl Brick\nKd 1 1 1\nmap_Kd -s 2 3 1 -o 0.5 0 0 C:\\Revit\\Export\\wall.tga\nmap_d leaves_mask.png\n").unwrap();
    std::fs::write(dir.join("m.obj"), "mtllib m.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nusemtl Brick\nf 1/1 2/2 3/3\n").unwrap();
    let path = dir.join("m.obj");
    let model = import::load(&path, &ImportSettings::default_for(&path)).unwrap();
    let m = model.materials.iter().find(|m| m.name == "Brick").unwrap();
    assert!(m.texture.is_some(), "TGA in a sub-folder with a foreign path should resolve");
    assert_eq!(m.params.uv_transform[0], 2.0);
    assert_eq!(m.params.uv_transform[1], 3.0);
    assert_eq!(m.params.uv_transform[2], 0.5);
    assert_eq!(model.missing_textures, vec!["leaves_mask.png".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}
