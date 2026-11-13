//! Library 3D models: loading, centring, materials, placement.
use lumencraft::objects::{self, ObjectKind, PlacedObject};

fn setup() -> bool {
    let lib = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("library");
    std::env::set_var("LUMENCRAFT_LIBRARY", &lib);
    objects::builtin_models_dir().is_some()
}

#[test]
fn builtin_model_loads_centred_with_pbr_maps() {
    if !setup() {
        eprintln!("built-in models not installed; skipping");
        return;
    }
    let a = objects::model_asset("builtin:Seating/sofa_02").expect("sofa loads");
    assert!(!a.parts.is_empty());
    let (mut lo, mut hi) = (glam::Vec3::splat(f32::MAX), glam::Vec3::splat(f32::MIN));
    for p in &a.parts {
        let (l, h) = p.bounds().unwrap();
        lo = lo.min(l);
        hi = hi.max(h);
    }
    assert!(lo.y.abs() < 0.01, "sits on the ground: {lo}");
    assert!(((lo.x + hi.x) * 0.5).abs() < 0.01 && ((lo.z + hi.z) * 0.5).abs() < 0.01, "centred");
    assert!(a.size.x > 1.0 && a.size.x < 4.0, "a sofa is a couple of metres wide: {}", a.size);
    assert!(a.materials.iter().any(|m| m.texture.is_some() && m.normal_texture.is_some() && m.roughness_texture.is_some()));
    assert!(a.materials.iter().all(|m| m.params.metallic < 0.5), "fabric isn't metal");
}

#[test]
fn placed_model_registers_materials_and_transforms() {
    if !setup() {
        return;
    }
    let mut scene = lumencraft::scene::Scene::default();
    let mut o = PlacedObject::new(ObjectKind::Model, [10.0, 0.0, 5.0], 0.0, 2.0, 1);
    o.model = Some("builtin:Tables/side_table_01".into());
    let objs = vec![o];
    objects::sync_objects(&mut scene, &objs);
    let (lo, hi) = scene.bounds_with_prefix("obj:0:").expect("meshes");
    assert!(((lo.x + hi.x) * 0.5 - 10.0).abs() < 0.1);
    for m in scene.meshes.values() {
        assert!(scene.imported_materials.contains_key(&m.data.material), "material {} registered", m.data.material);
    }
}

#[test]
fn missing_model_is_harmless() {
    let mut o = PlacedObject::new(ObjectKind::Model, [0.0; 3], 0.0, 1.0, 1);
    o.model = Some("C:/does/not/exist.fbx".into());
    assert!(objects::build(&o).is_empty());
}
