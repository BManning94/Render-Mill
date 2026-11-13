//! Landscape templates: a start-of-project terrain with rolling hills, lakes
//! or sea, scattered trees, shrubs, rocks and wildflowers, plus matching sky
//! and sun - what Lumion's "Create new" page offers.
//!
//! Everything is procedural and deterministic (seeded), so a project only
//! stores the [`Landscape`] parameters, not the generated meshes.

use crate::env::Environment;
use crate::objects::{self, ObjectKind, PlacedObject};
use crate::scene::{Material, MaterialKind, MaterialParams, MeshData, Scene, TextureData};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

pub const MAT_WATER: &str = "Landscape - Water";
pub const MAT_FLOWERS: &str = "Landscape - Wildflowers";
pub const MAT_DRY_GRASS: &str = "Landscape - Tall grass";
pub const MAT_FAR_TREES: &str = "Landscape - Distant trees";

/// Beyond this distance trees are drawn as low-poly crowns.
const NEAR_TREES: f32 = 380.0;

/// A body of water: a lake (disc) or the sea (everything past a coastline).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Water {
    /// Centre (x, z) and radius in metres. For the sea the "radius" is huge
    /// and the centre far away, so the shore is almost straight.
    pub center: [f32; 2],
    pub radius: f32,
    /// Water surface height (m).
    pub level: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Landscape {
    /// Template id it was made from (for the UI).
    pub template: String,
    pub seed: u32,
    /// Half size of the terrain square (m); mountains start beyond it.
    pub extent: f32,
    /// Flat building plot radius around the origin (m).
    pub flat_radius: f32,
    /// Height of the rolling hills (m).
    pub hills: f32,
    /// Extra height of the foothills towards the edge (m).
    pub foothills: f32,
    pub water: Vec<Water>,
    /// Built-in library ground texture (e.g. "Grass004").
    pub ground: String,
    /// Plants per hectare (100 m x 100 m) away from the plot.
    pub trees: f32,
    pub conifers: f32,
    pub shrubs: f32,
    pub rocks: f32,
    /// Wildflower patches per hectare.
    pub flowers: f32,
    /// Tall dry-grass tufts per hectare.
    pub tufts: f32,
}

impl Landscape {
    /// Terrain height at (x, z).
    pub fn height(&self, x: f32, z: f32) -> f32 {
        let r = (x * x + z * z).sqrt();
        let open = smoothstep(self.flat_radius, self.flat_radius * 2.5 + 80.0, r);
        let s = self.seed;
        let rolling = (fbm(x / 420.0, z / 420.0, s, 4) - 0.35) * self.hills;
        let foot = smoothstep(350.0, self.extent * 0.95, r) * ridged(x / 650.0, z / 650.0, s + 5) * self.foothills;
        // Gentle undulation even on the plot edge, so it doesn't look like a table.
        let micro = (fbm(x / 60.0, z / 60.0, s + 9, 3) - 0.5) * 0.6 * open;
        let mut h = (rolling.max(-self.hills * 0.25) + foot) * open + micro;
        for w in &self.water {
            let d = ((x - w.center[0]).powi(2) + (z - w.center[1]).powi(2)).sqrt();
            // Irregular shoreline.
            let wobble = (fbm(x / 90.0, z / 90.0, s + 21, 3) - 0.5) * w.radius.min(300.0) * 0.5;
            let t = smoothstep(w.radius - 30.0, w.radius + 50.0, d + wobble);
            let bed = w.level - 3.0;
            h = bed + (h - bed) * t;
        }
        h
    }

    /// Height and steepness (rise per metre) at (x, z).
    fn slope(&self, x: f32, z: f32) -> f32 {
        let e = 2.0;
        let dx = self.height(x + e, z) - self.height(x - e, z);
        let dz = self.height(x, z + e) - self.height(x, z - e);
        (dx * dx + dz * dz).sqrt() / (2.0 * e)
    }

    fn under_water(&self, x: f32, z: f32, h: f32, margin: f32) -> bool {
        self.water.iter().any(|w| {
            h < w.level + margin && {
                let d = ((x - w.center[0]).powi(2) + (z - w.center[1]).powi(2)).sqrt();
                d < w.radius + 120.0
            }
        })
    }

    /// Stable key: regenerate meshes only when this changes.
    pub fn key(&self) -> u64 {
        let s = serde_json::to_string(self).unwrap_or_default();
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in s.bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
        h
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn hash(x: i32, z: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d) ^ (z as u32).wrapping_mul(0x1656_67b1) ^ seed.wrapping_mul(0x9e37_79b9);
    h = (h ^ (h >> 15)).wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    (h & 0xff_ffff) as f32 / 16_777_216.0
}

fn vnoise(x: f32, z: f32, seed: u32) -> f32 {
    let (xi, zi) = (x.floor() as i32, z.floor() as i32);
    let (fx, fz) = (x - x.floor(), z - z.floor());
    let (ux, uz) = (fx * fx * (3.0 - 2.0 * fx), fz * fz * (3.0 - 2.0 * fz));
    let a = hash(xi, zi, seed);
    let b = hash(xi + 1, zi, seed);
    let c = hash(xi, zi + 1, seed);
    let d = hash(xi + 1, zi + 1, seed);
    let top = a + (b - a) * ux;
    top + ((c + (d - c) * ux) - top) * uz
}

fn fbm(x: f32, z: f32, seed: u32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 0.5, 1.0, 0.0);
    for o in 0..octaves {
        sum += vnoise(x * freq, z * freq, seed + o * 17) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    sum / norm
}

fn ridged(x: f32, z: f32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut freq) = (0.0, 0.55, 1.0);
    for o in 0..4 {
        let n = 1.0 - (vnoise(x * freq, z * freq, seed + o * 31) * 2.0 - 1.0).abs();
        sum += n * n * amp;
        amp *= 0.5;
        freq *= 2.1;
    }
    sum
}

/// Small deterministic random generator.
struct Rng(u64);
impl Rng {
    fn f(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as u32) as f32 / 16_777_216.0
    }
}

/// Grid coordinate: fine near the plot, coarser towards the edge.
fn grid_coord(i: usize, n: usize, extent: f32) -> f32 {
    let u = i as f32 / n as f32 * 2.0 - 1.0;
    let a = 0.08;
    u * (a + (1.0 - a) * u.abs()) * extent
}

/// Terrain as square tiles (for culling and fast ray casts).
pub fn terrain_meshes(l: &Landscape) -> Vec<(String, MeshData)> {
    let n = 320usize; // cells per side
    let tiles = 8usize;
    let per = n / tiles;
    let coords: Vec<f32> = (0..=n).map(|i| grid_coord(i, n, l.extent)).collect();
    let mut hts = vec![0.0f32; (n + 1) * (n + 1)];
    for j in 0..=n {
        for i in 0..=n {
            hts[j * (n + 1) + i] = l.height(coords[i], coords[j]);
        }
    }
    let hat = |i: usize, j: usize| hts[j.min(n) * (n + 1) + i.min(n)];
    let normal = |i: usize, j: usize| {
        let (i0, i1) = (i.saturating_sub(1), (i + 1).min(n));
        let (j0, j1) = (j.saturating_sub(1), (j + 1).min(n));
        let dx = (hat(i1, j) - hat(i0, j)) / (coords[i1] - coords[i0]).max(1e-3);
        let dz = (hat(i, j1) - hat(i, j0)) / (coords[j1] - coords[j0]).max(1e-3);
        Vec3::new(-dx, 1.0, -dz).normalize()
    };
    let mut out = Vec::new();
    for tj in 0..tiles {
        for ti in 0..tiles {
            let mut m = MeshData { name: "Terrain".into(), material: objects::MAT_GROUND.into(), ..Default::default() };
            for j in tj * per..=(tj + 1) * per {
                for i in ti * per..=(ti + 1) * per {
                    let (x, z) = (coords[i], coords[j]);
                    m.positions.push(Vec3::new(x, hat(i, j), z));
                    m.normals.push(normal(i, j));
                    m.uvs.push(Vec2::new(x, z));
                }
            }
            let w = (per + 1) as u32;
            for j in 0..per as u32 {
                for i in 0..per as u32 {
                    let a = j * w + i;
                    m.indices.extend_from_slice(&[a, a + w, a + 1, a + 1, a + w, a + w + 1]);
                }
            }
            out.push((format!("env:terrain:{ti}:{tj}"), m));
        }
    }
    out
}

/// Water surfaces (flat discs).
pub fn water_meshes(l: &Landscape) -> Vec<(String, MeshData)> {
    let mut out = Vec::new();
    for (k, w) in l.water.iter().enumerate() {
        let mut m = MeshData { name: "Water".into(), material: MAT_WATER.into(), ..Default::default() };
        let r = (w.radius * 1.5 + 80.0).min(l.extent * 3.0);
        let (segs, rings) = (96u32, 24u32);
        m.positions.push(Vec3::new(w.center[0], w.level, w.center[1]));
        m.normals.push(Vec3::Y);
        m.uvs.push(Vec2::new(w.center[0], w.center[1]));
        for j in 1..=rings {
            let rr = r * (j as f32 / rings as f32).powf(1.5);
            for i in 0..segs {
                let a = i as f32 / segs as f32 * std::f32::consts::TAU;
                let (x, z) = (w.center[0] + a.cos() * rr, w.center[1] + a.sin() * rr);
                m.positions.push(Vec3::new(x, w.level, z));
                m.normals.push(Vec3::Y);
                m.uvs.push(Vec2::new(x, z));
            }
        }
        for i in 0..segs {
            m.indices.extend_from_slice(&[0, 1 + (i + 1) % segs, 1 + i]);
        }
        for j in 0..rings - 1 {
            let (a0, b0) = (1 + j * segs, 1 + (j + 1) * segs);
            for i in 0..segs {
                let i1 = (i + 1) % segs;
                m.indices.extend_from_slice(&[a0 + i, a0 + i1, b0 + i, a0 + i1, b0 + i1, b0 + i]);
            }
        }
        out.push((format!("env:water:{k}"), m));
    }
    out
}

/// Trees, shrubs, rocks, flowers and tall grass, merged per 200 m tile and
/// material (thousands of plants as a few dozen draw calls).
pub fn vegetation_meshes(l: &Landscape) -> Vec<(String, MeshData)> {
    let tile = 200.0f32;
    let nt = (l.extent * 2.0 / tile).ceil() as i32;
    let mut out = Vec::new();
    let mut groups: std::collections::HashMap<((i32, i32, u8), String), MeshData> = std::collections::HashMap::new();
    let clear = l.flat_radius * 0.9;
    for tj in 0..nt {
        for ti in 0..nt {
            let x0 = -l.extent + ti as f32 * tile;
            let z0 = -l.extent + tj as f32 * tile;
            let mut rng = Rng(((ti as u64) << 32 | tj as u64) ^ (l.seed as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0x5bd1e995);
            let mut by_mat: Vec<(String, MeshData)> = Vec::new();
            let mut add = |parts: Vec<MeshData>| {
                for p in parts {
                    let slot = match by_mat.iter().position(|(k, _)| *k == p.material) {
                        Some(s) => s,
                        None => {
                            by_mat.push((p.material.clone(), MeshData { name: "Vegetation".into(), material: p.material.clone(), ..Default::default() }));
                            by_mat.len() - 1
                        }
                    };
                    let m = &mut by_mat[slot].1;
                    let base = m.positions.len() as u32;
                    m.positions.extend_from_slice(&p.positions);
                    m.normals.extend_from_slice(&p.normals);
                    m.uvs.extend_from_slice(&p.uvs);
                    m.indices.extend(p.indices.iter().map(|i| i + base));
                }
            };
            let area_ha = tile * tile / 10_000.0;
            let kinds: [(ObjectKind, f32); 4] =
                [(ObjectKind::Tree, l.trees), (ObjectKind::Conifer, l.conifers), (ObjectKind::Shrub, l.shrubs), (ObjectKind::Boulder, l.rocks)];
            for (kind, density) in kinds {
                let count = density * area_ha;
                let n = count.floor() as u32 + (rng.f() < count.fract()) as u32;
                for _ in 0..n {
                    let (x, z) = (x0 + rng.f() * tile, z0 + rng.f() * tile);
                    let (rot, sc, seed, keep) = (rng.f() * 360.0, rng.f(), (rng.f() * 1e6) as u32, rng.f());
                    let r = (x * x + z * z).sqrt();
                    // Keep the plot and the opening view (see start_camera) clear.
                    if r < clear || r > l.extent * 0.97 || Vec2::new(x - 20.0, z - 70.0).length() < 45.0 {
                        continue;
                    }
                    // Clumps: forests where a low-frequency noise is high.
                    let clump = fbm(x / 260.0, z / 260.0, l.seed + 77, 3);
                    let want = match kind {
                        ObjectKind::Conifer => smoothstep(0.35, 0.6, clump) + smoothstep(8.0, 60.0, l.height(x, z)) * 0.5,
                        ObjectKind::Tree => smoothstep(0.3, 0.55, clump),
                        _ => 0.6 + clump * 0.4,
                    };
                    if keep > want {
                        continue;
                    }
                    let h = l.height(x, z);
                    if l.under_water(x, z, h, 0.6) || (kind != ObjectKind::Boulder && l.slope(x, z) > 0.55) {
                        continue;
                    }
                    let scale = match kind {
                        ObjectKind::Boulder => 0.5 + sc * 1.6,
                        ObjectKind::Shrub => 0.7 + sc * 0.8,
                        _ => 0.75 + sc * 0.7,
                    };
                    if r > NEAR_TREES && matches!(kind, ObjectKind::Tree | ObjectKind::Conifer) {
                        // Far away: fewer, low-poly trees (the forest reads as a whole).
                        if rng.f() > 1.0 - 0.5 * smoothstep(NEAR_TREES, l.extent, r) {
                            continue;
                        }
                        add(vec![far_tree(Vec3::new(x, h, z), scale, kind == ObjectKind::Conifer, seed)]);
                        continue;
                    }
                    if r > NEAR_TREES * if kind == ObjectKind::Boulder { 2.5 } else { 1.6 } {
                        continue; // shrubs and rocks aren't visible that far
                    }
                    let o = PlacedObject::new(kind, [x, h - 0.05, z], rot, scale, seed);
                    add(objects::build(&o));
                }
            }
            // Wildflower patches and tall grass tufts.
            for (count_ha, flowers) in [(l.flowers, true), (l.tufts, false)] {
                let count = count_ha * area_ha;
                let n = count.floor() as u32 + (rng.f() < count.fract()) as u32;
                for _ in 0..n {
                    let (x, z) = (x0 + rng.f() * tile, z0 + rng.f() * tile);
                    let (seed, keep) = ((rng.f() * 1e6) as u32, rng.f());
                    let r = (x * x + z * z).sqrt();
                    // Near the plot only (they're small); keep the plot itself clear.
                    if r < l.flat_radius * 0.55 || r > 650.0 || keep > 1.0 - smoothstep(250.0, 650.0, r) {
                        continue;
                    }
                    let h = l.height(x, z);
                    if l.under_water(x, z, h, 0.3) || l.slope(x, z) > 0.6 {
                        continue;
                    }
                    add(vec![patch_mesh(l, x, z, seed, flowers)]);
                }
            }
            // Near tiles stay 200 m (fine culling); far ones are merged 3 x 3
            // into 600 m blocks so the whole landscape is a few hundred draws.
            let near = Vec2::new(x0 + tile * 0.5, z0 + tile * 0.5).length() < 550.0;
            let group = if near { (ti, tj, 0) } else { (ti / 3, tj / 3, 1) };
            for (_, m) in by_mat {
                if m.indices.is_empty() {
                    continue;
                }
                let slot = groups.entry((group, m.material.clone())).or_insert_with(|| MeshData {
                    name: "Vegetation".into(),
                    material: m.material.clone(),
                    ..Default::default()
                });
                let base = slot.positions.len() as u32;
                slot.positions.extend_from_slice(&m.positions);
                slot.normals.extend_from_slice(&m.normals);
                slot.uvs.extend_from_slice(&m.uvs);
                slot.indices.extend(m.indices.iter().map(|i| i + base));
            }
        }
    }
    let mut keys: Vec<_> = groups.keys().cloned().collect();
    keys.sort();
    for (k, key) in keys.into_iter().enumerate() {
        let m = groups.remove(&key).unwrap();
        let ((gx, gz, far), _) = key;
        out.push((format!("env:veg:{far}:{gx}:{gz}:{k}"), m));
    }
    out
}

/// Low-poly distant tree: a cone (conifer) or a lumpy ball on a short
/// trunk, ~30 triangles.
fn far_tree(p: Vec3, scale: f32, conifer: bool, seed: u32) -> MeshData {
    let mut m = MeshData { name: "Distant tree".into(), material: MAT_FAR_TREES.into(), ..Default::default() };
    let mut rng = Rng(seed as u64 * 7919 + 3);
    let segs = 7u32;
    let (h, r, base_y) = if conifer { (11.0 * scale, 2.4 * scale, 1.0 * scale) } else { (8.0 * scale, 3.6 * scale, 2.2 * scale) };
    // Rings: (height fraction, radius fraction)
    let rings: &[(f32, f32)] = if conifer { &[(0.0, 1.0), (0.45, 0.6), (1.0, 0.0)] } else { &[(0.0, 0.55), (0.3, 1.0), (0.7, 0.85), (1.0, 0.0)] };
    let phase = rng.f() * std::f32::consts::TAU;
    for &(fh, fr) in rings {
        for i in 0..segs {
            let a = phase + i as f32 / segs as f32 * std::f32::consts::TAU;
            let jitter = 0.85 + rng.f() * 0.3;
            let off = Vec3::new(a.cos() * r * fr * jitter, base_y + h * fh, a.sin() * r * fr * jitter);
            m.positions.push(p + off);
            m.normals.push(Vec3::new(a.cos(), 0.6 - fh * 0.3, a.sin()).normalize());
            m.uvs.push(Vec2::new(i as f32 / segs as f32, fh));
        }
    }
    for k in 0..rings.len() as u32 - 1 {
        let (a0, b0) = (k * segs, (k + 1) * segs);
        for i in 0..segs {
            let i1 = (i + 1) % segs;
            m.indices.extend_from_slice(&[a0 + i, b0 + i, a0 + i1, a0 + i1, b0 + i, b0 + i1]);
        }
    }
    m
}

/// A patch of crossed cards: wildflowers (atlas column picks the colour) or
/// tall grass tufts.
fn patch_mesh(l: &Landscape, cx: f32, cz: f32, seed: u32, flowers: bool) -> MeshData {
    let mut rng = Rng(seed as u64 * 2654435761 + 1);
    let mut m = MeshData {
        name: if flowers { "Wildflowers" } else { "Tall grass" }.into(),
        material: if flowers { MAT_FLOWERS } else { MAT_DRY_GRASS }.into(),
        ..Default::default()
    };
    let col = (rng.f() * 4.0).floor().min(3.0);
    let cards = if flowers { 10 } else { 6 };
    let spread = if flowers { 2.2 } else { 1.2 };
    for _ in 0..cards {
        let (x, z) = (cx + (rng.f() - 0.5) * spread * 2.0, cz + (rng.f() - 0.5) * spread * 2.0);
        let y = l.height(x, z) - 0.03;
        let a = rng.f() * std::f32::consts::PI;
        let hgt = if flowers { 0.45 + rng.f() * 0.45 } else { 0.4 + rng.f() * 0.4 };
        let wdt = hgt * if flowers { 0.6 } else { 0.9 };
        // Flowers: occasionally a different colour in the patch.
        let c = if flowers && rng.f() < 0.2 { (rng.f() * 4.0).floor().min(3.0) } else { col };
        let (u0, u1) = if flowers { (c / 4.0, (c + 1.0) / 4.0) } else { (0.0, 1.0) };
        for k in 0..2 {
            let ang = a + k as f32 * std::f32::consts::FRAC_PI_2;
            let d = Vec3::new(ang.cos(), 0.0, ang.sin()) * wdt * 0.5;
            let base = m.positions.len() as u32;
            let p = Vec3::new(x, y, z);
            let n = Vec3::new(-d.z, 0.4, d.x).normalize();
            for (pos, uv) in [
                (p - d, Vec2::new(u0, 1.0)),
                (p + d, Vec2::new(u1, 1.0)),
                (p + d + Vec3::Y * hgt, Vec2::new(u1, 0.0)),
                (p - d + Vec3::Y * hgt, Vec2::new(u0, 0.0)),
            ] {
                m.positions.push(pos);
                m.normals.push(n);
                m.uvs.push(uv);
            }
            m.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    m
}

/// Wildflower atlas: four columns (yellow, purple, white, orange-red), each
/// a cluster of stems with blossoms on a transparent background.
pub fn flower_texture() -> TextureData {
    let (w, h) = (512usize, 256usize);
    let mut img = vec![0u8; w * h * 4];
    let mut rng = Rng(4242);
    let mut put = |x: i32, y: i32, c: [f32; 3]| {
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            return;
        }
        let i = (y as usize * w + x as usize) * 4;
        for k in 0..3 {
            img[i + k] = (c[k].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8;
        }
        img[i + 3] = 255;
    };
    let colours = [[0.95, 0.72, 0.05], [0.55, 0.25, 0.75], [0.92, 0.92, 0.88], [0.95, 0.35, 0.08]];
    for (col, petal) in colours.iter().enumerate() {
        let x_lo = (col * 128) as f32;
        for _ in 0..14 {
            let bx = x_lo + 14.0 + rng.f() * 100.0;
            let top = 30.0 + rng.f() * 120.0;
            let lean = (rng.f() - 0.5) * 18.0;
            // Stem with a couple of leaves.
            let steps = (h as f32 - top) as i32;
            for s in 0..steps {
                let t = s as f32 / steps as f32;
                let x = bx + lean * (1.0 - t);
                let y = top + s as f32;
                let g = [0.08 + 0.05 * t, 0.22 + 0.08 * t, 0.04];
                put(x as i32, y as i32, g);
                put(x as i32 + 1, y as i32, g);
                if s % 37 == 20 {
                    for k in 0..9 {
                        let side = if (s / 37) % 2 == 0 { 1 } else { -1 };
                        put(x as i32 + side * k, y as i32 - k / 2, [0.09, 0.25, 0.05]);
                        put(x as i32 + side * k, y as i32 - k / 2 + 1, [0.09, 0.25, 0.05]);
                    }
                }
            }
            // Blossom: petals around a centre.
            let r = 5.0 + rng.f() * 6.0;
            let shade = 0.8 + rng.f() * 0.25;
            let fx = bx + lean;
            for dy in -(r as i32 + 1)..=(r as i32 + 1) {
                for dx in -(r as i32 + 1)..=(r as i32 + 1) {
                    let d = ((dx * dx + dy * dy) as f32).sqrt();
                    let ang = (dy as f32).atan2(dx as f32);
                    let petal_r = r * (0.75 + 0.25 * (ang * 6.0).cos().abs());
                    if d <= petal_r {
                        let c = if d < r * 0.3 {
                            if col == 0 {
                                [0.25, 0.12, 0.03]
                            } else {
                                [0.9, 0.75, 0.1]
                            }
                        } else {
                            [petal[0] * shade, petal[1] * shade, petal[2] * shade]
                        };
                        put(fx as i32 + dx, top as i32 + dy, c);
                    }
                }
            }
        }
    }
    TextureData::new(w as u32, h as u32, img)
}

/// Tall grass tuft card: many thin blades, straw to green.
pub fn tuft_texture() -> TextureData {
    let (w, h) = (256usize, 256usize);
    let mut img = vec![0u8; w * h * 4];
    let mut rng = Rng(777);
    for _ in 0..70 {
        let bx = 40.0 + rng.f() * 176.0;
        let top = 20.0 + rng.f() * 190.0;
        // Blades fan out from the clump centre and bend over.
        let lean = (bx - 128.0) * 0.9 + (rng.f() - 0.5) * 50.0;
        let g = rng.f();
        let c = [0.30 + 0.30 * g, 0.32 + 0.12 * g, 0.08 + 0.06 * g];
        let steps = (h as f32 - top) as i32;
        for s in 0..steps {
            let t = s as f32 / steps as f32; // 0 at the tip, 1 at the root
            let x = 128.0 + (bx - 128.0) * (1.0 - t) * 0.3 + lean * (1.0 - t) * (1.0 - t);
            let y = top + s as f32;
            let width = 1 + (t * 1.5) as i32;
            for k in 0..width {
                let xi = x as i32 + k;
                if xi >= 0 && (xi as usize) < w {
                    let i = (y as usize * w + xi as usize) * 4;
                    let sh = 0.55 + 0.45 * (1.0 - t * 0.6);
                    for (ch, v) in c.iter().enumerate() {
                        img[i + ch] = ((v * sh).clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8;
                    }
                    img[i + 3] = 255;
                }
            }
        }
    }
    TextureData::new(w as u32, h as u32, img)
}

/// Materials the landscape uses (registered once).
pub fn register_materials(scene: &mut Scene) {
    let d = MaterialParams::default();
    if !scene.imported_materials.contains_key(MAT_FAR_TREES) {
        let p = MaterialParams { kind: MaterialKind::Foliage, base_color: [0.05, 0.10, 0.035, 1.0], roughness: 0.85, variation: 0.8, ..d.clone() };
        scene.set_material(Material { name: MAT_FAR_TREES.into(), params: p, texture: None, normal_texture: None, roughness_texture: None });
    }
    if !scene.imported_materials.contains_key(MAT_WATER) {
        let p = MaterialParams { kind: MaterialKind::Water, base_color: [0.02, 0.06, 0.07, 1.0], roughness: 0.05, ..d.clone() };
        scene.set_material(Material { name: MAT_WATER.into(), params: p, texture: None, normal_texture: None, roughness_texture: None });
    }
    for (name, tex) in [(MAT_FLOWERS, flower_texture as fn() -> TextureData), (MAT_DRY_GRASS, tuft_texture)] {
        if !scene.imported_materials.contains_key(name) {
            let p = MaterialParams {
                kind: MaterialKind::Foliage,
                base_color: [1.0, 1.0, 1.0, 1.0],
                roughness: 0.8,
                cutout: true,
                use_texture: true,
                uv_scale: 1.0,
                variation: 0.3,
                ..d.clone()
            };
            scene.set_material(Material { name: name.into(), params: p, texture: Some(tex()), normal_texture: None, roughness_texture: None });
        }
    }
}

/// Build (or rebuild) every landscape mesh in the scene.
pub fn apply(scene: &mut Scene, l: Option<&Landscape>) {
    scene.remove_prefix("env:terrain:");
    scene.remove_prefix("env:water:");
    scene.remove_prefix("env:veg:");
    let Some(l) = l else { return };
    register_materials(scene);
    objects::register_library_materials(scene);
    for (id, m) in terrain_meshes(l).into_iter().chain(water_meshes(l)).chain(vegetation_meshes(l)) {
        scene.upsert_mesh(id, m);
    }
}

/// A "Create new project" template.
pub struct Template {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

pub const TEMPLATES: [Template; 9] = [
    Template { id: "meadow", name: "Green meadow", description: "Rolling grassland, scattered trees and wildflowers" },
    Template { id: "valley", name: "Mountain valley", description: "Dry meadow ringed by forested foothills" },
    Template { id: "lakeside", name: "Lakeside", description: "Plot beside a lake, pines along the shore" },
    Template { id: "forest", name: "Forest clearing", description: "Plot cut into dense mixed forest" },
    Template { id: "coast", name: "Coast", description: "Sandy grass by the sea" },
    Template { id: "desert", name: "Desert", description: "Sand, rocks and shrubs under a clear sky" },
    Template { id: "winter", name: "Winter", description: "Snow, conifers and a low sun" },
    Template { id: "evening", name: "Golden hour", description: "Meadow at sunset" },
    Template { id: "flat", name: "Flat lawn", description: "Plain lawn and backdrop (classic LumenCraft)" },
];

/// Landscape and environment for a template id.
pub fn template(id: &str) -> Option<(Option<Landscape>, Environment)> {
    let mut env = Environment::default();
    let base = Landscape {
        template: id.into(),
        seed: 11,
        extent: 1800.0,
        flat_radius: 85.0,
        hills: 22.0,
        foothills: 160.0,
        water: Vec::new(),
        ground: "Grass004".into(),
        trees: 4.0,
        conifers: 1.0,
        shrubs: 3.0,
        rocks: 0.6,
        flowers: 14.0,
        tufts: 0.0,
    };
    let l = match id {
        "meadow" => {
            env.cloudiness = 0.45;
            Landscape { ..base }
        }
        "valley" => {
            env.sun_heading = 200.0;
            env.sun_height = 42.0;
            env.cloudiness = 0.5;
            env.haze = 0.3;
            env.grass_height = 0.2;
            Landscape {
                seed: 23,
                ground: "Ground003".into(),
                hills: 18.0,
                foothills: 320.0,
                trees: 0.6,
                conifers: 5.0,
                shrubs: 2.0,
                flowers: 18.0,
                tufts: 30.0,
                ..base
            }
        }
        "lakeside" => {
            env.sun_heading = 240.0;
            env.cloudiness = 0.3;
            Landscape {
                seed: 5,
                water: vec![Water { center: [-40.0, -330.0], radius: 210.0, level: -0.8 }],
                hills: 20.0,
                foothills: 260.0,
                trees: 1.5,
                conifers: 7.0,
                flowers: 10.0,
                ..base
            }
        }
        "forest" => {
            env.cloudiness = 0.25;
            env.haze = 0.2;
            Landscape { seed: 31, ground: "Ground037".into(), flat_radius: 70.0, hills: 12.0, trees: 16.0, conifers: 20.0, shrubs: 10.0, flowers: 4.0, ..base }
        }
        "coast" => {
            env.sun_heading = 160.0;
            env.cloudiness = 0.3;
            env.haze = 0.35;
            Landscape {
                seed: 9,
                ground: "Grass005".into(),
                water: vec![Water { center: [0.0, -6150.0], radius: 6000.0, level: -1.2 }],
                hills: 12.0,
                foothills: 120.0,
                trees: 1.0,
                conifers: 1.5,
                shrubs: 4.0,
                rocks: 1.5,
                flowers: 6.0,
                tufts: 25.0,
                ..base
            }
        }
        "desert" => {
            env.sun_height = 55.0;
            env.cloudiness = 0.05;
            env.haze = 0.45;
            env.grass = false;
            Landscape {
                seed: 17,
                ground: "Ground033".into(),
                hills: 30.0,
                foothills: 200.0,
                trees: 0.0,
                conifers: 0.0,
                shrubs: 2.5,
                rocks: 4.0,
                flowers: 0.0,
                tufts: 8.0,
                ..base
            }
        }
        "winter" => {
            env.sun_height = 14.0;
            env.sun_heading = 190.0;
            env.cloudiness = 0.6;
            env.haze = 0.4;
            env.grass = false;
            Landscape {
                seed: 41,
                ground: "Snow006".into(),
                hills: 24.0,
                foothills: 340.0,
                trees: 0.0,
                conifers: 9.0,
                shrubs: 1.0,
                rocks: 1.0,
                flowers: 0.0,
                ..base
            }
        }
        "evening" => {
            env.sun_height = 6.0;
            env.sun_heading = 260.0;
            env.cloudiness = 0.4;
            env.haze = 0.45;
            Landscape { seed: 3, ..base }
        }
        "flat" => return Some((None, env)),
        _ => return None,
    };
    Some((Some(l), env))
}

/// Where to put the camera so a new project opens with a nice view of the
/// plot and landscape.
pub fn start_camera(l: Option<&Landscape>) -> crate::env::Camera {
    let mut c = crate::env::Camera::default();
    // Raised a little and looking across the plot towards the water (lakes
    // and the sea lie to the north, -Z), like Lumion's opening view.
    let h = l.map_or(0.0, |l| l.height(20.0, 70.0));
    c.position = [20.0, h + 14.0, 70.0];
    c.look_at(Vec3::new(0.0, 0.0, -140.0));
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_are_flat_on_the_plot_and_dry_there() {
        for t in &TEMPLATES {
            let (l, _) = template(t.id).unwrap();
            let Some(l) = l else { continue };
            for (x, z) in [(0.0, 0.0), (30.0, -20.0), (-50.0, 40.0)] {
                let h = l.height(x, z);
                assert!(h.abs() < 0.8, "{}: plot not flat at ({x},{z}): {h}", t.id);
                assert!(!l.under_water(x, z, h, 0.0), "{}: plot under water", t.id);
            }
        }
    }

    #[test]
    fn old_projects_load_and_landscape_round_trips() {
        // A 0.12 environment has no "landscape" key.
        let mut v = serde_json::to_value(Environment::default()).unwrap();
        v.as_object_mut().unwrap().remove("landscape");
        let e: Environment = serde_json::from_value(v).unwrap();
        assert!(e.landscape.is_none());
        let (l, mut env) = template("lakeside").unwrap();
        env.landscape = l;
        let back: Environment = serde_json::from_str(&serde_json::to_string(&env).unwrap()).unwrap();
        assert_eq!(back.landscape, env.landscape);
        assert_eq!(back.landscape.as_ref().unwrap().key(), env.landscape.as_ref().unwrap().key());
    }

    #[test]
    fn vegetation_avoids_plot_and_water() {
        let (l, _) = template("lakeside").unwrap();
        let l = l.unwrap();
        let veg = vegetation_meshes(&l);
        let tris: usize = veg.iter().map(|(_, m)| m.indices.len() / 3).sum();
        assert!(tris > 10_000, "{tris}");
        for (_, m) in &veg {
            if m.material == MAT_FLOWERS || m.material == MAT_DRY_GRASS {
                continue;
            }
            let (lo, hi) = m.bounds().unwrap();
            let _ = (lo, hi);
            for p in m.positions.iter().step_by(97) {
                let r = (p.x * p.x + p.z * p.z).sqrt();
                assert!(r > l.flat_radius * 0.9 - 12.0, "plant on the plot at {p}");
            }
        }
        let (forest, _) = template("forest").unwrap();
        let n: usize = vegetation_meshes(&forest.unwrap()).iter().map(|(_, m)| m.indices.len() / 3).sum();
        eprintln!("lakeside {tris} tris, forest {n} tris");
        assert!(n < 3_000_000, "forest too heavy: {n} tris");
        for t in &TEMPLATES {
            if let (Some(l), _) = template(t.id).unwrap() {
                let v: usize = vegetation_meshes(&l).iter().map(|(_, m)| m.indices.len() / 3).sum();
                eprintln!("{}: {v} vegetation tris", t.id);
                assert!(v < 3_000_000);
            }
        }
    }
}
