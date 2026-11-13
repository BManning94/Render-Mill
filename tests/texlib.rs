use lumencraft::project::{Project, World};
use lumencraft::renderer::Renderer;
use lumencraft::texlib;
use std::path::Path;

fn pbr_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/pbr")
}

#[test]
fn import_apply_render_and_save_texture_sets() {
    let files = texlib::scan_folder(&pbr_dir());
    let sets = texlib::group(&files);
    assert_eq!(sets.len(), 2, "{sets:#?}");
    for s in &sets {
        assert!(s.color.is_some() && s.normal.is_some() && s.roughness.is_some(), "{s:?}");
    }

    let mut world = World::new();
    lumencraft::demo::build_demo(&mut world);
    world.texture_library = sets.clone();
    let brick = sets.iter().find(|s| s.name.to_lowercase().contains("brick")).unwrap();
    let wood = sets.iter().find(|s| s.name.to_lowercase().contains("wood")).unwrap();
    for (mat, set) in [("Plaster - White", brick), ("Paving - Stone", wood)] {
        assert!(world.load_texture_set(set));
        let old = world.scene.effective(mat);
        world.scene.overrides.insert(mat.into(), texlib::apply(set, &old));
    }
    world.update_ground();

    // Saved and re-opened projects keep the library and the applied maps.
    let dir = std::env::temp_dir().join(format!("lc-texlib-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("p.lcproj");
    world.to_project().save(&path).unwrap();
    let reopened = World::from_project(Project::load(&path).unwrap());
    assert_eq!(reopened.texture_library.len(), 2);
    assert!(reopened.scene.effective("Plaster - White").roughness_map.is_some());
    assert!(reopened.scene.user_textures.len() >= 6, "maps reload with the project");

    let Ok((device, queue, _)) = lumencraft::headless::create_device() else { return };
    let mut r = Renderer::new(device, queue);
    let t = r.create_targets(960, 540);
    world.camera.position = [10.0, 1.7, 20.0];
    world.camera.look_at(glam::Vec3::new(-2.0, 2.5, 5.0));
    r.sync_scene(&world.scene);
    let fp = world.frame_params(960.0 / 540.0, 1.0);
    r.render(&world.scene, &fp, &t);
    let img = r.read_output(&t);
    img.save(std::env::temp_dir().join("lc_texlib_render.png")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
