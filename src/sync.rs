//! Live sync: (1) a folder/file watcher that reloads exports the moment Revit
//! (or anything else) rewrites them, and (2) the LiveSync TCP server.

use crate::import::{self, ImportSettings, ImportedModel, Units, UpAxis};
use crate::protocol::{self, Header};
use crate::scene::{guess_params_from_name, Material, MaterialKind, MaterialParams, MeshData};
use glam::{Vec2, Vec3};
use notify::{RecursiveMode, Watcher};
use std::io::BufReader;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

pub enum SyncEvent {
    FileLoaded { path: PathBuf, model: ImportedModel, millis: u128 },
    FileError(String),
    NetClear { client: String },
    NetMesh { id: String, data: MeshData },
    NetRemove { id: String },
    NetMaterial(Material),
    NetCamera { position: Vec3, target: Vec3, fov: Option<f32> },
    Status(String),
}

// ------------------------------------------------------------------ file watch

enum WatchCmd {
    Reload,
    Stop,
}

pub struct FileSync {
    pub target: PathBuf,
    cmd: Sender<WatchCmd>,
    _watcher: Option<notify::RecommendedWatcher>,
}

impl FileSync {
    /// `target` may be a model file, or a folder (then the newest supported
    /// file in it is used — handy when exporting with timestamped names).
    pub fn start(target: PathBuf, settings: Arc<Mutex<ImportSettings>>, events: Sender<SyncEvent>, wake: impl Fn() + Send + 'static) -> anyhow::Result<Self> {
        let (cmd_tx, cmd_rx) = channel::<WatchCmd>();
        let (fs_tx, fs_rx) = channel::<()>();
        let watch_dir = if target.is_dir() { target.clone() } else { target.parent().unwrap_or(Path::new(".")).to_path_buf() };
        let tgt = target.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                if ev.kind.is_access() {
                    return;
                }
                let relevant = ev.paths.iter().any(|p| {
                    if tgt.is_dir() {
                        import::is_supported(p) || p.extension().is_some_and(|e| e == "mtl" || e == "bin" || e == "png" || e == "jpg" || e == "fbm")
                    } else {
                        p.file_name() == tgt.file_name() || p.extension().is_some_and(|e| e == "mtl" || e == "bin")
                    }
                });
                if relevant {
                    let _ = fs_tx.send(());
                }
            }
        })?;
        watcher.watch(&watch_dir, RecursiveMode::NonRecursive)?;

        let t2 = target.clone();
        let ev2 = events.clone();
        std::thread::Builder::new().name("file-sync".into()).spawn(move || {
            let load = |why: &str| {
                let Some(file) = resolve_target(&t2) else {
                    let _ = ev2.send(SyncEvent::FileError(format!("No .fbx/.gltf/.glb/.obj found in {}", t2.display())));
                    return;
                };
                wait_until_stable(&file);
                let s = settings.lock().unwrap().clone();
                let start = Instant::now();
                let _ = ev2.send(SyncEvent::Status(format!("{why}: loading {}", file.display())));
                match import::load(&file, &s) {
                    Ok(model) => {
                        let _ = ev2.send(SyncEvent::FileLoaded { path: file, model, millis: start.elapsed().as_millis() });
                    }
                    Err(e) => {
                        let _ = ev2.send(SyncEvent::FileError(format!("{e:#}")));
                    }
                }
                wake();
            };
            load("Opened");
            loop {
                // Merge watcher pings and commands; debounce bursts of writes.
                let mut dirty = false;
                match cmd_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(WatchCmd::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                    Ok(WatchCmd::Reload) => {
                        load("Reload");
                        continue;
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                }
                while fs_rx.try_recv().is_ok() {
                    dirty = true;
                }
                if dirty {
                    // Exporters write in several chunks; wait for quiet.
                    let mut quiet_since = Instant::now();
                    while quiet_since.elapsed() < Duration::from_millis(400) {
                        std::thread::sleep(Duration::from_millis(50));
                        while fs_rx.try_recv().is_ok() {
                            quiet_since = Instant::now();
                        }
                    }
                    load("Change detected");
                }
            }
        })?;

        Ok(Self { target, cmd: cmd_tx, _watcher: Some(watcher) })
    }

    pub fn reload(&self) {
        let _ = self.cmd.send(WatchCmd::Reload);
    }
}

impl Drop for FileSync {
    fn drop(&mut self) {
        let _ = self.cmd.send(WatchCmd::Stop);
    }
}

fn resolve_target(t: &Path) -> Option<PathBuf> {
    if !t.is_dir() {
        return t.exists().then(|| t.to_path_buf());
    }
    std::fs::read_dir(t)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| import::is_supported(p))
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH))
}

fn wait_until_stable(p: &Path) {
    let mut last = None;
    for _ in 0..40 {
        let now = p.metadata().ok().map(|m| (m.len(), m.modified().ok()));
        if now.is_some() && now == last {
            return;
        }
        last = now;
        std::thread::sleep(Duration::from_millis(120));
    }
}

// ------------------------------------------------------------------ tcp server

pub struct NetServer {
    pub addr: String,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for NetServer {
    fn drop(&mut self) {
        // Unblock accept() so the old listener thread exits and frees the port.
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr.replace("0.0.0.0", "127.0.0.1"));
    }
}

/// Random pairing code required from non-local LiveSync clients.
pub fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..2u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i ^ std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64));
        out.push_str(&format!("{:06}", h.finish() % 1_000_000));
    }
    format!("{}-{}", &out[..6], &out[6..])
}

impl NetServer {
    /// `token`: pairing code required from other computers (local connections
    /// from this machine are always allowed).
    pub fn start(bind: &str, token: String, events: Sender<SyncEvent>, wake: Arc<dyn Fn() + Send + Sync>) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(bind)?;
        let addr = listener.local_addr()?.to_string();
        let ev = events.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop2 = stop.clone();
        let token = Arc::new(token);
        std::thread::Builder::new().name("livesync-server".into()).spawn(move || {
            for stream in listener.incoming().flatten() {
                if stop2.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                let ev = ev.clone();
                let wake = wake.clone();
                let token = token.clone();
                std::thread::spawn(move || {
                    let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
                    match handle_client(stream, &token, &ev, &*wake) {
                        Err(e) if e.kind() != std::io::ErrorKind::UnexpectedEof => {
                            let _ = ev.send(SyncEvent::Status(format!("LiveSync client {peer} disconnected: {e}")));
                        }
                        _ => {
                            let _ = ev.send(SyncEvent::Status(format!("LiveSync client {peer} disconnected")));
                        }
                    }
                    wake();
                });
            }
        })?;
        Ok(Self { addr, stop })
    }
}

pub fn parse_kind(s: &str) -> MaterialKind {
    match s.to_ascii_lowercase().as_str() {
        "glass" => MaterialKind::Glass,
        "water" => MaterialKind::Water,
        "emissive" | "light" => MaterialKind::Emissive,
        "foliage" | "vegetation" => MaterialKind::Foliage,
        "landscape" | "ground" | "grass" => MaterialKind::Ground,
        _ => MaterialKind::Standard,
    }
}

fn settings_for(up: &Option<String>, units: &Option<String>) -> ImportSettings {
    ImportSettings {
        up: if up.as_deref().is_some_and(|u| u.eq_ignore_ascii_case("z")) { UpAxis::Z } else { UpAxis::Y },
        units: units.as_deref().and_then(Units::parse).unwrap_or(Units::Meters),
        recenter: false,
        offset: None,
        texture_dirs: Vec::new(),
    }
}

fn handle_client(stream: TcpStream, token: &str, ev: &Sender<SyncEvent>, wake: &dyn Fn()) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    let local = stream.peer_addr().is_ok_and(|a| a.ip().is_loopback());
    let mut writer = stream.try_clone()?;
    // Until a valid hello arrives, only accept small frames and don't wait forever.
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    let mut authed = false;
    let mut reader = BufReader::with_capacity(1 << 20, stream);
    let mut client = "client".to_string();
    let mut last_wake = Instant::now();
    loop {
        let (h, payload) = protocol::read_frame(&mut reader)?;
        if !authed && !matches!(h, Header::Hello { .. }) {
            let _ = protocol::write_frame(&mut writer, &Header::Error { message: "send hello first".into() }, &[]);
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "no hello"));
        }
        match h {
            Header::Hello { client: c, version, token: t } => {
                if !local && t.as_deref() != Some(token) {
                    let _ = protocol::write_frame(
                        &mut writer,
                        &Header::Error { message: "wrong or missing pairing code (see Render Mill > LiveSync)".into() },
                        &[],
                    );
                    return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "bad pairing code"));
                }
                authed = true;
                reader.get_ref().set_read_timeout(None)?;
                client = c;
                protocol::write_frame(&mut writer, &Header::Welcome { server: "LumenCraft".into(), version: protocol::PROTOCOL_VERSION }, &[])?;
                let _ = ev.send(SyncEvent::Status(format!("LiveSync: {client} connected (protocol v{version})")));
            }
            Header::BeginScene { clear } => {
                if clear {
                    let _ = ev.send(SyncEvent::NetClear { client: client.clone() });
                }
            }
            Header::EndScene => {
                let _ = ev.send(SyncEvent::Status(format!("LiveSync: scene received from {client}")));
            }
            Header::Material { name, kind, base_color, metallic, roughness, emissive, emissive_strength, texture } => {
                let mut p = guess_params_from_name(&name, MaterialParams::default());
                if let Some(k) = kind {
                    p.kind = parse_kind(&k);
                }
                if let Some(c) = base_color {
                    p.base_color = c;
                }
                if let Some(v) = metallic {
                    p.metallic = v;
                }
                if let Some(v) = roughness {
                    p.roughness = v;
                }
                if let Some(v) = emissive {
                    p.emissive = v;
                }
                if let Some(v) = emissive_strength {
                    p.emissive_strength = v;
                }
                // Only trust file paths from this machine; never let a remote
                // client make us read arbitrary local files.
                let texture = if local {
                    texture
                        .filter(|t| {
                            let e = Path::new(t).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                            matches!(e.as_str(), "png" | "jpg" | "jpeg")
                        })
                        .and_then(|t| import::load_texture_file(Path::new(&t)))
                } else {
                    None
                };
                let _ = ev.send(SyncEvent::NetMaterial(Material { name, params: p, texture, normal_texture: None, roughness_texture: None }));
            }
            Header::Mesh { id, name, material, vertex_count, index_count, has_normals, has_uvs, up_axis, units } => {
                let m = protocol::decode_mesh(&payload, vertex_count, index_count, has_normals, has_uvs)?;
                let s = settings_for(&up_axis, &units);
                let basis = s.basis();
                let nb = basis.inverse().transpose();
                let data = MeshData {
                    name: name.unwrap_or_else(|| id.clone()),
                    material,
                    positions: m.positions.iter().map(|p| basis * Vec3::from(*p)).collect(),
                    normals: m.normals.iter().map(|n| (nb * Vec3::from(*n)).normalize_or_zero()).collect(),
                    uvs: m.uvs.iter().map(|u| Vec2::from(*u)).collect(),
                    indices: m.indices,
                };
                let _ = ev.send(SyncEvent::NetMesh { id: format!("net:{id}"), data });
            }
            Header::Remove { id } => {
                let _ = ev.send(SyncEvent::NetRemove { id: format!("net:{id}") });
            }
            Header::Camera { position, target, fov, up_axis, units } => {
                let b = settings_for(&up_axis, &units).basis();
                let _ = ev.send(SyncEvent::NetCamera { position: b * Vec3::from(position), target: b * Vec3::from(target), fov });
            }
            Header::Ping | Header::Welcome { .. } | Header::Error { .. } => {}
        }
        if last_wake.elapsed() > Duration::from_millis(30) {
            wake();
            last_wake = Instant::now();
        }
    }
}

/// Drain helper used by the app each frame.
pub fn drain(rx: &Receiver<SyncEvent>, max: usize) -> Vec<SyncEvent> {
    let mut v = Vec::new();
    while v.len() < max {
        match rx.try_recv() {
            Ok(e) => v.push(e),
            Err(_) => break,
        }
    }
    v
}
