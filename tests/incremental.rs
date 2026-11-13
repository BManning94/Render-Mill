use lumencraft::import::{self, ImportSettings};
use lumencraft::project::World;
use lumencraft::sync::SyncEvent;
use std::fmt::Write;
use std::time::Instant;

/// A "city block" OBJ in Revit units (feet, Z-up): n x n buildings.
fn city(n: usize, bump: Option<usize>) -> String {
    let mut s = String::from("o city\n");
    let mut v = 1;
    for i in 0..n * n {
        let (x, y) = ((i % n) as f32 * 60.0, (i / n) as f32 * 60.0);
        let h = 30.0 + (i % 7) as f32 * 10.0 + if Some(i) == bump { 25.0 } else { 0.0 };
        for (cx, cy, cz) in [(0., 0., 0.), (40., 0., 0.), (0., 40., 0.), (40., 40., 0.), (0., 0., h), (40., 0., h), (0., 40., h), (40., 40., h)] {
            writeln!(s, "v {} {} {}", x + cx, y + cy, cz).unwrap();
        }
        writeln!(s, "usemtl {}", if i % 3 == 0 { "Glass - Curtain Wall" } else { "Concrete" }).unwrap();
        for f in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
            writeln!(s, "f {} {} {} {}", v + f[0], v + f[1], v + f[2], v + f[3]).unwrap();
        }
        v += 8;
    }
    s
}

#[test]
fn editing_one_building_reuploads_only_its_chunk() {
    let dir = std::env::temp_dir().join(format!("lc-inc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("city.obj");
    std::fs::write(&path, city(30, None)).unwrap();

    let mut world = World::new();
    let mut settings = ImportSettings::default_for(&path);
    let t = Instant::now();
    let m = import::load(&path, &settings).unwrap();
    settings.offset = Some(m.offset.to_array());
    world.import = Some(settings.clone());
    world.apply(SyncEvent::FileLoaded { path: path.clone(), model: m, millis: 0 });
    let first = world.scene.meshes.iter().filter(|(k, _)| k.starts_with("file:")).count();
    println!("900 buildings -> {first} chunks in {:?}", t.elapsed());
    assert!(first > 10, "model should be split into spatial chunks");
    let revs: std::collections::HashMap<String, u64> = world.scene.meshes.iter().map(|(k, m)| (k.clone(), m.rev)).collect();

    // Re-export with one building made taller.
    std::fs::write(&path, city(30, Some(417))).unwrap();
    let m = import::load(&path, &settings).unwrap();
    let msg = world.apply(SyncEvent::FileLoaded { path: path.clone(), model: m, millis: 0 }).unwrap();
    println!("{msg}");
    let changed = world.scene.meshes.iter().filter(|(k, m)| k.starts_with("file:") && revs.get(*k) != Some(&m.rev)).count();
    assert!((1..=4).contains(&changed), "expected 1-4 changed chunks, got {changed}");
    assert!(first < 700, "too many chunks/draw calls: {first}");

    // Picking stays fast on the chunked model.
    let t = Instant::now();
    for i in 0..200 {
        let o = glam::Vec3::new(-200.0 + i as f32, 80.0, -200.0);
        let _ = world.scene.raycast(o, (glam::Vec3::new(0.0, 0.0, 0.0) - o).normalize());
    }
    println!("200 picks: {:?}", t.elapsed());
    let _ = std::fs::remove_dir_all(&dir);
}
