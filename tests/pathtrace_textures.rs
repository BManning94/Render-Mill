//! Texture resolution in the path tracer: a high-resolution material must
//! keep its fine detail even when a large model loads many other textures
//! (v0.10 resampled every texture to one global size: 256 px once more than
//! 80 maps were loaded).

use glam::Vec3;
use lumencraft::pathtrace::{PathTracer, PtOpts};
use lumencraft::project::World;
use lumencraft::renderer::Renderer;
use lumencraft::scene::{Material, MaterialParams, MeshData, Scene, TextureData};

fn noise_texture(size: u32, seed: u32) -> TextureData {
    // Independent per-texel noise (integer hash): any resolution loss
    // averages it toward flat grey.
    let data = (0..size * size)
        .flat_map(|i| {
            let mut h = i.wrapping_mul(0x9E37_79B1) ^ seed.wrapping_mul(0x85EB_CA77);
            h ^= h >> 15;
            h = h.wrapping_mul(0x2C1B_3C6D);
            h ^= h >> 12;
            let v = if h & 0x100 == 0 { 30 } else { 225 };
            [v, v, v, 255]
        })
        .collect();
    TextureData::new(size, size, data)
}

/// Pearson correlation of two images after averaging k x k blocks (removes
/// path-tracing sampling noise but keeps texel-scale structure).
fn correlation(a: &image::RgbaImage, b: &image::RgbaImage, k: u32) -> f32 {
    let (w, h) = a.dimensions();
    let block = |img: &image::RgbaImage, bx: u32, by: u32| -> f32 {
        let mut s = 0.0;
        for y in 0..k {
            for x in 0..k {
                s += img.get_pixel(bx * k + x, by * k + y)[0] as f32;
            }
        }
        s / (k * k) as f32
    };
    let (mut va, mut vb) = (Vec::new(), Vec::new());
    for by in h / k / 4..h / k * 3 / 4 {
        for bx in w / k / 4..w / k * 3 / 4 {
            va.push(block(a, bx, by));
            vb.push(block(b, bx, by));
        }
    }
    let ma = va.iter().sum::<f32>() / va.len() as f32;
    let mb = vb.iter().sum::<f32>() / vb.len() as f32;
    let (mut c, mut sa, mut sb) = (0.0, 0.0, 0.0);
    for (x, y) in va.iter().zip(&vb) {
        c += (x - ma) * (y - mb);
        sa += (x - ma).powi(2);
        sb += (y - mb).powi(2);
    }
    c / (sa * sb).sqrt().max(1e-6)
}

fn quad(name: &str, z: f32, x0: f32) -> MeshData {
    MeshData {
        name: name.into(),
        material: name.into(),
        positions: vec![Vec3::new(x0, 0.0, z), Vec3::new(x0 + 1.0, 0.0, z), Vec3::new(x0 + 1.0, 1.0, z), Vec3::new(x0, 1.0, z)],
        normals: vec![Vec3::NEG_Z; 4],
        uvs: vec![glam::Vec2::new(0.0, 1.0), glam::Vec2::new(1.0, 1.0), glam::Vec2::new(1.0, 0.0), glam::Vec2::new(0.0, 0.0)],
        indices: vec![0, 2, 1, 0, 3, 2],
    }
}

/// Standard deviation of the red channel over the central region.
fn contrast(img: &image::RgbaImage) -> f32 {
    let (w, h) = img.dimensions();
    let vals: Vec<f32> = (h / 4..h * 3 / 4).flat_map(|y| (w / 4..w * 3 / 4).map(move |x| (x, y))).map(|(x, y)| img.get_pixel(x, y)[0] as f32).collect();
    let mean = vals.iter().sum::<f32>() / vals.len() as f32;
    (vals.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / vals.len() as f32).sqrt()
}

#[test]
fn high_res_texture_keeps_detail_with_many_materials() {
    let Ok((device, queue, _)) = lumencraft::headless::create_device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut world = World::new();
    world.scene = Scene::default();
    world.env.fog = 0.0;
    world.env.haze = 0.0;
    // The material under test: 2048 px of per-texel noise; we look at a
    // 1/16 x 1/16 corner of it, i.e. 128 texels across the image.
    let mut p = MaterialParams { uv_scale: 1.0 / 16.0, roughness: 1.0, variation: 0.0, ..Default::default() };
    p.base_color = [1.0, 1.0, 1.0, 1.0];
    world.scene.imported_materials.insert(
        "detail".into(),
        Material { name: "detail".into(), params: p, texture: Some(noise_texture(2048, 7)), normal_texture: None, roughness_texture: None },
    );
    world.scene.upsert_mesh("file:detail".into(), quad("detail", 0.0, 0.0));
    // 120 other textured materials far off to the side (a big imported model).
    for i in 0..120 {
        let name = format!("other{i}");
        world.scene.imported_materials.insert(
            name.clone(),
            Material {
                name: name.clone(),
                params: MaterialParams::default(),
                texture: Some(noise_texture(512, (i + 100) * 2 + 1)),
                normal_texture: None,
                roughness_texture: None,
            },
        );
        world.scene.upsert_mesh(format!("file:{name}"), quad(&name, 50.0, 10.0 + i as f32 * 2.0));
    }
    world.scene.material_rev += 1;
    world.camera.position = [1.0 / 32.0, 1.0 - 1.0 / 32.0, -0.06];
    world.camera.look_at(Vec3::new(1.0 / 32.0, 1.0 - 1.0 / 32.0, 0.0));
    world.camera.fov = 55.0;
    let (w, h) = (256u32, 256u32);
    let mut fp = world.frame_params(1.0, 1.0);
    fp.lens = [0.0; 4]; // no depth of field, chromatic aberration or grain
    fp.bloom = 0.0;
    fp.vignette = 0.0;
    fp.sharpen = 0.0;

    let mut r = Renderer::new(device, queue);
    r.sync_scene(&world.scene);
    let t = r.create_targets(w, h);
    r.render(&world.scene, &fp, &t);
    let raster = r.read_output(&t);

    let mut pt = PathTracer::new(&r.device);
    let opts = PtOpts { spp: 16, bounces: 1, denoise: false };
    let (traced, _) = lumencraft::pathtrace::render_still(&mut r, &mut pt, &world.scene, &fp, w, h, opts, |_| true).unwrap();
    let m = pt.memory();
    let (cr, ct) = (contrast(&raster), contrast(&traced));
    let corr = correlation(&raster, &traced, 4);
    eprintln!(
        "contrast raster {cr:.1}, path traced {ct:.1}, correlation {corr:.2}; texture layers {:?}/{:?}, demoted {}",
        m.color_layers, m.data_layers, m.demoted
    );
    if let Ok(dir) = std::env::var("LC_TEST_OUT") {
        raster.save(format!("{dir}/tex_raster.png")).unwrap();
        traced.save(format!("{dir}/tex_traced.png")).unwrap();
    }
    assert_eq!(m.color_layers[4], 1, "the 2048 px map stays in the 2K class");
    assert_eq!(m.color_layers[2], 120, "the 512 px maps stay at 512");
    assert!(m.reduction_note().is_none());
    // Per-texel noise seen at ~2 px per texel: blurred to a 256 px layer it
    // would be ~8x softer and nearly flat.
    assert!(cr > 20.0, "raster shows the texture ({cr})");
    // Same texel pattern as the rasterizer (which samples the full-res map).
    assert!(corr > 0.6, "path tracer lost texture detail: correlation {corr} with the raster render");
}

fn many_textures_world(n: u32, size: u32) -> World {
    let mut world = World::new();
    world.scene = Scene::default();
    for i in 0..n {
        let name = format!("m{i}");
        world.scene.imported_materials.insert(
            name.clone(),
            Material {
                name: name.clone(),
                params: MaterialParams::default(),
                texture: Some(noise_texture(size, i * 2 + 1)),
                normal_texture: Some(noise_texture(size, i * 2 + 2)),
                roughness_texture: None,
            },
        );
        world.scene.upsert_mesh(format!("file:{name}"), quad(&name, 2.0, i as f32 * 1.2 - n as f32 * 0.6));
    }
    world.scene.material_rev += 1;
    world.camera.position = [0.0, 0.5, -3.0];
    world.camera.look_at(Vec3::new(0.0, 0.5, 2.0));
    world
}

/// v0.12: the texture budget is a hard limit in both renderers. Maps go
/// below 512 px when needed and the reduction is reported.
#[test]
fn texture_budget_is_enforced_and_reported() {
    let Ok((device, queue, _)) = lumencraft::headless::create_device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    // 60 materials x (colour + normal) at 1024 px = ~680 MB of maps with mips.
    let world = many_textures_world(60, 1024);
    let budget = 24u64 << 20;
    let mut r = Renderer::new(device, queue);
    r.set_texture_budget(budget);
    r.sync_scene(&world.scene);
    let st = r.texture_stats();
    eprintln!("viewport {st:?}");
    assert!(st.bytes <= budget, "viewport textures {} > budget", st.bytes);
    assert!(st.max_side.unwrap() < 512, "maps must shrink below 512 px");
    assert_eq!(st.reduced, 120);
    assert!(st.reduction_note().is_some());

    let mut pt = PathTracer::new(&r.device);
    pt.set_texture_budget(budget);
    let fp = world.frame_params(1.0, 0.0);
    let opts = PtOpts { spp: 2, bounces: 1, denoise: false };
    let out = lumencraft::pathtrace::render_still(&mut r, &mut pt, &world.scene, &fp, 64, 64, opts, |_| true);
    assert!(out.is_some());
    let m = pt.memory();
    eprintln!("path tracer {m:?}");
    assert!(m.textures <= budget, "path-tracer textures {} > budget", m.textures);
    assert!(m.smallest_reduced.unwrap() < 512);
    assert_eq!(m.dropped, 0);
    assert!(m.reduction_note().unwrap().contains("scaled down"));

    // Raising the budget restores full resolution.
    r.set_texture_budget(4 << 30);
    r.sync_scene(&world.scene);
    assert!(r.texture_stats().reduction_note().is_none());
}

/// A size the card can't allocate is refused before rendering starts
/// (v0.11 panicked inside wgpu).
#[test]
fn oversize_render_is_refused_without_panicking() {
    let Ok((device, queue, _)) = lumencraft::headless::create_device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let world = many_textures_world(1, 64);
    let mut r = Renderer::new(device, queue);
    r.sync_scene(&world.scene);
    let l = r.device.limits();
    let (w, h) = (l.max_texture_dimension_2d + 64, 64);
    let e = lumencraft::pathtrace::check_resolution(&l, w, h, true).unwrap_err();
    assert!(lumencraft::pathtrace::check_resolution(&l, e.max_supported.0, e.max_supported.1, true).is_ok());
    let mut pt = PathTracer::new(&r.device);
    let fp = world.frame_params(w as f32 / h as f32, 0.0);
    let opts = PtOpts { spp: 1, bounces: 1, denoise: false };
    assert!(lumencraft::pathtrace::render_still(&mut r, &mut pt, &world.scene, &fp, w, h, opts, |_| true).is_none());
}

/// v0.12: in the live preview a large image's sample is split over several
/// short passes (so the GPU never stalls the UI for a whole 4K pass). The
/// result must be the same image with the same sample count.
#[test]
fn sliced_passes_give_the_same_image() {
    let Ok((device, queue, _)) = lumencraft::headless::create_device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let world = many_textures_world(4, 64);
    let mut r = Renderer::new(device, queue);
    r.sync_scene(&world.scene);
    let mut pt = PathTracer::new(&r.device);
    let (w, h) = (400u32, 300u32);
    let fp = world.frame_params(w as f32 / h as f32, 0.0);
    let opts = PtOpts { spp: 32, bounces: 2, denoise: false };
    let mut run = |target_ms: f32| {
        let t = r.create_targets(w, h);
        let mut job = pt.begin(&mut r, &world.scene, &fp, &t, opts);
        job.target_ms = target_ms;
        let mut max_slices = 1;
        while !job.finished() {
            pt.step(&r, &mut job, std::time::Duration::from_millis(50));
            max_slices = max_slices.max(job.passes_per_sample());
        }
        assert_eq!(job.done_spp, 32);
        let (c, ..) = pt.read_layers(&r, &mut job);
        pt.recycle(job);
        let mean = c.iter().map(|p| (p[0] + p[1] + p[2]) as f64).sum::<f64>() / c.len() as f64;
        (mean, max_slices)
    };
    let (m_full, _) = run(1000.0);
    let (m_sliced, slices) = run(0.001);
    eprintln!("whole passes {m_full:.4}, sliced ({slices} passes per sample) {m_sliced:.4}");
    assert!(slices > 1, "tiny target must split passes");
    assert!((m_full - m_sliced).abs() / m_full.max(1e-6) < 0.03, "{m_full} vs {m_sliced}");
}
