//! Real-world (box) texture mapping: a texture applied to a model whose UVs
//! are stretched 0..1 per face (common in Revit exports) must still tile at
//! its real-world size, in both the rasterizer and the path tracer.

use lumencraft::project::World;
use lumencraft::renderer::Renderer;
use std::path::PathBuf;

fn write_box(dir: &std::path::Path) -> PathBuf {
    let (w, d, h) = (10.0f32, 6.0f32, 4.0f32);
    let v = [(0., 0., 0.), (w, 0., 0.), (w, h, 0.), (0., h, 0.), (0., 0., d), (w, 0., d), (w, h, d), (0., h, d)];
    let faces = [(0, 1, 2, 3), (5, 4, 7, 6), (4, 0, 3, 7), (1, 5, 6, 2), (3, 2, 6, 7)];
    let mut o = String::from("mtllib b.mtl\nusemtl wall\n");
    for p in v {
        o += &format!("v {} {} {}\n", p.0, p.1, p.2);
    }
    o += "vt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\n";
    for f in faces {
        o += &format!("f {}/1 {}/2 {}/3\nf {}/1 {}/3 {}/4\n", f.0 + 1, f.1 + 1, f.2 + 1, f.0 + 1, f.2 + 1, f.3 + 1);
    }
    std::fs::write(dir.join("b.obj"), o).unwrap();
    std::fs::write(dir.join("b.mtl"), "newmtl wall\nKd 0.8 0.8 0.8\n").unwrap();
    // Vertical stripes: black/white halves of one tile.
    let img = image::RgbaImage::from_fn(64, 64, |x, _| if x < 32 { image::Rgba([20, 20, 20, 255]) } else { image::Rgba([235, 235, 235, 255]) });
    img.save(dir.join("stripes.png")).unwrap();
    dir.join("b.obj")
}

/// Number of dark/light transitions along the middle row of the image,
/// thresholded relative to that row's range (lit and shaded walls differ).
fn transitions(img: &image::RgbaImage) -> usize {
    let y = img.height() / 2;
    // Central 80% of the row: only the wall (the camera is close enough).
    let (x0, x1) = (img.width() / 10, img.width() * 9 / 10);
    let row: Vec<i32> = (x0..x1).map(|x| img.get_pixel(x, y)[0] as i32).collect();
    // Smooth path-tracing noise a little.
    let sm: Vec<i32> = (0..row.len())
        .map(|i| {
            let a = i.saturating_sub(2);
            let b = (i + 3).min(row.len());
            row[a..b].iter().sum::<i32>() / (b - a) as i32
        })
        .collect();
    let (lo, hi) = (*sm.iter().min().unwrap(), *sm.iter().max().unwrap());
    let (t_lo, t_hi) = (lo + (hi - lo) / 3, hi - (hi - lo) / 3);
    let mut last = None;
    let mut n = 0;
    for &l in &sm {
        let s = if l < t_lo {
            Some(false)
        } else if l > t_hi {
            Some(true)
        } else {
            None
        };
        if let (Some(a), Some(b)) = (last, s) {
            if a != b {
                n += 1;
            }
        }
        if s.is_some() {
            last = s;
        }
    }
    n
}

#[test]
fn box_mapping_tiles_at_real_world_size() {
    let Ok((device, queue, _)) = lumencraft::headless::create_device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("lc-mapping-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let obj = write_box(&dir);
    let mut world = World::new();
    world.load_model_now(&obj).unwrap();
    world.env.ground_enabled = false;
    world.env.fog = 0.0;
    world.update_ground();
    let mat = world.scene.meshes.values().find(|m| m.data.material.contains("wall")).expect("wall mesh").data.material.clone();
    let set = lumencraft::texlib::TextureSet {
        name: "stripes".into(),
        color: Some(dir.join("stripes.png").to_string_lossy().to_string()),
        normal: None,
        roughness: None,
        size_m: 1.0,
    };
    // Look straight at the long wall, filling the frame (the importer may
    // rescale/recentre the model, so use its real bounds).
    let (lo, hi) = world.scene.model_bounds().expect("bounds");
    let size = hi - lo;
    let (long_x, len) = if size.x >= size.z { (true, size.x) } else { (false, size.z) };
    let c = (lo + hi) * 0.5;
    let dist = len * 0.35;
    let eye = if long_x { glam::Vec3::new(c.x, c.y, lo.z - dist) } else { glam::Vec3::new(lo.x - dist, c.y, c.z) };
    world.camera.position = eye.to_array();
    world.camera.look_at(c);
    world.camera.fov = 70.0;
    let mut r = Renderer::new(device, queue);
    let t = r.create_targets(320, 180);
    let out = std::env::var("LC_TEST_OUT").ok();
    let mut counts = Vec::new();
    for boxed in [false, true] {
        let old = world.scene.effective(&mat);
        let mut p = lumencraft::texlib::apply(&set, &old);
        p.box_mapping = boxed;
        world.scene.overrides.insert(mat.clone(), p);
        world.scene.material_rev += 1;
        world.load_user_textures();
        r.sync_scene(&world.scene);
        let fp = world.frame_params(320.0 / 180.0, 1.0);
        r.render(&world.scene, &fp, &t);
        r.render(&world.scene, &fp, &t);
        let img = r.read_output(&t);
        if let Some(o) = &out {
            img.save(format!("{o}/mapping_{boxed}.png")).unwrap();
        }
        counts.push(transitions(&img));
        if boxed {
            // The path tracer places the texture the same way.
            let mut pt = lumencraft::pathtrace::PathTracer::new(&r.device);
            let opts = lumencraft::pathtrace::PtOpts { spp: 2, bounces: 1, denoise: false };
            let (img, _) = lumencraft::pathtrace::render_still(&mut r, &mut pt, &world.scene, &fp, 320, 180, opts, |_| true).unwrap();
            if let Some(o) = &out {
                img.save(format!("{o}/mapping_pt.png")).unwrap();
            }
            counts.push(transitions(&img));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    // One 1 m tile = one dark and one light stripe = 2 transitions per metre,
    // over the central 80% of the visible width.
    let visible = 2.0 * dist * (35f32.to_radians().tan()) * (320.0 / 180.0);
    let expect = (visible * 0.8 * 2.0).round() as usize;
    eprintln!("wall {len:.1} m; stripe transitions: uv {}, box {}, path traced box {} (expect ~{expect})", counts[0], counts[1], counts[2]);
    // Model UVs: one stretched tile across the wall.
    assert!(counts[0] <= 3, "uv mapping {counts:?}");
    // Real-world: one tile per metre.
    assert!(counts[1] + 3 >= expect && counts[1] <= expect + 3, "box mapping {counts:?}");
    assert!(counts[2] + 3 >= expect && counts[2] <= expect + 3, "path traced box mapping {counts:?}");
}
