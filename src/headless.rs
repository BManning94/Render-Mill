//! Command-line rendering without a window (batch renders, CI, testing).

use crate::project::{Project, World};
use crate::renderer::Renderer;
use anyhow::{Context, Result};
use std::path::Path;
use std::sync::Arc;

pub fn create_device() -> Result<(Arc<wgpu::Device>, Arc<wgpu::Queue>, String)> {
    let instance = wgpu::Instance::default();
    let default = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }));
    // A graphics card chosen by name (Settings or LUMENCRAFT_GPU).
    let chosen = std::env::var("LUMENCRAFT_GPU").ok().filter(|s| !s.trim().is_empty()).and_then(|want| {
        let want = want.trim().to_lowercase();
        let mut all = instance.enumerate_adapters(wgpu::Backends::all());
        all.sort_by_key(|a| (a.get_info().backend == wgpu::Backend::Gl) as u8);
        all.into_iter().find(|a| a.get_info().name.to_lowercase().contains(&want))
    });
    let adapter = chosen.or(default).context("no GPU adapter found")?;
    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("lumencraft-headless"),
            required_features: crate::pathtrace::wanted_features(adapter.features()),
            required_limits: crate::pathtrace::wanted_limits(&adapter.limits()),
            memory_hints: Default::default(),
        },
        None,
    ))?;
    Ok((Arc::new(device), Arc::new(queue), format!("{} ({:?})", info.name, info.backend)))
}

/// `input` may be a model file, a .lcproj project, or "--demo".
pub struct RenderOpts {
    pub width: u32,
    pub height: u32,
    pub view: Option<usize>,
    pub preset: Option<String>,
    /// eye x,y,z + target x,y,z (render space: Y-up meters)
    pub camera: Option<[f32; 6]>,
    /// Path trace (with AI denoising) instead of rasterizing.
    pub raytrace: Option<crate::pathtrace::PtOpts>,
}

pub fn load_world(input: &str) -> Result<World> {
    if let Some(spec) = input.strip_prefix("--template:") {
        // --template:<id>[+demo]: a new project from a landscape template,
        // optionally with the demo house on it.
        let (id, demo) = match spec.strip_suffix("+demo") {
            Some(id) => (id, true),
            None => (spec, false),
        };
        let mut w = World::new();
        anyhow::ensure!(w.apply_template(id), "unknown template {id}");
        if demo {
            let cam = w.camera.clone();
            crate::demo::build_demo(&mut w);
            w.camera = cam;
        }
        w.update_ground();
        return Ok(w);
    }
    Ok(if input == "--demo" {
        let mut w = World::new();
        crate::demo::build_demo(&mut w);
        w
    } else if input.ends_with(".lcproj") {
        let p = Project::load(Path::new(input))?;
        let mut w = World::from_project(p);
        if let Some(src) = w.source.clone() {
            let target = if src.is_dir() {
                std::fs::read_dir(&src)?
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| crate::import::is_supported(p))
                    .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
                    .context("no model in source folder")?
            } else {
                src
            };
            w.load_model_now(&target)?;
        }
        w
    } else if crate::lumion::is_scene(Path::new(input)) {
        let mut w = World::new();
        let r = w.open_lumion_scene(Path::new(input))?;
        eprintln!("{}", r.summary());
        w
    } else {
        let mut w = World::new();
        w.load_model_now(Path::new(input))?;
        if let Some((lo, hi)) = w.scene.meshes.iter().filter(|(k, _)| k.starts_with("file:")).map(|(_, m)| m.bounds).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
        {
            w.camera.frame_bounds(lo, hi);
        }
        w
    })
}

/// Final-quality still: render larger than asked (2x up to 1440p, 1.5x up
/// to 4K) and filter down, for clean edges, foliage and fine detail.
pub fn render_supersampled(r: &mut Renderer, scene: &crate::scene::Scene, fp: &crate::renderer::FrameParams, w: u32, h: u32) -> image::RgbaImage {
    let max = r.device.limits().max_texture_dimension_2d;
    let mut f = if w * h <= 2560 * 1440 {
        2.0
    } else if w.max(h) <= 4096 {
        1.5
    } else {
        1.0
    };
    while f > 1.0 && ((w as f32 * f) as u32 > max || (h as f32 * f) as u32 > max) {
        f -= 0.25;
    }
    let (sw, sh) = ((w as f32 * f) as u32, (h as f32 * f) as u32);
    let targets = r.create_targets(sw, sh);
    // Effects measured in pixels (depth of field) scale with the resolution
    // automatically; render twice so cached shadows/settle are fully up to date.
    r.settle = true;
    r.render(scene, fp, &targets);
    let img = r.read_output(&targets);
    if (sw, sh) == (w, h) {
        img
    } else {
        image::imageops::resize(&img, w, h, image::imageops::FilterType::Lanczos3)
    }
}

pub fn render_to_file(input: &str, out: &Path, o: &RenderOpts) -> Result<()> {
    let (width, height, view) = (o.width, o.height, o.view);
    let mut world = load_world(input)?;
    if let Some(i) = view {
        if let Some(v) = world.views.get(i) {
            world.camera = v.camera.clone();
        }
    }
    if let Some(name) = &o.preset {
        let (_, env) =
            crate::env::Environment::presets().into_iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).with_context(|| format!("unknown preset '{name}'"))?;
        world.env = env;
    }
    if let Some(c) = o.camera {
        world.camera.position = [c[0], c[1], c[2]];
        world.camera.look_at(glam::Vec3::new(c[3], c[4], c[5]));
    }
    world.update_ground();
    let (device, queue, name) = create_device()?;
    eprintln!("GPU: {name}");
    let mut r = Renderer::new(device, queue);
    r.sync_scene(&world.scene);
    let fp = world.frame_params(width as f32 / height as f32, 12.0);
    // Refuse (with the reason and the largest size that works) before
    // allocating anything the card can't hold.
    if let Err(e) = crate::pathtrace::check_resolution(&r.device.limits(), width, height, o.raytrace.is_some()) {
        anyhow::bail!("{e}");
    }
    let t0 = std::time::Instant::now();
    let img = if let Some(opts) = o.raytrace {
        let mut pt = crate::pathtrace::PathTracer::new(&r.device);
        let mut last = -1i32;
        let (img, method) = crate::pathtrace::render_still(&mut r, &mut pt, &world.scene, &fp, width, height, opts, |p| {
            let pc = (p * 100.0) as i32;
            if pc / 10 != last / 10 {
                eprintln!("  ray tracing {pc}%");
                last = pc;
            }
            true
        })
        .context("ray tracing cancelled")?;
        eprintln!(
            "Ray traced {} samples/pixel, {} bounces; denoiser: {}",
            opts.spp,
            opts.bounces,
            match method {
                Some(crate::denoise::Method::Ai) => "AI (Open Image Denoise)",
                Some(crate::denoise::Method::Wavelet) => "wavelet",
                None => "off",
            }
        );
        img
    } else {
        render_supersampled(&mut r, &world.scene, &fp, width, height)
    };
    eprintln!("Rendered {}x{} ({} triangles) in {:.2}s", width, height, world.scene.triangle_count(), t0.elapsed().as_secs_f32());
    img.save(out)?;
    Ok(())
}

/// Render `frames` frames while orbiting the camera and report timings.
/// GPU work is waited on every frame so the numbers are real frame times.
pub fn bench(input: &str, width: u32, height: u32, frames: u32, quality: Option<u8>) -> Result<()> {
    let still = std::env::var("LC_BENCH_STILL").is_ok();
    let mut world = load_world(input)?;
    world.update_ground();
    let (device, queue, name) = create_device()?;
    eprintln!("GPU: {name}");
    let mut r = Renderer::new(device.clone(), queue);
    if let Some(q) = quality {
        r.set_quality(crate::renderer::Quality::preset(q));
    }
    let targets = r.create_targets(width, height);
    r.sync_scene(&world.scene);
    let center = world
        .scene
        .meshes
        .iter()
        .filter(|(k, _)| !k.starts_with("env:"))
        .map(|(_, m)| m.bounds)
        .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
        .map(|(lo, hi)| (lo + hi) * 0.5)
        .unwrap_or_default();
    let start = world.camera.pos();
    let mut times = Vec::new();
    for f in 0..frames + 2 {
        // slow orbit so caches can't hide the work
        let a = if still { 0.0 } else { f as f32 * 0.01 };
        let rel = start - center;
        let rot = glam::Mat3::from_rotation_y(a);
        world.camera.position = (center + rot * rel).to_array();
        world.camera.look_at(center);
        let fp = world.frame_params(width as f32 / height as f32, 12.0 + f as f32 * 0.016);
        let t0 = std::time::Instant::now();
        r.sync_scene(&world.scene);
        r.render(&world.scene, &fp, &targets);
        device.poll(wgpu::Maintain::Wait);
        if f >= 2 {
            times.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
    }
    times.sort_by(|a, b| a.total_cmp(b));
    let avg = times.iter().sum::<f64>() / times.len() as f64;
    let med = times[times.len() / 2];
    let p95 = times[(times.len() * 95 / 100).min(times.len() - 1)];
    let line = format!(
        "{input} @ {width}x{height}{}: avg {avg:.1} ms, median {med:.1} ms ({:.1} fps), p95 {p95:.1} ms, max {:.1} ms, {} tris, drawn {} culled {} [{name}]",
        quality.map(|q| format!(" q{q}")).unwrap_or_default(),
        1000.0 / med,
        times.last().copied().unwrap_or(0.0),
        world.scene.triangle_count(),
        r.stats.drawn,
        r.stats.culled
    );
    println!("{line}");
    // Also next to the exe, so the Windows build (no console) leaves a record.
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("viewport-benchmark.txt")) {
            let _ = writeln!(f, "{line}");
        }
    }
    Ok(())
}

/// Render a project's movie clips (or, for a project without clips, a slow
/// orbit around the model) to PNG frames, plus MP4 when ffmpeg is available.
pub fn render_movie(input: &str, out_dir: &Path, width: u32, height: u32, fps: u32, seconds: f32) -> Result<()> {
    let mut world = load_world(input)?;
    world.update_ground();
    let mut clips = world.clips.clone();
    if clips.is_empty() {
        let (lo, hi) = world
            .scene
            .meshes
            .iter()
            .filter(|(k, _)| !k.starts_with("env:"))
            .map(|(_, m)| m.bounds)
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
            .context("empty scene")?;
        let c = (lo + hi) * 0.5;
        let r = (hi - lo).length() * 0.75;
        let mut clip = crate::env::Clip::new("Orbit".into());
        clip.duration = seconds;
        for k in 0..5 {
            let a = 0.4 + k as f32 * 0.35;
            let mut cam = world.camera.clone();
            cam.position = [c.x + a.sin() * r, c.y + r * 0.22, c.z + a.cos() * r];
            cam.look_at(c);
            clip.keys.push(cam);
        }
        clips.push(clip);
    }
    let (device, queue, name) = create_device()?;
    eprintln!("GPU: {name}");
    let mut r = Renderer::new(device, queue);
    let targets = r.create_targets(width, height);
    r.sync_scene(&world.scene);
    std::fs::create_dir_all(out_dir)?;
    for clip in &clips {
        let frames = ((clip.duration * fps as f32).round() as u32).max(1);
        let dir = out_dir.join(format!("{} frames", clip.name));
        std::fs::create_dir_all(&dir)?;
        for f in 0..frames {
            let t = f as f32 / fps as f32;
            world.camera = clip.camera_at(t).context("clip has no keyframes")?;
            let fp = world.frame_params(width as f32 / height as f32, 10.0 + t);
            r.settle = true;
            r.render(&world.scene, &fp, &targets);
            r.read_output(&targets).save(dir.join(format!("frame_{f:05}.png")))?;
            eprint!("\r{}: frame {}/{}", clip.name, f + 1, frames);
        }
        eprintln!();
        if let Some(ff) = crate::app::find_ffmpeg() {
            let mp4 = out_dir.join(format!("{}.mp4", clip.name));
            let st = std::process::Command::new(ff)
                .args(["-y", "-loglevel", "error", "-framerate", &fps.to_string(), "-i"])
                .arg(dir.join("frame_%05d.png"))
                .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18"])
                .arg(&mp4)
                .status()?;
            eprintln!("{} {}", if st.success() { "Wrote" } else { "ffmpeg failed for" }, mp4.display());
        }
    }
    Ok(())
}
