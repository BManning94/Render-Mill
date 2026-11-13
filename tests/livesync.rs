use lumencraft::project::World;
use lumencraft::protocol::{Header, SyncClient};
use lumencraft::sync::{NetServer, SyncEvent};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn tcp_livesync_upserts_and_removes_meshes() {
    let (tx, rx) = channel();
    let server = NetServer::start("127.0.0.1:0", "123456-654321".into(), tx, Arc::new(|| {})).unwrap();
    let mut c = SyncClient::connect(&server.addr, "test").unwrap();
    c.send(&Header::BeginScene { clear: true }, &[]).unwrap();
    c.send(
        &Header::Material {
            name: "Glass - Curtain Wall".into(),
            kind: None,
            base_color: None,
            metallic: None,
            roughness: None,
            emissive: None,
            emissive_strength: None,
            texture: None,
        },
        &[],
    )
    .unwrap();
    // A 1ft x 1ft square in Revit coordinates (Z-up, feet)
    let pos = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    c.mesh("wall-1", "Glass - Curtain Wall", &pos, &[], &[], &[0, 1, 2, 0, 2, 3], "z", "ft").unwrap();
    c.send(&Header::Remove { id: "nope".into() }, &[]).unwrap();
    c.send(&Header::EndScene, &[]).unwrap();

    let mut world = World::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut got_mesh = false;
    while Instant::now() < deadline && !got_mesh {
        if let Ok(ev) = rx.recv_timeout(Duration::from_millis(100)) {
            got_mesh |= matches!(ev, SyncEvent::NetMesh { .. });
            world.apply(ev);
        }
    }
    assert!(got_mesh, "mesh never arrived");
    let m = world.scene.meshes.get("net:wall-1").expect("mesh stored under net: prefix");
    // Z-up feet -> Y-up meters: the (0,0,1ft) vertex becomes (0, 0.3048, 0)
    let top = m.data.positions[3];
    assert!((top.y - 0.3048).abs() < 1e-5, "{top:?}");
    assert_eq!(world.scene.effective("Glass - Curtain Wall").kind, lumencraft::scene::MaterialKind::Glass);

    c.send(&Header::Remove { id: "wall-1".into() }, &[]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && world.scene.meshes.contains_key("net:wall-1") {
        if let Ok(ev) = rx.recv_timeout(Duration::from_millis(100)) {
            world.apply(ev);
        }
    }
    assert!(!world.scene.meshes.contains_key("net:wall-1"));
}

#[test]
fn material_overrides_survive_resync() {
    use lumencraft::scene::{MaterialParams, MeshData};
    let mut world = World::new();
    let tri = || MeshData {
        name: "a".into(),
        material: "Brick, Common".into(),
        positions: vec![glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Y],
        ..Default::default()
    };
    world.scene.upsert_mesh("file:Brick".into(), tri());
    let edit = MaterialParams { roughness: 0.123, ..Default::default() };
    world.scene.overrides.insert("Brick, Common".into(), edit.clone());
    // simulated re-export
    world.scene.remove_prefix("file:");
    world.scene.upsert_mesh("file:Brick".into(), tri());
    assert_eq!(world.scene.effective("Brick, Common"), edit);
}

#[test]
fn file_watch_reloads_on_change() {
    use lumencraft::import::ImportSettings;
    use lumencraft::sync::FileSync;
    use std::sync::Mutex;
    let dir = std::env::temp_dir().join(format!("lc-watch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let obj = dir.join("model.obj");
    std::fs::write(&obj, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();
    let (tx, rx) = channel();
    let settings = Arc::new(Mutex::new(ImportSettings::default_for(&obj)));
    let _fs = FileSync::start(obj.clone(), settings, tx, || {}).unwrap();
    let wait_loaded = |want_tris: usize| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(SyncEvent::FileLoaded { model, .. }) = rx.recv_timeout(Duration::from_millis(100)) {
                let t: usize = model.meshes.iter().map(|(_, m)| m.indices.len() / 3).sum();
                if t == want_tris {
                    return true;
                }
            }
        }
        false
    };
    assert!(wait_loaded(1), "initial load");
    std::thread::sleep(Duration::from_millis(300));
    // Simulate a re-export from Revit: the file is rewritten with more geometry.
    std::fs::write(&obj, "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 1 1 0\nf 1 2 3\nf 2 4 3\n").unwrap();
    assert!(wait_loaded(2), "reload after change");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Find this machine's non-loopback address (no packets are sent).
fn lan_ip() -> Option<std::net::IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("10.255.255.255:1").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

#[test]
fn remote_clients_need_the_pairing_code() {
    let Some(ip) = lan_ip() else {
        eprintln!("no LAN address; skipping");
        return;
    };
    let (tx, rx) = channel();
    let server = NetServer::start("0.0.0.0:0", "111111-222222".into(), tx, Arc::new(|| {})).unwrap();
    let port = server.addr.rsplit(':').next().unwrap();
    let addr = format!("{ip}:{port}");
    // Wrong code is refused before any geometry is accepted.
    let err = SyncClient::connect_with_token(&addr, "intruder", Some("000000-000000")).err().expect("must be refused");
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(SyncClient::connect_with_token(&addr, "intruder", None).is_err());
    // Correct code works.
    let mut ok = SyncClient::connect_with_token(&addr, "bimcraft", Some("111111-222222")).unwrap();
    ok.mesh("a", "m", &[[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], &[], &[], &[0, 1, 2], "y", "m").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut got = false;
    while Instant::now() < deadline && !got {
        if let Ok(SyncEvent::NetMesh { .. }) = rx.recv_timeout(Duration::from_millis(100)) {
            got = true;
        }
    }
    assert!(got);
    // Localhost never needs the code.
    assert!(SyncClient::connect(&format!("127.0.0.1:{port}"), "local").is_ok());
}

#[test]
fn server_restart_frees_the_port() {
    let (tx, _rx) = channel();
    let s1 = NetServer::start("127.0.0.1:0", "x".into(), tx.clone(), Arc::new(|| {})).unwrap();
    let addr = s1.addr.clone();
    drop(s1);
    let mut ok = false;
    for _ in 0..40 {
        if NetServer::start(&addr, "x".into(), tx.clone(), Arc::new(|| {})).is_ok() {
            ok = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(ok, "port {addr} was not released");
}
