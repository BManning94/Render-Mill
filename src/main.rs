#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;

const USAGE: &str = "Render Mill — real-time architectural renderer with live sync

USAGE:
  RenderMill                          open the start screen
  RenderMill <model.gltf|glb|obj>     open a model and live-sync it
  RenderMill <export-folder>          watch a folder, always showing the newest export
  RenderMill <project.lcproj>         open a saved project
  RenderMill --render <input|--demo> <out.png> [--size 1920x1080] [--view N]
             [--preset \"Golden hour\"] [--camera ex,ey,ez,tx,ty,tz]
             [--raytrace fast|high|ultra|max|<samples>] [--no-denoise]
                                      render an image without opening a window
  RenderMill --bench-pt <small-exterior|large-exterior|interior|large-model|all> [--size 1280x720] [--spp 64] [--out DIR]
                                      path-tracing benchmark (time, samples/s, GPU memory)
  RenderMill --movie <input|--demo> <out-folder> [--size 1280x720] [--fps 30] [--seconds 8]
                                      render the project's movie clips (or an orbit)
                                      to PNG frames + MP4 when ffmpeg is available";

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,lumencraft=info")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The graphics card chosen in Settings (read by the wgpu setup).
    if std::env::var_os("LUMENCRAFT_GPU").is_none() {
        if let Some(g) = lumencraft::library::AppSettings::load().gpu {
            std::env::set_var("LUMENCRAFT_GPU", g);
        }
    }

    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return;
    }
    // Release builds are GUI-subsystem on Windows; re-attach to the terminal
    // so command-line renders can print progress and errors.
    #[cfg(windows)]
    if !args.is_empty() && args[0].starts_with("--") {
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
        }
    }
    if args.first().map(String::as_str) == Some("--bench-preview") {
        // Live-preview stalls: --bench-preview [scene] [--size WxH] [--seconds N] [--out FILE]
        let scene = args.get(1).filter(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| "small-exterior".into());
        let (mut w, mut h, mut secs, mut out) = (1600u32, 900u32, 20.0f32, None::<PathBuf>);
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--size" => {
                    if let Some((a, b)) = args.get(i + 1).and_then(|s| s.split_once('x')) {
                        w = a.parse().unwrap_or(w);
                        h = b.parse().unwrap_or(h);
                    }
                    i += 1;
                }
                "--seconds" => {
                    secs = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(secs);
                    i += 1;
                }
                "--out" => {
                    out = args.get(i + 1).map(PathBuf::from);
                    i += 1;
                }
                _ => {}
            }
            i += 1;
        }
        match lumencraft::bench::run_preview(&scene, w, h, secs) {
            Ok(report) => {
                println!("{report}");
                let path = out.unwrap_or_else(|| {
                    std::env::current_exe()
                        .ok()
                        .and_then(|e| e.parent().map(|p| p.join("preview-benchmark.txt")))
                        .unwrap_or_else(|| PathBuf::from("preview-benchmark.txt"))
                });
                let mut text = std::fs::read_to_string(&path).unwrap_or_default();
                text.push_str(&report);
                let _ = std::fs::write(&path, text);
                eprintln!("report appended to {}", path.display());
            }
            Err(e) => {
                eprintln!("preview benchmark failed: {e:#}");
                std::process::exit(1);
            }
        }
        return;
    }
    if args.first().map(String::as_str) == Some("--bench-pt") {
        // Path-tracing benchmark: --bench-pt <scene|all> [--size WxH] [--spp N] [--out DIR]
        let which = args.get(1).cloned().unwrap_or_else(|| "all".into());
        let (mut w, mut h, mut spp, mut out) = (1280u32, 720u32, 64u32, None::<PathBuf>);
        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
                "--size" => {
                    if let Some((a, b)) = args.get(i + 1).and_then(|s| s.split_once('x')) {
                        w = a.parse().unwrap_or(w);
                        h = b.parse().unwrap_or(h);
                    }
                    i += 1;
                }
                "--spp" => {
                    spp = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(spp);
                    i += 1;
                }
                "--out" => {
                    out = args.get(i + 1).map(PathBuf::from);
                    i += 1;
                }
                _ => {}
            }
            i += 1;
        }
        let scenes: Vec<&str> = if which == "all" { lumencraft::bench::SCENES.to_vec() } else { vec![which.as_str()] };
        for sc in scenes {
            let img = out.as_ref().map(|d| d.join(format!("{sc}.png")));
            match lumencraft::bench::run(sc, w, h, spp, img.as_deref()) {
                Ok(r) => println!(
                    "{}
",
                    r.text
                ),
                Err(e) => {
                    eprintln!("benchmark {sc} failed: {e:#}");
                    std::process::exit(1);
                }
            }
        }
        return;
    }
    if args.first().map(String::as_str) == Some("--model-info") {
        // --model-info <file>: per-material triangle counts and bounds after import.
        let Some(f) = args.get(1) else { std::process::exit(2) };
        let path = std::path::Path::new(f);
        match lumencraft::import::load(path, &lumencraft::import::ImportSettings { recenter: false, ..lumencraft::import::ImportSettings::default_for(path) }) {
            Ok(m) => {
                let mut by: std::collections::BTreeMap<String, (usize, glam::Vec3, glam::Vec3)> = Default::default();
                for (_, md) in &m.meshes {
                    let e = by.entry(md.material.clone()).or_insert((0, glam::Vec3::splat(f32::MAX), glam::Vec3::splat(f32::MIN)));
                    e.0 += md.indices.len() / 3;
                    for p in &md.positions {
                        e.1 = e.1.min(*p);
                        e.2 = e.2.max(*p);
                    }
                }
                for (k, (t, lo, hi)) in by {
                    println!("{t:8} tris  {k}  [{:.2} {:.2} {:.2}] .. [{:.2} {:.2} {:.2}]", lo.x, lo.y, lo.z, hi.x, hi.y, hi.z);
                }
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if args.first().map(String::as_str) == Some("--match-photo") {
        // --match-photo <photo> <texture folder> [<out overlay.png>]: regions and closest textures.
        let (Some(photo), Some(dir)) = (args.get(1), args.get(2)) else {
            eprintln!("usage: --match-photo <photo> <texture folder> [overlay.png]");
            std::process::exit(2);
        };
        if let Err(e) = lumencraft::photomatch::cli(std::path::Path::new(photo), std::path::Path::new(dir), args.get(3).map(std::path::Path::new)) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    if args.first().map(String::as_str) == Some("--bench") {
        let input = args.get(1).cloned().unwrap_or_else(|| "--demo".into());
        let (mut w, mut h, mut frames, mut q) = (1280, 720, 20, None);
        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
                "--size" => {
                    if let Some((a, b)) = args.get(i + 1).and_then(|s| s.split_once('x')) {
                        w = a.parse().unwrap_or(w);
                        h = b.parse().unwrap_or(h);
                    }
                    i += 1;
                }
                "--frames" => {
                    frames = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(frames);
                    i += 1;
                }
                "--quality" => {
                    q = args.get(i + 1).and_then(|s| s.parse().ok());
                    i += 1;
                }
                _ => {}
            }
            i += 1;
        }
        if let Err(e) = lumencraft::headless::bench(&input, w, h, frames, q) {
            eprintln!("Bench failed: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    if args.first().map(String::as_str) == Some("--movie") {
        if args.len() < 3 {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
        let (mut w, mut h, mut fps, mut secs) = (1280, 720, 30, 8.0);
        let mut i = 3;
        while i < args.len() {
            match args[i].as_str() {
                "--size" => {
                    if let Some((a, b)) = args.get(i + 1).and_then(|s| s.split_once('x')) {
                        w = a.parse().unwrap_or(w);
                        h = b.parse().unwrap_or(h);
                    }
                    i += 1;
                }
                "--fps" => {
                    fps = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(fps);
                    i += 1;
                }
                "--seconds" => {
                    secs = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(secs);
                    i += 1;
                }
                _ => {}
            }
            i += 1;
        }
        if let Err(e) = lumencraft::headless::render_movie(&args[1], std::path::Path::new(&args[2]), w, h, fps, secs) {
            eprintln!("Movie render failed: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    if args.first().map(String::as_str) == Some("--render") {
        if args.len() < 3 {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
        let mut o = lumencraft::headless::RenderOpts { width: 1920, height: 1080, view: None, preset: None, camera: None, raytrace: None };
        let mut i = 3;
        while i < args.len() {
            match args[i].as_str() {
                "--size" => {
                    if let Some((a, b)) = args.get(i + 1).and_then(|s| s.split_once('x')) {
                        o.width = a.parse().unwrap_or(o.width);
                        o.height = b.parse().unwrap_or(o.height);
                    }
                    i += 1;
                }
                "--view" => {
                    o.view = args.get(i + 1).and_then(|s| s.parse().ok());
                    i += 1;
                }
                "--preset" => {
                    o.preset = args.get(i + 1).cloned();
                    i += 1;
                }
                "--raytrace" => {
                    let v = args.get(i + 1).map(|s| s.to_ascii_lowercase()).unwrap_or_default();
                    let mut p = match v.as_str() {
                        "fast" => lumencraft::pathtrace::PtOpts::preset(0),
                        "high" => lumencraft::pathtrace::PtOpts::preset(1),
                        "ultra" => lumencraft::pathtrace::PtOpts::preset(2),
                        "max" => lumencraft::pathtrace::PtOpts::preset(3),
                        n => {
                            let mut p = lumencraft::pathtrace::PtOpts::preset(1);
                            p.spp = n.parse().unwrap_or(p.spp).max(1);
                            p
                        }
                    };
                    if let Some(old) = o.raytrace {
                        p.denoise = old.denoise;
                    }
                    o.raytrace = Some(p);
                    i += 1;
                }
                "--no-denoise" => {
                    let mut p = o.raytrace.unwrap_or(lumencraft::pathtrace::PtOpts::preset(1));
                    p.denoise = false;
                    o.raytrace = Some(p);
                }
                "--camera" => {
                    let v: Vec<f32> = args.get(i + 1).map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default();
                    if v.len() == 6 {
                        o.camera = Some([v[0], v[1], v[2], v[3], v[4], v[5]]);
                    }
                    i += 1;
                }
                _ => {}
            }
            i += 1;
        }
        if let Err(e) = lumencraft::headless::render_to_file(&args[1], &PathBuf::from(&args[2]), &o) {
            eprintln!("Render failed: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    let open = args.first().map(PathBuf::from);
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title(lumencraft::APP_NAME)
            .with_inner_size([1600.0, 950.0])
            .with_min_inner_size([1100.0, 680.0])
            // Lumion-style custom title bar (drawn by the app).
            .with_decorations(false)
            .with_maximized(std::env::var("LC_WINDOWED").is_err()),
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            // Larger storage buffers for the path tracer (4K photos).
            device_descriptor: std::sync::Arc::new(|adapter| {
                let base = if adapter.get_info().backend == eframe::wgpu::Backend::Gl {
                    eframe::wgpu::Limits::downlevel_webgl2_defaults()
                } else {
                    lumencraft::pathtrace::wanted_limits(&adapter.limits())
                };
                eframe::wgpu::DeviceDescriptor {
                    label: Some("lumencraft"),
                    required_features: if adapter.get_info().backend == eframe::wgpu::Backend::Gl {
                        eframe::wgpu::Features::empty()
                    } else {
                        lumencraft::pathtrace::wanted_features(adapter.features())
                    },
                    required_limits: eframe::wgpu::Limits { max_texture_dimension_2d: 8192.min(adapter.limits().max_texture_dimension_2d), ..base },
                    memory_hints: Default::default(),
                }
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    if let Err(e) = eframe::run_native(lumencraft::APP_NAME, options, Box::new(move |cc| Ok(Box::new(lumencraft::app::LumenApp::new(cc, open))))) {
        eprintln!("Render Mill failed to start: {e}");
        std::process::exit(1);
    }
}
