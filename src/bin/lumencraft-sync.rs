//! lumencraft-sync — push a model into a running LumenCraft over the LiveSync
//! protocol, optionally re-sending whenever the file changes.
//!
//! This is also the reference client for BimCraft: it shows exactly which
//! messages to send (see `lumencraft::protocol`).

use lumencraft::import::{self, ImportSettings, Units, UpAxis};
use lumencraft::protocol::{Header, SyncClient, DEFAULT_PORT};
use notify::Watcher;
use std::path::{Path, PathBuf};
use std::time::Duration;

const USAGE: &str = "lumencraft-sync <model.fbx|gltf|glb|obj> [--addr 127.0.0.1:47811] [--token 123456-789012] [--up y|z] [--units m|ft|in|mm|cm] [--watch]";

fn push(addr: &str, token: Option<&str>, path: &Path, settings: &ImportSettings) -> anyhow::Result<()> {
    let model = import::load(path, settings)?;
    let mut c = SyncClient::connect_with_token(addr, "lumencraft-sync", token)?;
    c.send(&Header::BeginScene { clear: true }, &[])?;
    for m in &model.materials {
        let p = &m.params;
        c.send(
            &Header::Material {
                name: m.name.clone(),
                kind: Some(format!("{:?}", p.kind).to_lowercase()),
                base_color: Some(p.base_color),
                metallic: Some(p.metallic),
                roughness: Some(p.roughness),
                emissive: Some(p.emissive),
                emissive_strength: Some(p.emissive_strength),
                texture: None,
            },
            &[],
        )?;
    }
    let mut tris = 0;
    for (id, m) in &model.meshes {
        let pos: Vec<[f32; 3]> = m.positions.iter().map(|v| v.to_array()).collect();
        let nrm: Vec<[f32; 3]> = m.normals.iter().map(|v| v.to_array()).collect();
        let uv: Vec<[f32; 2]> = m.uvs.iter().map(|v| v.to_array()).collect();
        // Geometry was already converted to Y-up meters by the importer.
        c.mesh(id, &m.material, &pos, &nrm, &uv, &m.indices, "y", "m")?;
        tris += m.indices.len() / 3;
    }
    c.send(&Header::EndScene, &[])?;
    println!("Sent {} meshes / {} triangles from {}", model.meshes.len(), tris, path.display());
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(file) = args.first().filter(|a| !a.starts_with("--")) else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    let path = PathBuf::from(file);
    let mut addr = format!("127.0.0.1:{DEFAULT_PORT}");
    let mut settings = ImportSettings::default_for(&path);
    let mut watch = false;
    let mut token: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--addr" => {
                addr = args.get(i + 1).cloned().unwrap_or(addr);
                i += 1;
            }
            "--up" => {
                settings.up = if args.get(i + 1).is_some_and(|s| s == "z") { UpAxis::Z } else { UpAxis::Y };
                i += 1;
            }
            "--units" => {
                settings.units = args.get(i + 1).and_then(|s| Units::parse(s)).unwrap_or(settings.units);
                i += 1;
            }
            "--watch" => watch = true,
            "--token" => {
                token = args.get(i + 1).cloned();
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    // Keep the placement stable between pushes.
    if let Ok(m) = import::load(&path, &settings) {
        settings.offset = Some(m.offset.to_array());
    }
    if let Err(e) = push(&addr, token.as_deref(), &path, &settings) {
        eprintln!("Sync failed: {e:#}");
        if !watch {
            std::process::exit(1);
        }
    }
    if !watch {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let mut w = notify::recommended_watcher(move |r: notify::Result<notify::Event>| {
        if let Ok(e) = r {
            if !e.kind.is_access() {
                let _ = tx.send(());
            }
        }
    })
    .expect("watcher");
    w.watch(path.parent().unwrap_or(Path::new(".")), notify::RecursiveMode::NonRecursive).expect("watch");
    println!("Watching {} — Ctrl+C to stop", path.display());
    loop {
        if rx.recv().is_err() {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
        while rx.try_recv().is_ok() {}
        if let Err(e) = push(&addr, token.as_deref(), &path, &settings) {
            eprintln!("Sync failed: {e:#}");
        }
    }
}
