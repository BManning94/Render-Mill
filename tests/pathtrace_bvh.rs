//! GPU traversal regression tests: the path tracer's first-hit distance for
//! every pixel must match a CPU brute-force intersection of the same rays, on
//! scenes built to stress the BVH (dense clutter, scales from millimetres to
//! kilometres, coordinates far from the origin, stacked duplicate geometry).
//! A traversal that drops nodes shows up as pixels that see "sky" or a
//! farther surface than the CPU.

use glam::{Mat4, Vec3, Vec4};
use lumencraft::pathtrace::{PathTracer, PtOpts};
use lumencraft::project::World;
use lumencraft::renderer::Renderer;
use lumencraft::scene::{MeshData, Scene};

const W: u32 = 96;
const H: u32 = 64;

fn rng(seed: &mut u32) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    (*seed & 0xFFFFFF) as f32 / 16777216.0
}

fn ray_tri(o: Vec3, d: Vec3, t: &[Vec3; 3]) -> Option<f32> {
    let e1 = t[1] - t[0];
    let e2 = t[2] - t[0];
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det == 0.0 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - t[0];
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 1e-6).then_some(t)
}

fn mesh(tris: &[[Vec3; 3]], material: &str) -> MeshData {
    let mut m = MeshData { name: material.into(), material: material.into(), ..Default::default() };
    for t in tris {
        let b = m.positions.len() as u32;
        m.positions.extend_from_slice(t);
        m.indices.extend_from_slice(&[b, b + 1, b + 2]);
    }
    m
}

/// Render `tris` with centre rays and compare first-hit distances.
/// Returns (pixels compared, mismatches).
fn check(name: &str, tris: Vec<[Vec3; 3]>, eye: Vec3, target: Vec3) -> (usize, usize) {
    let Ok((device, queue, _)) = lumencraft::headless::create_device() else {
        eprintln!("no adapter; skipping");
        return (0, 0);
    };
    let mut world = World::new();
    world.scene = Scene::default();
    // Split into several meshes so the scene has multiple materials/meshes.
    for (i, chunk) in tris.chunks(tris.len().div_ceil(4).max(1)).enumerate() {
        world.scene.upsert_mesh(format!("file:m{i}"), mesh(chunk, &format!("mat{i}")));
    }
    world.env.fog = 0.0;
    world.env.haze = 0.0;
    world.camera.position = eye.to_array();
    world.camera.look_at(target);
    world.camera.fov = 60.0;
    let mut fp = world.frame_params(W as f32 / H as f32, 1.0);
    fp.lens[0] = 0.0; // no depth of field
    let mut r = Renderer::new(device, queue);
    r.sync_scene(&world.scene);
    let targets = r.create_targets(W, H);
    let mut pt = PathTracer::new(&r.device);
    let opts = PtOpts { spp: 1, bounces: 0, denoise: false };
    let mut job = pt.begin(&mut r, &world.scene, &fp, &targets, opts);
    job.centre_rays = true;
    while !job.finished() {
        pt.step(&r, &mut job, std::time::Duration::from_secs(5));
    }
    let (_, _, _, depth) = pt.read_layers(&r, &mut job);

    // Same rays on the CPU (pixel centres, same un-projection as the shader).
    let inv: Mat4 = (fp.proj * fp.view).inverse();
    let mut bad = 0;
    let mut n = 0;
    for y in 0..H {
        for x in 0..W {
            let ndc = ((x as f32 + 0.5) / W as f32 * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / H as f32 * 2.0);
            let far = inv * Vec4::new(ndc.0, ndc.1, 0.5, 1.0);
            let near = inv * Vec4::new(ndc.0, ndc.1, 1.0, 1.0);
            let o = near.truncate() / near.w;
            let d = (far.truncate() / far.w - o).normalize();
            let cpu = tris.iter().filter_map(|t| ray_tri(o, d, t)).fold(f32::INFINITY, f32::min);
            let gpu = depth[(y * W + x) as usize];
            let gpu_hit = gpu < 9999.0;
            n += 1;
            let ok = if cpu.is_finite() {
                // A hit must be found, at the same distance (relative tolerance
                // for far geometry; tiny tolerance for grazing edge pixels).
                gpu_hit && (gpu - cpu).abs() <= 1e-3 * cpu.max(1.0)
            } else {
                !gpu_hit
            };
            if !ok {
                bad += 1;
                if bad <= 5 {
                    eprintln!("{name}: pixel ({x},{y}) cpu {cpu} gpu {gpu}");
                }
            }
        }
    }
    eprintln!("{name}: {n} pixels, {bad} mismatches, {} triangles", tris.len());
    (n, bad)
}

fn assert_ok(name: &str, (n, bad): (usize, usize)) {
    // Allow a handful of pixels whose centre ray grazes a shared edge, where
    // f32 rounding on GPU vs CPU can legitimately differ.
    assert!(bad <= n / 200, "{name}: {bad} of {n} pixels disagree with the CPU");
}

#[test]
fn dense_clutter_matches_cpu() {
    let mut seed = 0x1234567;
    let mut tris = Vec::new();
    for _ in 0..60_000 {
        let c = Vec3::new(rng(&mut seed) * 20.0 - 10.0, rng(&mut seed) * 10.0, rng(&mut seed) * 20.0 - 10.0);
        let s = 0.05 + rng(&mut seed) * 0.6;
        tris.push([c, c + Vec3::new(s, rng(&mut seed) * s, 0.0), c + Vec3::new(0.0, s, rng(&mut seed) * s)]);
    }
    assert_ok("dense", check("dense", tris, Vec3::new(0.0, 5.0, -25.0), Vec3::new(0.0, 5.0, 0.0)));
}

#[test]
fn mixed_scales_and_far_coordinates_match_cpu() {
    // A house-sized cluster of fine detail placed at survey-style
    // coordinates, a large ground and far mountains-scale triangles.
    let off = Vec3::new(4200.0, 0.0, -3100.0);
    let mut seed = 99;
    let mut tris = vec![
        [off + Vec3::new(-500.0, 0.0, -500.0), off + Vec3::new(500.0, 0.0, -500.0), off + Vec3::new(0.0, 0.0, 600.0)],
        [off + Vec3::new(-3000.0, 0.0, 2000.0), off + Vec3::new(3000.0, 0.0, 2000.0), off + Vec3::new(0.0, 900.0, 2500.0)],
    ];
    // geometric clusters: 2 mm .. 30 m
    for k in 0..40 {
        let scale = 0.002 * 1.28f32.powi(k);
        for _ in 0..300 {
            let c = off + Vec3::new(rng(&mut seed) * 8.0 - 4.0, 0.2 + rng(&mut seed) * 6.0, rng(&mut seed) * 8.0 - 4.0);
            tris.push([c, c + Vec3::new(scale, 0.0, 0.0), c + Vec3::new(0.0, scale, scale * 0.5)]);
        }
    }
    // stacked duplicates (overlapping copies, common in Revit exports)
    for _ in 0..500 {
        tris.push([off + Vec3::new(-2.0, 0.5, 3.0), off + Vec3::new(2.0, 0.5, 3.0), off + Vec3::new(0.0, 4.0, 3.0)]);
    }
    let eye = off + Vec3::new(0.0, 3.0, -12.0);
    assert_ok("mixed", check("mixed", tris, eye, off + Vec3::new(0.0, 2.0, 0.0)));
}
