//! GPU smoke tests: every shader compiles and every quality level renders.
//! Skipped automatically on machines without any GPU/software adapter.

use lumencraft::project::World;
use lumencraft::renderer::{Quality, Renderer};

#[test]
fn shaders_compile_and_all_quality_levels_render() {
    let Ok((device, queue, _)) = lumencraft::headless::create_device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut world = World::new();
    lumencraft::demo::build_demo(&mut world);
    world.update_ground();
    let mut r = Renderer::new(device, queue);
    let t = r.create_targets(160, 90);
    for q in [4, 3, 2, 1, 4] {
        r.set_quality(Quality::preset(q));
        r.sync_scene(&world.scene);
        let fp = world.frame_params(160.0 / 90.0, 1.0);
        r.render(&world.scene, &fp, &t);
        let img = r.read_output(&t);
        // something other than black was drawn
        assert!(img.pixels().any(|p| p[0] > 20 || p[1] > 20 || p[2] > 20), "quality {q} rendered black");
    }
    // A static camera re-renders no shadow cascades after the first frame.
    let fp = world.frame_params(160.0 / 90.0, 1.0);
    r.render(&world.scene, &fp, &t);
    r.render(&world.scene, &fp, &t);
    assert_eq!(r.stats.shadow_updates, 0, "static view should reuse shadow maps");
}
