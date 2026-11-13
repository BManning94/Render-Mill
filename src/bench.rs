//! Reproducible path-tracing benchmark scenes (`lumencraft --bench-pt`).
//!
//! * `small-exterior` - the demo house.
//! * `large-exterior` - the demo house with 400 plants, 3D grass and a
//!   larger site.
//! * `interior` - a living room lit only by sun and sky through a large
//!   window (glass, indirect light).
//! * `large-model` - an imported-model stand-in: 144 building blocks with
//!   150 materials (110 library 1K maps, 20 at 2K, 4 at 4K, 16 more at
//!   1K), glazing and mullions.
//!
//! Everything is generated deterministically, so numbers are comparable
//! between versions and machines.

use crate::objects::{ObjectKind, PlacedObject};
use crate::pathtrace::{PathTracer, PtOpts};
use crate::project::World;
use crate::renderer::Renderer;
use crate::scene::{presets, Material, MaterialParams, MeshData, TextureData};
use anyhow::Result;
use glam::Vec3;
use std::time::Instant;

pub const SCENES: [&str; 4] = ["small-exterior", "large-exterior", "interior", "large-model"];

fn preset(name: &str) -> MaterialParams {
    presets().into_iter().find(|(n, _)| *n == name).map(|(_, p)| p).unwrap_or_default()
}

fn mat(world: &mut World, name: &str, params: MaterialParams, texture: Option<TextureData>) {
    world.scene.set_material(Material { name: name.into(), params, texture, normal_texture: None, roughness_texture: None });
}

fn mesh(name: &str) -> MeshData {
    MeshData { name: name.into(), material: name.into(), ..Default::default() }
}

/// Deterministic noise texture (stands in for high-res photo materials).
fn noise_texture(size: u32, seed: u32) -> TextureData {
    let data = (0..size * size)
        .flat_map(|i| {
            let mut h = i.wrapping_mul(0x9E37_79B1) ^ seed.wrapping_mul(0x85EB_CA77);
            h ^= h >> 15;
            h = h.wrapping_mul(0x2C1B_3C6D);
            h ^= h >> 12;
            let v = 90 + (h & 0x7f) as u8;
            [v, (v as u32 * 9 / 10) as u8, (v as u32 * 8 / 10) as u8, 255]
        })
        .collect();
    TextureData::new(size, size, data)
}

/// Library material textures (CC0 set), if installed.
fn library_textures(limit: usize) -> Vec<TextureData> {
    let Some(dir) = crate::library::builtin_dir() else { return Vec::new() };
    let mut out = Vec::new();
    for m in crate::library::scan_builtin(&dir) {
        if out.len() >= limit {
            break;
        }
        if let Some(c) = &m.set.color {
            if let Some(t) = crate::import::load_texture_file(std::path::Path::new(c)) {
                out.push(t);
            }
        }
    }
    out
}

pub fn build(name: &str) -> Result<World> {
    let mut w = World::new();
    match name {
        "small-exterior" => crate::demo::build_demo(&mut w),
        "large-exterior" => {
            crate::demo::build_demo(&mut w);
            let kinds = [ObjectKind::Tree, ObjectKind::Conifer, ObjectKind::Shrub, ObjectKind::Shrub, ObjectKind::Boulder];
            let mut seed = 1000u32;
            for i in 0..400 {
                let a = i as f32 * 2.399_963; // golden angle spiral
                let r = 22.0 + (i as f32).sqrt() * 4.0;
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let k = kinds[(seed >> 16) as usize % kinds.len()];
                w.objects.push(PlacedObject::new(k, [a.cos() * r, 0.0, a.sin() * r], a, 0.8 + (seed & 0xff) as f32 / 640.0, seed));
            }
            w.env.grass = true;
            w.rebuild_objects();
            w.camera.position = [38.0, 9.0, 46.0];
            w.camera.look_at(Vec3::new(0.0, 2.0, 0.0));
        }
        "interior" => {
            let v = Vec3::new;
            let mut wood = preset("Oak wood");
            wood.roughness = 0.45;
            mat(&mut w, "Floor - Oak", wood, None);
            mat(&mut w, "Walls - Plaster", preset("White plaster"), None);
            mat(&mut w, "Glass - Window", preset("Clear glass"), None);
            mat(&mut w, "Frames - Metal", preset("Black metal"), None);
            let mut floor = mesh("Floor - Oak");
            crate::demo::add_box(&mut floor, v(-4.0, -0.2, -3.0), v(4.0, 0.0, 3.0));
            let mut walls = mesh("Walls - Plaster");
            crate::demo::add_box(&mut walls, v(-4.2, 0.0, -3.2), v(-4.0, 3.0, 3.2)); // west
            crate::demo::add_box(&mut walls, v(4.0, 0.0, -3.2), v(4.2, 3.0, 3.2)); // east
            crate::demo::add_box(&mut walls, v(-4.2, 0.0, -3.2), v(4.2, 3.0, -3.0)); // north
            crate::demo::add_box(&mut walls, v(-4.2, 3.0, -3.2), v(4.2, 3.2, 3.2)); // ceiling
                                                                                    // south wall with a 5 x 2.2 m window
            crate::demo::add_box(&mut walls, v(-4.2, 0.0, 3.0), v(-2.5, 3.0, 3.2));
            crate::demo::add_box(&mut walls, v(2.5, 0.0, 3.0), v(4.2, 3.0, 3.2));
            crate::demo::add_box(&mut walls, v(-2.5, 0.0, 3.0), v(2.5, 0.5, 3.2));
            crate::demo::add_box(&mut walls, v(-2.5, 2.7, 3.0), v(2.5, 3.0, 3.2));
            let mut glass = mesh("Glass - Window");
            crate::demo::add_box(&mut glass, v(-2.5, 0.5, 3.08), v(2.5, 2.7, 3.12));
            let mut frames = mesh("Frames - Metal");
            for x in [-2.5, -0.85, 0.8, 2.45] {
                crate::demo::add_box(&mut frames, v(x, 0.5, 3.05), v(x + 0.05, 2.7, 3.15));
            }
            for (id, m) in [("floor", floor), ("walls", walls), ("glass", glass), ("frames", frames)] {
                w.scene.upsert_mesh(format!("file:{id}"), m);
            }
            let o = |k, x: f32, z: f32, r: f32| PlacedObject::new(k, [x, 0.0, z], r, 1.0, 7);
            w.objects = vec![
                o(ObjectKind::Sofa, -1.5, -2.2, 0.0),
                o(ObjectKind::Table, -1.2, -0.6, 0.0),
                o(ObjectKind::ArmChair, 1.8, -0.8, -1.2),
                o(ObjectKind::Planter, 3.4, 2.4, 0.0),
                o(ObjectKind::Chair, 0.4, 0.8, 2.6),
            ];
            w.rebuild_objects();
            w.env.ground_enabled = true;
            w.env.sun_heading = 200.0;
            w.env.sun_height = 32.0;
            w.camera.position = [3.4, 1.55, -2.6];
            w.camera.look_at(Vec3::new(-1.0, 1.0, 2.2));
            w.camera.fov = 70.0;
        }
        "large-model" => {
            let lib = library_textures(110);
            let v = Vec3::new;
            let mut meshes: Vec<MeshData> = Vec::new();
            for m in 0..150u32 {
                let name = format!("Revit Material {m:03}");
                // 110 library maps (1K), 20 at 2K, 4 at 4K, 16 more at 1K.
                let tex = if m < 110 && (m as usize) < lib.len() {
                    Some(lib[m as usize].clone())
                } else if (110..130).contains(&m) {
                    Some(noise_texture(2048, m))
                } else if (130..134).contains(&m) {
                    Some(noise_texture(4096, m))
                } else {
                    Some(noise_texture(1024, m))
                };
                let mut p = MaterialParams { box_mapping: true, uv_scale: 1.0 / 2.0, roughness: 0.6, ..Default::default() };
                p.base_color = [1.0, 1.0, 1.0, 1.0];
                if m % 25 == 7 {
                    p = preset("Clear glass");
                }
                mat(&mut w, &name, p, tex.filter(|_| m % 25 != 7));
                meshes.push(mesh(&name));
            }
            // 12 x 12 blocks: walls, floor plates, mullions and glazing.
            for bx in 0..12 {
                for bz in 0..12 {
                    let i = bx * 12 + bz;
                    let base = v(bx as f32 * 14.0 - 84.0, 0.0, bz as f32 * 14.0 - 84.0);
                    let floors = 1 + (i * 7 % 4);
                    for f in 0..floors {
                        let y = f as f32 * 3.2;
                        let m = &mut meshes[(i * 3 + f) % 150];
                        crate::demo::add_box(m, base + v(0.0, y, 0.0), base + v(10.0, y + 0.3, 8.0));
                        let m = &mut meshes[(i * 5 + f + 1) % 150];
                        crate::demo::add_box(m, base + v(0.0, y + 0.3, 0.0), base + v(0.25, y + 3.2, 8.0));
                        crate::demo::add_box(m, base + v(9.75, y + 0.3, 0.0), base + v(10.0, y + 3.2, 8.0));
                        crate::demo::add_box(m, base + v(0.0, y + 0.3, 0.0), base + v(10.0, y + 3.2, 0.25));
                        let g = &mut meshes[7 + 25 * ((i + f) % 6)];
                        crate::demo::add_box(g, base + v(0.25, y + 0.9, 7.9), base + v(9.75, y + 3.0, 7.95));
                        let mu = &mut meshes[(i * 11 + f) % 150];
                        for k in 0..20 {
                            let x = 0.25 + k as f32 * 0.475;
                            crate::demo::add_box(mu, base + v(x, y + 0.3, 7.85), base + v(x + 0.05, y + 3.2, 8.0));
                        }
                    }
                }
            }
            for (k, m) in meshes.into_iter().enumerate() {
                if !m.positions.is_empty() {
                    w.scene.upsert_mesh(format!("file:part{k}"), m);
                }
            }
            w.camera.position = [60.0, 30.0, 95.0];
            w.camera.look_at(Vec3::new(-10.0, 2.0, -10.0));
        }
        other => anyhow::bail!("unknown benchmark scene '{other}' (use one of {SCENES:?})"),
    }
    w.update_ground();
    w.load_user_textures();
    Ok(w)
}

pub struct BenchResult {
    pub text: String,
}

/// Path-trace `scene` and report timing, throughput and GPU memory.
pub fn run(scene: &str, width: u32, height: u32, spp: u32, out: Option<&std::path::Path>) -> Result<BenchResult> {
    let t_build = Instant::now();
    let mut world = build(scene)?;
    let build_s = t_build.elapsed().as_secs_f32();
    let (device, queue, gpu) = crate::headless::create_device()?;
    let timestamps = device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
    let mut r = Renderer::new(device, queue);
    r.sync_scene(&world.scene);
    let fp = world.frame_params(width as f32 / height as f32, 12.0);
    let targets = r.create_targets(width, height);
    let mut pt = PathTracer::new(&r.device);
    // A/B switch for window (portal) sampling (off by default).
    if std::env::var_os("LC_BENCH_WINDOWS").is_some() {
        pt.window_sampling = true;
    }
    let t_sync = Instant::now();
    pt.sync(&r.device.clone(), &r.queue.clone(), &world.scene);
    let sync_s = t_sync.elapsed().as_secs_f32();
    // Second sync with nothing changed: should be ~free (cache check only).
    let t_resync = Instant::now();
    pt.sync(&r.device.clone(), &r.queue.clone(), &world.scene);
    let resync_ms = t_resync.elapsed().as_secs_f32() * 1000.0;
    // Interactive edits: change one material, then move one mesh, re-syncing
    // the path tracer after each (what restarting the live preview costs).
    let first_mat = world.scene.meshes.values().next().map(|m| m.data.material.clone()).unwrap_or_default();
    let mut p = world.scene.effective(&first_mat);
    p.roughness = (p.roughness + 0.1).min(1.0);
    world.scene.overrides.insert(first_mat, p);
    world.scene.material_rev += 1;
    let t_edit = Instant::now();
    pt.sync(&r.device.clone(), &r.queue.clone(), &world.scene);
    let material_edit_ms = t_edit.elapsed().as_secs_f32() * 1000.0;
    let (id, mut moved) = world.scene.meshes.iter().map(|(k, m)| (k.clone(), (*m.data).clone())).next().unwrap();
    for q in &mut moved.positions {
        q.x += 0.5;
    }
    world.scene.upsert_mesh(id, moved);
    let t_move = Instant::now();
    pt.sync(&r.device.clone(), &r.queue.clone(), &world.scene);
    let move_ms = t_move.elapsed().as_secs_f32() * 1000.0;
    eprintln!("EDITS material_edit_ms={material_edit_ms:.1} move_mesh_ms={move_ms:.1}");
    let opts = PtOpts { spp, bounces: 6, denoise: std::env::var_os("LC_BENCH_NODENOISE").is_none() };
    r.settle = true;
    // Timed from begin() (one raster frame for the frame uniforms + buffer
    // setup), like v0.10's render_still, so versions compare like for like.
    let t_trace = Instant::now();
    let mut job = pt.begin(&mut r, &world.scene, &fp, &targets, opts);
    job.target_ms = 250.0;
    let mut passes = 0u32;
    let mut pass_ms_sum = 0.0f32;
    let mut last = 0;
    while !job.finished() {
        pt.step(&r, &mut job, std::time::Duration::from_millis(250));
        if job.done_spp != last {
            passes += 1;
            pass_ms_sum += job.last_pass_ms;
            last = job.done_spp;
        }
    }
    let trace_s = t_trace.elapsed().as_secs_f32();
    let t_post = Instant::now();
    let (img, method) = pt.finish(&mut r, &world.scene, &fp, &targets, &mut job);
    let post_s = t_post.elapsed().as_secs_f32();
    if let Some(p) = out {
        img.save(p)?;
    }
    let m = pt.memory();
    let mb = |b: u64| b as f64 / 1048576.0;
    let samples = (width as f64) * (height as f64) * spp as f64;
    let text = format!(
        "scene={scene} size={width}x{height} spp={spp} gpu=\"{gpu}\" timestamps={timestamps}\n\
         triangles={} bvh_depth={} scene_build_s={build_s:.2} pt_sync_s={sync_s:.2} pt_resync_ms={resync_ms:.2}\n\
         textures_mb={:.1} color_layers(128/256/512/1K/2K/4K)={:?} data_layers={:?} demoted={} geometry_mb={:.1} job_buffers_mb={:.1}\n\
         trace_s={trace_s:.2} samples_per_s={:.3}M passes={passes} mean_pass_ms={:.1} denoise_post_s={post_s:.2} denoiser={:?}",
        pt.triangles(),
        pt.bvh_depth(),
        mb(m.textures),
        m.color_layers,
        m.data_layers,
        m.demoted,
        mb(m.geometry),
        mb(m.job_buffers + width as u64 * height as u64 * 48 + (width.div_ceil(32) * 32) as u64 * height as u64 * 8 * 3 * 2),
        samples / trace_s as f64 / 1e6,
        pass_ms_sum / passes.max(1) as f32,
        method,
    );
    pt.recycle(job);
    Ok(BenchResult { text })
}

/// Live-preview responsiveness benchmark (`lumencraft --bench-preview`):
/// runs the real live-preview code at a 60 Hz frame cadence, first with the
/// v0.11 blocking update and then with the asynchronous one, and reports
/// frame times and stalls as the user would see them.
pub fn run_preview(scene: &str, width: u32, height: u32, seconds: f32) -> Result<String> {
    use crate::preview::LivePreview;
    let world = build(scene)?;
    let (device, queue, gpu) = crate::headless::create_device()?;
    let limits = device.limits();
    let mut r = Renderer::new(device, queue);
    r.set_quality(crate::renderer::Quality::preset(4));
    r.sync_scene(&world.scene);
    let fp = world.frame_params(width as f32 / height as f32, 12.0);
    let mut tracer = PathTracer::new(&r.device);
    // Warm-up: scene upload, shader compilation, denoiser initialisation.
    {
        let mut p = LivePreview::start(&mut tracer, &mut r, &world.scene, &fp, (width, height), true, false);
        let t = Instant::now();
        while p.samples() < 2 && t.elapsed().as_secs_f32() < 60.0 {
            p.frame(&mut tracer, &mut r, &world.scene, std::time::Duration::from_millis(12));
        }
        p.recycle(&mut tracer);
    }
    let mut out = format!(
        "Render Mill live-preview benchmark\nscene={scene} size={width}x{height} seconds_per_mode={seconds}\ngpu=\"{gpu}\"\nlimits: max_buffer={} MB, max_storage_binding={} MB, max_texture_2d={}, max_texture_layers={}\nAI denoiser available: {}\n\n",
        limits.max_buffer_size >> 20,
        limits.max_storage_buffer_binding_size >> 20,
        limits.max_texture_dimension_2d,
        limits.max_texture_array_layers,
        crate::denoise::ai_available()
    );
    let frame_target = 1.0 / 60.0;
    for asynchronous in [false, true] {
        let mut p = LivePreview::start(&mut tracer, &mut r, &world.scene, &fp, (width, height), true, asynchronous);
        let mut frames: Vec<f32> = Vec::new();
        let t0 = Instant::now();
        while t0.elapsed().as_secs_f32() < seconds && !p.finished() {
            let t = Instant::now();
            p.frame(&mut tracer, &mut r, &world.scene, std::time::Duration::from_millis(12));
            let busy = t.elapsed().as_secs_f32();
            if busy < frame_target {
                std::thread::sleep(std::time::Duration::from_secs_f32(frame_target - busy));
            }
            frames.push(t.elapsed().as_secs_f32() * 1000.0);
        }
        let mut sorted = frames.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let pct = |q: f32| sorted.get(((sorted.len() as f32 - 1.0) * q) as usize).copied().unwrap_or(0.0);
        let total: f32 = frames.iter().sum::<f32>() / 1000.0;
        let ups = &p.updates;
        let avg = |f: &dyn Fn(&crate::preview::UpdateStat) -> f32| if ups.is_empty() { 0.0 } else { ups.iter().map(f).sum::<f32>() / ups.len() as f32 };
        let max_blocked = ups.iter().map(|u| u.blocked_ms).fold(0.0, f32::max);
        let reach = |n: u32| p.image_times.iter().find(|(s, _)| *s >= n).map_or("-".to_string(), |(_, t)| format!("{t:.1}s"));
        out += &format!(
            "== {} preview update\n\
             frames={} avg_fps={:.1} frame_ms p50={:.1} p95={:.1} p99={:.1} max={:.1}\n\
             stalls: frames>50ms={} frames>100ms={} longest={:.0}ms\n\
             updates={} ui_blocked_per_update avg={:.1}ms max={:.1}ms  update_latency avg={:.0}ms  denoise avg={:.0}ms\n\
             samples shown at end={}  time to 1/4/16/64/256 spp: {} {} {} {} {}\n\n",
            if asynchronous { "ASYNC (v0.12)" } else { "BLOCKING (v0.11)" },
            frames.len(),
            frames.len() as f32 / total.max(1e-3),
            pct(0.5),
            pct(0.95),
            pct(0.99),
            sorted.last().copied().unwrap_or(0.0),
            frames.iter().filter(|&&f| f > 50.0).count(),
            frames.iter().filter(|&&f| f > 100.0).count(),
            sorted.last().copied().unwrap_or(0.0),
            ups.len(),
            avg(&|u| u.blocked_ms),
            max_blocked,
            avg(&|u| u.latency_ms),
            avg(&|u| u.denoise_ms),
            p.samples(),
            reach(1),
            reach(4),
            reach(16),
            reach(64),
            reach(256),
        );
        p.recycle(&mut tracer);
    }
    Ok(out)
}
