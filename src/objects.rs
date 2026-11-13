// Plant generation uses `6.28` as its random-angle range; keeping it (instead
// of TAU) keeps the trees in existing projects exactly as they were.
#![allow(clippy::approx_constant)]

//! Object library: procedural trees, shrubs, people and lamps you can drop
//! into the scene (Lumion's "Objects" mode, with built-in procedural assets).

use crate::scene::{Material, MaterialKind, MaterialParams, MeshData, Scene};
use glam::{Mat3, Vec2, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObjectKind {
    Tree,
    Conifer,
    Shrub,
    Person,
    StreetLamp,
    Boulder,
    PointLight,
    SpotLight,
    Car,
    ArmChair,
    Sofa,
    Table,
    Chair,
    Bench,
    Planter,
    /// A 3D model file (built-in CC0 library or imported by the user); see `PlacedObject::model`.
    Model,
}

impl ObjectKind {
    pub const ALL: [ObjectKind; 15] = [
        ObjectKind::Tree,
        ObjectKind::Conifer,
        ObjectKind::Shrub,
        ObjectKind::Person,
        ObjectKind::StreetLamp,
        ObjectKind::Boulder,
        ObjectKind::PointLight,
        ObjectKind::SpotLight,
        ObjectKind::Car,
        ObjectKind::ArmChair,
        ObjectKind::Sofa,
        ObjectKind::Table,
        ObjectKind::Chair,
        ObjectKind::Bench,
        ObjectKind::Planter,
    ];
    /// Search tags shown on the library's preview card.
    pub fn tags(self) -> &'static [&'static str] {
        match self {
            ObjectKind::Tree => &["tree", "deciduous", "nature"],
            ObjectKind::Conifer => &["tree", "pine", "conifer"],
            ObjectKind::Shrub => &["shrub", "bush", "plant"],
            ObjectKind::Person => &["person", "people", "standing"],
            ObjectKind::StreetLamp => &["lamp", "street", "light"],
            ObjectKind::Boulder => &["rock", "stone", "boulder"],
            ObjectKind::PointLight => &["light", "point", "omni"],
            ObjectKind::SpotLight => &["light", "spot", "cone"],
            ObjectKind::Car => &["car", "vehicle", "transport"],
            ObjectKind::ArmChair => &["armchair", "seating", "interior"],
            ObjectKind::Sofa => &["sofa", "couch", "seating"],
            ObjectKind::Table => &["table", "dining", "interior"],
            ObjectKind::Chair => &["chair", "seating", "dining"],
            ObjectKind::Bench => &["bench", "park", "seating"],
            ObjectKind::Planter => &["planter", "pot", "plant"],
            ObjectKind::Model => &["model"],
        }
    }
    pub fn is_light(self) -> bool {
        matches!(self, ObjectKind::PointLight | ObjectKind::SpotLight)
    }
    pub fn label(self) -> &'static str {
        match self {
            ObjectKind::Tree => "Deciduous tree",
            ObjectKind::Conifer => "Conifer",
            ObjectKind::Shrub => "Shrub",
            ObjectKind::Person => "Person",
            ObjectKind::StreetLamp => "Street lamp",
            ObjectKind::Boulder => "Boulder",
            ObjectKind::PointLight => "Point light",
            ObjectKind::SpotLight => "Spot light",
            ObjectKind::Car => "Car",
            ObjectKind::ArmChair => "Armchair",
            ObjectKind::Sofa => "Sofa",
            ObjectKind::Table => "Dining table",
            ObjectKind::Chair => "Chair",
            ObjectKind::Bench => "Park bench",
            ObjectKind::Planter => "Planter",
            ObjectKind::Model => "3D model",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PlacedObject {
    pub kind: ObjectKind,
    pub position: [f32; 3],
    pub rotation: f32,
    pub scale: f32,
    pub seed: u32,
    /// Light colour (lights and street lamps).
    #[serde(default = "warm_white")]
    pub color: [f32; 3],
    /// Light brightness; 0 turns a street lamp's light off.
    #[serde(default = "default_intensity")]
    pub intensity: f32,
    /// Light reach in metres.
    #[serde(default = "default_range")]
    pub range: f32,
    /// Spot cone angle in degrees.
    #[serde(default = "default_cone")]
    pub cone: f32,
    /// For `ObjectKind::Model`: "builtin:<Category>/<id>" or a model file path.
    #[serde(default)]
    pub model: Option<String>,
}

fn warm_white() -> [f32; 3] {
    [1.0, 0.82, 0.62]
}
fn default_intensity() -> f32 {
    40.0
}
fn default_range() -> f32 {
    18.0
}
fn default_cone() -> f32 {
    70.0
}

impl PlacedObject {
    pub fn new(kind: ObjectKind, position: [f32; 3], rotation: f32, scale: f32, seed: u32) -> Self {
        Self {
            kind,
            position,
            rotation,
            scale,
            seed,
            color: warm_white(),
            intensity: default_intensity(),
            range: default_range(),
            cone: if kind == ObjectKind::StreetLamp { 120.0 } else { default_cone() },
            model: None,
        }
    }

    /// The light this object emits, if any.
    pub fn light(&self) -> Option<crate::renderer::LightU> {
        use crate::renderer::LightU;
        let p = Vec3::from(self.position);
        let c = Vec3::from(self.color);
        match self.kind {
            ObjectKind::PointLight => Some(LightU::point(p, c, self.intensity, self.range)),
            ObjectKind::SpotLight => Some(LightU::spot(p, -Vec3::Y, c, self.intensity, self.range, self.cone)),
            ObjectKind::StreetLamp if self.intensity > 0.0 => {
                Some(LightU::spot(p + Vec3::Y * 4.0 * self.scale, -Vec3::Y, c, self.intensity, self.range * self.scale, self.cone))
            }
            _ => None,
        }
    }
}

pub const MAT_BARK: &str = "Library - Bark";
pub const MAT_LEAVES: &str = "Library - Leaves";
pub const MAT_CONIFER: &str = "Library - Conifer needles";
pub const MAT_PERSON: &str = "Library - People";
pub const MAT_LAMP_POST: &str = "Library - Lamp post";
pub const MAT_LAMP_LIGHT: &str = "Library - Lamp light";
pub const MAT_STONE: &str = "Library - Stone";
pub const MAT_GROUND: &str = "Landscape - Ground";
pub const MAT_MOUNTAINS: &str = "Landscape - Mountains";
pub const MAT_GRASS: &str = "Landscape - Grass";
pub const MAT_CAR_PAINT: [&str; 5] =
    ["Library - Car paint white", "Library - Car paint black", "Library - Car paint red", "Library - Car paint blue", "Library - Car paint silver"];
pub const MAT_CAR_GLASS: &str = "Library - Car glass";
pub const MAT_TIRE: &str = "Library - Rubber";
pub const MAT_FABRIC: &str = "Library - Fabric";
pub const MAT_WOOD: &str = "Library - Furniture wood";
pub const MAT_METAL: &str = "Library - Furniture metal";
pub const MAT_POT: &str = "Library - Concrete planter";
pub const MAT_CHROME: &str = "Library - Chrome";

pub fn register_library_materials(scene: &mut Scene) {
    let d = MaterialParams::default();
    let defs = [
        (MAT_BARK, MaterialParams { base_color: [0.16, 0.11, 0.07, 1.0], roughness: 0.9, variation: 0.6, ..d.clone() }),
        (MAT_LEAVES, MaterialParams { kind: MaterialKind::Foliage, base_color: [0.10, 0.22, 0.04, 1.0], roughness: 0.7, variation: 0.8, ..d.clone() }),
        (MAT_CONIFER, MaterialParams { kind: MaterialKind::Foliage, base_color: [0.04, 0.12, 0.05, 1.0], roughness: 0.8, variation: 0.6, ..d.clone() }),
        (MAT_PERSON, MaterialParams { base_color: [0.35, 0.36, 0.40, 1.0], roughness: 0.7, ..d.clone() }),
        (MAT_LAMP_POST, MaterialParams { base_color: [0.04, 0.04, 0.045, 1.0], metallic: 1.0, roughness: 0.4, ..d.clone() }),
        (
            MAT_LAMP_LIGHT,
            MaterialParams { kind: MaterialKind::Emissive, base_color: [1.0, 0.9, 0.7, 1.0], emissive: [1.0, 0.78, 0.5], emissive_strength: 12.0, ..d.clone() },
        ),
        (MAT_STONE, MaterialParams { base_color: [0.32, 0.31, 0.29, 1.0], roughness: 0.85, variation: 0.7, ..d.clone() }),
        (MAT_GROUND, MaterialParams { kind: MaterialKind::Ground, base_color: [0.075, 0.13, 0.035, 1.0], roughness: 0.92, variation: 0.7, ..d.clone() }),
        (MAT_MOUNTAINS, MaterialParams { kind: MaterialKind::Terrain, base_color: [0.30, 0.29, 0.27, 1.0], roughness: 0.9, variation: 1.0, ..d.clone() }),
        (
            MAT_GRASS,
            MaterialParams {
                kind: MaterialKind::Foliage,
                base_color: [0.085, 0.15, 0.035, 1.0],
                roughness: 0.75,
                variation: 0.6,
                reflectivity: 0.25,
                ..d.clone()
            },
        ),
        (MAT_CAR_PAINT[0], MaterialParams { base_color: [0.85, 0.85, 0.84, 1.0], roughness: 0.18, reflectivity: 1.0, ..d.clone() }),
        (MAT_CAR_PAINT[1], MaterialParams { base_color: [0.015, 0.015, 0.018, 1.0], roughness: 0.15, reflectivity: 1.0, ..d.clone() }),
        (MAT_CAR_PAINT[2], MaterialParams { base_color: [0.45, 0.02, 0.02, 1.0], roughness: 0.18, reflectivity: 1.0, ..d.clone() }),
        (MAT_CAR_PAINT[3], MaterialParams { base_color: [0.03, 0.08, 0.25, 1.0], roughness: 0.18, reflectivity: 1.0, ..d.clone() }),
        (MAT_CAR_PAINT[4], MaterialParams { base_color: [0.55, 0.56, 0.58, 1.0], metallic: 0.8, roughness: 0.3, ..d.clone() }),
        (MAT_CAR_GLASS, MaterialParams { kind: MaterialKind::Glass, base_color: [0.02, 0.03, 0.035, 0.8], roughness: 0.03, ..d.clone() }),
        (MAT_TIRE, MaterialParams { base_color: [0.02, 0.02, 0.02, 1.0], roughness: 0.85, ..d.clone() }),
        (MAT_FABRIC, MaterialParams { base_color: [0.72, 0.71, 0.68, 1.0], roughness: 0.95, reflectivity: 0.2, ..d.clone() }),
        (MAT_WOOD, MaterialParams { base_color: [0.30, 0.17, 0.08, 1.0], roughness: 0.55, variation: 0.3, ..d.clone() }),
        (MAT_METAL, MaterialParams { base_color: [0.05, 0.05, 0.055, 1.0], metallic: 1.0, roughness: 0.35, ..d.clone() }),
        (MAT_CHROME, MaterialParams { base_color: [0.8, 0.8, 0.82, 1.0], metallic: 1.0, roughness: 0.15, ..d.clone() }),
        (MAT_POT, MaterialParams { base_color: [0.42, 0.41, 0.39, 1.0], roughness: 0.9, variation: 0.4, ..d.clone() }),
    ];
    for (name, mut params) in defs {
        if !scene.imported_materials.contains_key(name) {
            // Leaf cards: alpha-cut photo-like leaf textures, generated once.
            let texture = match name {
                MAT_LEAVES => Some(leaf_texture(false)),
                MAT_CONIFER => Some(leaf_texture(true)),
                _ => None,
            };
            if texture.is_some() {
                params.cutout = true;
                params.use_texture = true;
                params.base_color = [1.0, 1.0, 1.0, 1.0];
                params.uv_scale = 1.0;
            }
            scene.set_material(Material { name: name.to_string(), params, texture, normal_texture: None, roughness_texture: None });
        }
    }
}

/// Procedural foliage texture (RGBA, transparent background): a cluster of
/// leaves (broadleaf) or needle sprays (conifer) for alpha-tested cards.
pub fn leaf_texture(conifer: bool) -> crate::scene::TextureData {
    let n = 256usize;
    let mut img = vec![0u8; n * n * 4];
    let mut rng = Rng(if conifer { 99 } else { 7 });
    let put = |img: &mut Vec<u8>, x: i32, y: i32, c: [f32; 3]| {
        if x < 0 || y < 0 || x >= n as i32 || y >= n as i32 {
            return;
        }
        let i = (y as usize * n + x as usize) * 4;
        img[i] = (c[0].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8;
        img[i + 1] = (c[1].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8;
        img[i + 2] = (c[2].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8;
        img[i + 3] = 255;
    };
    if conifer {
        // Needle sprays: a twig with short needles along both sides.
        for _ in 0..110 {
            let (cx, cy) = (rng.f() * n as f32, rng.f() * n as f32);
            let a = rng.f() * std::f32::consts::TAU;
            let len = 50.0 + rng.f() * 60.0;
            let (dx, dy) = (a.cos(), a.sin());
            let shade = 0.6 + rng.f() * 0.5;
            let base = [0.035 * shade, 0.075 * shade, 0.035 * shade];
            let steps = len as i32;
            for s in 0..steps {
                let t = s as f32;
                let (px, py) = (cx + dx * t, cy + dy * t);
                put(&mut img, px as i32, py as i32, [0.05, 0.035, 0.02]);
                if s % 2 == 0 {
                    for side in [-1.0f32, 1.0] {
                        let na = a + side * 0.9;
                        let nl = 11.0 * (1.0 - t / len * 0.5);
                        for k in 0..nl as i32 {
                            let kk = k as f32;
                            let tip = 1.0 + kk / nl * 0.4;
                            let c = [base[0] * tip, base[1] * tip, base[2] * tip];
                            put(&mut img, (px + na.cos() * kk) as i32, (py + na.sin() * kk) as i32, c);
                            put(&mut img, (px + na.cos() * kk) as i32 + 1, (py + na.sin() * kk) as i32, c);
                        }
                    }
                }
            }
        }
    } else {
        // Broad leaves: rotated ellipses with a lighter midrib and a little colour variation.
        for _ in 0..70 {
            let (cx, cy) = (rng.f() * n as f32, rng.f() * n as f32);
            let a = rng.f() * std::f32::consts::TAU;
            let (l, w) = (16.0 + rng.f() * 12.0, 7.0 + rng.f() * 5.0);
            let hue = rng.f();
            let shade = 0.55 + rng.f() * 0.6;
            let base = [(0.07 + hue * 0.06) * shade, (0.15 + hue * 0.05) * shade, 0.03 * shade];
            let (ca, sa) = (a.cos(), a.sin());
            let r = l.max(w) as i32 + 1;
            for y in -r..=r {
                for x in -r..=r {
                    let (fx, fy) = (x as f32, y as f32);
                    let u = (fx * ca + fy * sa) / l;
                    let v = (-fx * sa + fy * ca) / w;
                    // pointed leaf: narrower toward the tip
                    let width = (1.0 - u * u).max(0.0).sqrt() * (1.0 - 0.35 * u);
                    if u.abs() <= 1.0 && v.abs() <= width {
                        let rib = if v.abs() < 0.08 { 1.35 } else { 1.0 };
                        let edge = 0.8 + 0.2 * (1.0 - v.abs() / width.max(1e-3));
                        let sun = 1.0 + 0.25 * u;
                        put(&mut img, cx as i32 + x, cy as i32 + y, [base[0] * rib * edge * sun, base[1] * rib * edge * sun, base[2] * rib * edge]);
                    }
                }
            }
            // twig
            for t in 0..10 {
                put(&mut img, (cx - ca * (l + t as f32)) as i32, (cy - sa * (l + t as f32)) as i32, [0.06, 0.045, 0.025]);
            }
        }
    }
    crate::scene::TextureData::new(n as u32, n as u32, img)
}

/// Leaf cards scattered through a crown volume. Normals point away from the
/// crown centre so the whole canopy shades like a soft volume.
fn add_leaf_cards(m: &mut MeshData, center: Vec3, radius: Vec3, count: u32, size: f32, droop: f32, rng: &mut Rng) {
    for _ in 0..count {
        // Mostly near the surface of the crown.
        let d = loop {
            let v = Vec3::new(rng.f() * 2.0 - 1.0, rng.f() * 2.0 - 1.0, rng.f() * 2.0 - 1.0);
            if v.length_squared() <= 1.0 && v.length_squared() > 0.05 {
                break v.normalize() * (0.55 + 0.45 * rng.f().sqrt());
            }
        };
        let p = center + d * radius;
        let s = size * (0.75 + rng.f() * 0.5);
        let yaw = rng.f() * std::f32::consts::TAU;
        let pitch = (rng.f() - 0.5) * 1.6 - droop;
        let right = Vec3::new(yaw.cos(), 0.0, yaw.sin());
        let up = (Vec3::Y * pitch.cos() + Vec3::new(-yaw.sin(), 0.0, yaw.cos()) * pitch.sin()).normalize();
        let nrm = (d.normalize() + Vec3::Y * 0.35).normalize();
        let i = m.positions.len() as u32;
        for (a, b) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
            m.positions.push(p + right * a * s + up * b * s);
            m.normals.push(nrm);
            m.uvs.push(Vec2::new(a + 0.5, 0.5 - b));
        }
        m.indices.extend_from_slice(&[i, i + 1, i + 2, i, i + 2, i + 3]);
    }
}

pub fn ground_mesh(size: f32) -> MeshData {
    let h = size * 0.5;
    // A coarse grid keeps vertex fog/precision good on huge planes.
    let n = 32;
    let mut m = MeshData { name: "Ground".into(), material: MAT_GROUND.into(), ..Default::default() };
    for j in 0..=n {
        for i in 0..=n {
            let x = -h + size * i as f32 / n as f32;
            let z = -h + size * j as f32 / n as f32;
            m.positions.push(Vec3::new(x, -0.002, z));
            m.normals.push(Vec3::Y);
            m.uvs.push(Vec2::new(x, z));
        }
    }
    for j in 0..n {
        for i in 0..n {
            let a = j * (n + 1) + i;
            m.indices.extend_from_slice(&[a, a + n + 1, a + 1, a + 1, a + n + 1, a + n + 2]);
        }
    }
    m
}

fn hash2(x: i32, z: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374761393) ^ (z as u32).wrapping_mul(668265263) ^ seed.wrapping_mul(2246822519);
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    ((h ^ (h >> 16)) & 0xFFFFFF) as f32 / 16777216.0
}

fn vnoise(x: f32, z: f32, seed: u32) -> f32 {
    let (xi, zi) = (x.floor() as i32, z.floor() as i32);
    let (fx, fz) = (x - x.floor(), z - z.floor());
    let (ux, uz) = (fx * fx * (3.0 - 2.0 * fx), fz * fz * (3.0 - 2.0 * fz));
    let a = hash2(xi, zi, seed);
    let b = hash2(xi + 1, zi, seed);
    let c = hash2(xi, zi + 1, seed);
    let d = hash2(xi + 1, zi + 1, seed);
    (a + (b - a) * ux) + ((c + (d - c) * ux) - (a + (b - a) * ux)) * uz
}

/// Ridged multifractal noise: sharp mountain crests.
fn ridged(x: f32, z: f32) -> f32 {
    let (mut sum, mut amp, mut freq, mut weight) = (0.0, 0.5, 1.0, 1.0);
    for o in 0..6 {
        let n = 1.0 - (vnoise(x * freq, z * freq, 7 + o) * 2.0 - 1.0).abs();
        let n = n * n * weight;
        weight = (n * 1.8).clamp(0.0, 1.0);
        sum += n * amp;
        amp *= 0.5;
        freq *= 2.05;
    }
    sum
}

/// A ring of mountains around the site (Lumion-style backdrop). Heights
/// rise from the inner radius outward; it never casts shadows on the model.
pub fn mountains_mesh(inner: f32, outer: f32) -> MeshData {
    let mut m = MeshData { name: "Mountains".into(), material: MAT_MOUNTAINS.into(), ..Default::default() };
    let (segs, rings) = (384u32, 40u32);
    let height = |a: f32, r: f32| -> f32 {
        let t = ((r - inner) / (outer - inner)).clamp(0.0, 1.0);
        let env = (t * 3.2).min(1.0).powf(1.6) * (1.0 - ((t - 0.85) / 0.15).clamp(0.0, 1.0) * 0.3);
        let (x, z) = (a.cos() * r, a.sin() * r);
        let big = vnoise(x / 2600.0 + 11.0, z / 2600.0, 3);
        let peaks = 220.0 + 900.0 * big * big;
        ridged(x / 1500.0, z / 1500.0) * peaks * env + vnoise(x / 400.0, z / 400.0, 9) * 25.0 * env - 2.0
    };
    for j in 0..=rings {
        let tr = j as f32 / rings as f32;
        let r = inner + (outer - inner) * tr * tr.sqrt().max(tr);
        for i in 0..=segs {
            let a = i as f32 / segs as f32 * std::f32::consts::TAU;
            let h = height(a, r);
            m.positions.push(Vec3::new(a.cos() * r, h, a.sin() * r));
            m.uvs.push(Vec2::new(a.cos() * r, a.sin() * r));
            m.normals.push(Vec3::Y);
        }
    }
    let w = segs + 1;
    for j in 0..rings {
        for i in 0..segs {
            let a = j * w + i;
            m.indices.extend_from_slice(&[a, a + w, a + 1, a + 1, a + w, a + w + 1]);
        }
    }
    // Smooth normals from the triangle mesh.
    let mut acc = vec![Vec3::ZERO; m.positions.len()];
    for t in m.indices.chunks_exact(3) {
        let (p0, p1, p2) = (m.positions[t[0] as usize], m.positions[t[1] as usize], m.positions[t[2] as usize]);
        let mut n = (p1 - p0).cross(p2 - p0);
        if n.y < 0.0 {
            n = -n;
        }
        for &k in t {
            acc[k as usize] += n;
        }
    }
    m.normals = acc.into_iter().map(|n| n.normalize_or(Vec3::Y)).collect();
    m
}

/// Blades of grass around the site, split into tiles for culling. `blocked`
/// says where the model sits so grass doesn't grow through floors.
pub fn grass_meshes(center: Vec3, radius: f32, height: f32, blocked: &dyn Fn(f32, f32) -> bool) -> Vec<(String, MeshData)> {
    let tile = 16.0f32;
    let n = (radius / tile).ceil() as i32;
    let mut out = Vec::new();
    for tj in -n..n {
        for ti in -n..n {
            let x0 = center.x + ti as f32 * tile;
            let z0 = center.z + tj as f32 * tile;
            let mut m = MeshData { name: "Grass".into(), material: MAT_GRASS.into(), ..Default::default() };
            let mut rng = Rng(((ti + 1000) as u32).wrapping_mul(73856093) ^ ((tj + 1000) as u32).wrapping_mul(19349663) | 1);
            // Tufts per square metre, thinning out toward the edge.
            let mid = Vec3::new(x0 + tile / 2.0, 0.0, z0 + tile / 2.0) - Vec3::new(center.x, 0.0, center.z);
            let fall = (1.0 - (mid.length() / radius)).clamp(0.0, 1.0);
            if fall <= 0.0 {
                continue;
            }
            // About 300k blades in total, however big the lawn is.
            let per_m2 = (700_000.0 / (std::f32::consts::PI * radius * radius) / 4.0).clamp(8.0, 90.0);
            let tufts = (tile * tile * per_m2 * fall.sqrt()) as u32;
            for _ in 0..tufts {
                let x = x0 + rng.f() * tile;
                let z = z0 + rng.f() * tile;
                let d = Vec3::new(x - center.x, 0.0, z - center.z).length();
                // Thin out smoothly to nothing at the edge, so the lawn has no visible border.
                let t = ((radius - d) / (radius * 0.45)).clamp(0.0, 1.0);
                if d > radius || rng.f() > t * t * (3.0 - 2.0 * t) || blocked(x, z) {
                    continue;
                }
                // Clump density varies in patches, like real lawns.
                if vnoise(x * 0.35, z * 0.35, 21) < 0.22 && rng.f() < 0.4 {
                    continue;
                }
                for _ in 0..4 {
                    let a = rng.f() * std::f32::consts::TAU;
                    let h = height * (0.45 + rng.f() * 0.9) * (0.7 + vnoise(x * 0.2, z * 0.2, 5) * 0.6);
                    let w = 0.006 + rng.f() * 0.009;
                    let dir = Vec3::new(a.cos(), 0.0, a.sin());
                    let side = Vec3::new(-dir.z, 0.0, dir.x);
                    let lean = dir * h * (0.15 + rng.f() * 0.45);
                    let base = Vec3::new(x + (rng.f() - 0.5) * 0.08, 0.0, z + (rng.f() - 0.5) * 0.08);
                    let i = m.positions.len() as u32;
                    m.positions.push(base - side * w);
                    m.positions.push(base + side * w);
                    m.positions.push(base + lean + Vec3::Y * h);
                    // Normals lean up so blades read as a soft lit carpet.
                    let nrm = (dir * 0.35 + Vec3::Y).normalize();
                    m.normals.extend_from_slice(&[nrm, nrm, nrm]);
                    m.uvs.extend_from_slice(&[Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.5, 1.0)]);
                    m.indices.extend_from_slice(&[i, i + 1, i + 2]);
                }
            }
            if !m.indices.is_empty() {
                out.push((format!("env:grass:{ti}:{tj}"), m));
            }
        }
    }
    out
}

struct Rng(u32);
impl Rng {
    fn f(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 & 0xFFFFFF) as f32 / 16777216.0
    }
}

fn add_cylinder(m: &mut MeshData, base: Vec3, r0: f32, r1: f32, h: f32, seg: u32) {
    let start = m.positions.len() as u32;
    for i in 0..=seg {
        let a = i as f32 / seg as f32 * std::f32::consts::TAU;
        let d = Vec3::new(a.cos(), 0.0, a.sin());
        let slope = (r0 - r1) / h;
        let n = (d + Vec3::Y * slope).normalize();
        m.positions.push(base + d * r0);
        m.positions.push(base + d * r1 + Vec3::Y * h);
        m.normals.push(n);
        m.normals.push(n);
        m.uvs.push(Vec2::new(i as f32 / seg as f32 * 2.0, 0.0));
        m.uvs.push(Vec2::new(i as f32 / seg as f32 * 2.0, h));
    }
    for i in 0..seg {
        let a = start + i * 2;
        m.indices.extend_from_slice(&[a, a + 1, a + 2, a + 2, a + 1, a + 3]);
    }
    // cap
    let c = m.positions.len() as u32;
    m.positions.push(base + Vec3::Y * h);
    m.normals.push(Vec3::Y);
    m.uvs.push(Vec2::ZERO);
    for i in 0..seg {
        m.indices.extend_from_slice(&[c, start + i * 2 + 3, start + i * 2 + 1]);
    }
}

/// Noise-displaced UV sphere (organic blobs for crowns, shrubs, rocks).
fn add_blob(m: &mut MeshData, center: Vec3, radius: Vec3, rng: &mut Rng, rough: f32) {
    let (rings, segs) = (10u32, 16u32);
    let start = m.positions.len() as u32;
    let phases: Vec<f32> = (0..6).map(|_| rng.f() * 6.28).collect();
    for r in 0..=rings {
        let v = r as f32 / rings as f32;
        let th = v * std::f32::consts::PI;
        for s in 0..=segs {
            let u = s as f32 / segs as f32;
            let ph = u * std::f32::consts::TAU;
            let d = Vec3::new(th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin());
            let bump = 1.0
                + rough
                    * (0.18 * (d.x * 3.1 + phases[0]).sin() * (d.z * 2.7 + phases[1]).cos()
                        + 0.12 * (d.y * 5.3 + phases[2]).sin()
                        + 0.08 * ((d.x + d.z) * 7.0 + phases[3]).sin());
            m.positions.push(center + d * radius * bump);
            m.normals.push(d);
            m.uvs.push(Vec2::new(u * 3.0, v * 3.0));
        }
    }
    for r in 0..rings {
        for s in 0..segs {
            let a = start + r * (segs + 1) + s;
            let b = a + segs + 1;
            m.indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
}

/// Axis-aligned box with flat normals.
fn add_box(m: &mut MeshData, center: Vec3, half: Vec3) {
    let faces = [
        (Vec3::X, Vec3::Z, Vec3::Y),
        (-Vec3::X, -Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::Z),
        (-Vec3::Y, Vec3::X, -Vec3::Z),
        (Vec3::Z, -Vec3::X, Vec3::Y),
        (-Vec3::Z, Vec3::X, Vec3::Y),
    ];
    for (n, u, v) in faces {
        let start = m.positions.len() as u32;
        let c = center + n * half;
        let (hu, hv) = ((u * half).length(), (v * half).length());
        for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            m.positions.push(c + u * a * hu + v * b * hv);
            m.normals.push(n);
            m.uvs.push(Vec2::new(a * hu, b * hv));
        }
        m.indices.extend_from_slice(&[start, start + 2, start + 1, start, start + 3, start + 2]);
    }
}

/// Box with rounded edges and corners (cushions, car bodies).
/// `taper` narrows the top in x and z (0 = straight sides).
fn add_rounded_box(m: &mut MeshData, center: Vec3, half: Vec3, radius: f32, taper: f32) {
    let r = radius.min(half.min_element() * 0.95);
    let inner = half - Vec3::splat(r);
    let n = 6u32;
    let faces = [
        (Vec3::X, Vec3::Z, Vec3::Y),
        (-Vec3::X, -Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::Z),
        (-Vec3::Y, Vec3::X, -Vec3::Z),
        (Vec3::Z, -Vec3::X, Vec3::Y),
        (-Vec3::Z, Vec3::X, Vec3::Y),
    ];
    for (f, u, v) in faces {
        let start = m.positions.len() as u32;
        for j in 0..=n {
            for i in 0..=n {
                let a = i as f32 / n as f32 * 2.0 - 1.0;
                let b = j as f32 / n as f32 * 2.0 - 1.0;
                // point on the unit cube face, scaled to the box
                let q = (f + u * a + v * b) * half;
                let c = q.clamp(-inner, inner);
                let d = q - c;
                let nrm = if d.length_squared() > 1e-10 { d.normalize() } else { f };
                let mut p = c + nrm * r;
                let ty = ((p.y + half.y) / (2.0 * half.y)).clamp(0.0, 1.0);
                p.x *= 1.0 - taper * ty;
                p.z *= 1.0 - taper * 0.6 * ty;
                m.positions.push(center + p);
                m.normals.push(nrm);
                m.uvs.push(Vec2::new(a * half.length(), b * half.length()));
            }
        }
        let w = n + 1;
        for j in 0..n {
            for i in 0..n {
                let k = start + j * w + i;
                m.indices.extend_from_slice(&[k, k + w + 1, k + 1, k, k + w, k + w + 1]);
            }
        }
    }
}

/// Wheel: a cylinder lying along X.
fn add_wheel(m: &mut MeshData, center: Vec3, r: f32, width: f32) {
    let start = m.positions.len();
    add_cylinder(m, Vec3::new(0.0, -width / 2.0, 0.0), r, r, width, 16);
    // close the other side too
    let c = m.positions.len() as u32;
    m.positions.push(Vec3::new(0.0, -width / 2.0, 0.0));
    m.normals.push(-Vec3::Y);
    m.uvs.push(Vec2::ZERO);
    for i in 0..16u32 {
        m.indices.extend_from_slice(&[c, start as u32 + i * 2, start as u32 + i * 2 + 2]);
    }
    let rot = Mat3::from_rotation_z(std::f32::consts::FRAC_PI_2);
    for p in &mut m.positions[start..] {
        *p = rot * *p + center;
    }
    for n in &mut m.normals[start..] {
        *n = rot * *n;
    }
}

fn add_legs(m: &mut MeshData, hx: f32, hz: f32, h: f32, r: f32) {
    for (x, z) in [(-hx, -hz), (hx, -hz), (hx, hz), (-hx, hz)] {
        add_cylinder(m, Vec3::new(x, 0.0, z), r, r, h, 8);
    }
}

// ---------------------------------------------------------------- 3D models

/// A model file loaded once and shared by every placed copy.
pub struct ModelAsset {
    pub name: String,
    pub parts: Vec<MeshData>,
    pub materials: Vec<Material>,
    pub size: Vec3,
}

type ModelCache = std::sync::Mutex<std::collections::HashMap<String, Option<std::sync::Arc<ModelAsset>>>>;

fn model_cache() -> &'static ModelCache {
    static CACHE: std::sync::OnceLock<ModelCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Folder of the built-in CC0 3D models (next to the executable).
pub fn builtin_models_dir() -> Option<std::path::PathBuf> {
    let lib = crate::library::builtin_dir()?;
    let d = lib.parent()?.join("models");
    d.is_dir().then_some(d)
}

/// Resolve a model key to a file on disk.
pub fn model_path(key: &str) -> Option<std::path::PathBuf> {
    if let Some(rel) = key.strip_prefix("builtin:") {
        let dir = builtin_models_dir()?.join(rel);
        let id = dir.file_name()?.to_string_lossy().to_string();
        let p = dir.join(format!("{id}.glb"));
        return p.is_file().then_some(p);
    }
    let p = std::path::PathBuf::from(key);
    p.is_file().then_some(p)
}

/// Load (once) and return a model. Materials are renamed per model so two
/// models' "Wood" don't clash, and the model sits centred on the origin.
pub fn model_asset(key: &str) -> Option<std::sync::Arc<ModelAsset>> {
    if let Some(v) = model_cache().lock().unwrap().get(key) {
        return v.clone();
    }
    let loaded = model_path(key).and_then(|path| {
        let settings = crate::import::ImportSettings::default_for(&path);
        let m = crate::import::load(&path, &settings).map_err(|e| log::warn!("model {}: {e:#}", path.display())).ok()?;
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let prefix = format!("Model {stem} - ");
        let mut parts: Vec<MeshData> = m.meshes.into_iter().map(|(_, d)| d).collect();
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for p in &mut parts {
            p.material = format!("{prefix}{}", p.material);
            if let Some((a, b)) = p.bounds() {
                lo = lo.min(a);
                hi = hi.max(b);
            }
        }
        let materials = m
            .materials
            .into_iter()
            .map(|mut mt| {
                mt.name = format!("{prefix}{}", mt.name);
                mt
            })
            .collect();
        let name = stem.replace(['_', '-'], " ");
        Some(std::sync::Arc::new(ModelAsset { name, parts, materials, size: if lo.x <= hi.x { hi - lo } else { Vec3::ONE } }))
    });
    model_cache().lock().unwrap().insert(key.to_string(), loaded.clone());
    loaded
}

/// Make sure a placed model's materials exist in the scene.
pub fn register_model_materials(scene: &mut Scene, obj: &PlacedObject) {
    if obj.kind != ObjectKind::Model {
        return;
    }
    let Some(asset) = obj.model.as_deref().and_then(model_asset) else { return };
    for m in &asset.materials {
        if !scene.imported_materials.contains_key(&m.name) {
            scene.set_material(m.clone());
        }
    }
}

/// Build meshes (one per material) for a placed object, in world space.
pub fn build(obj: &PlacedObject) -> Vec<MeshData> {
    let mut rng = Rng(obj.seed.wrapping_mul(2654435761).max(1));
    let mut parts: Vec<MeshData> = Vec::new();
    let label = obj.kind.label();
    let part = |parts: &mut Vec<MeshData>, mat: &str| -> usize {
        parts.push(MeshData { name: label.into(), material: mat.into(), ..Default::default() });
        parts.len() - 1
    };
    match obj.kind {
        ObjectKind::Tree => {
            let b = part(&mut parts, MAT_BARK);
            let l = part(&mut parts, MAT_LEAVES);
            let h = 3.0 + rng.f() * 1.6;
            let lean = Vec3::new(rng.f() - 0.5, 0.0, rng.f() - 0.5) * 0.25;
            add_cylinder(&mut parts[b], Vec3::ZERO, 0.24, 0.14, h + 0.6, 10);
            let crown = Vec3::Y * (h + 1.9) + lean;
            // Main limbs reaching into the crown.
            for i in 0..5 {
                let a = i as f32 * 1.26 + rng.f() * 0.6;
                let dir = Vec3::new(a.cos(), 0.9 + rng.f() * 0.5, a.sin()).normalize();
                let start = Vec3::Y * (h * (0.62 + rng.f() * 0.25));
                let len = 1.6 + rng.f() * 0.9;
                let s0 = parts[b].positions.len();
                add_cylinder(&mut parts[b], Vec3::ZERO, 0.09, 0.03, len, 6);
                let rot = glam::Quat::from_rotation_arc(Vec3::Y, dir);
                for p in &mut parts[b].positions[s0..] {
                    *p = rot * *p + start;
                }
                for nn in &mut parts[b].normals[s0..] {
                    *nn = rot * *nn;
                }
            }
            add_leaf_cards(&mut parts[l], crown, Vec3::new(2.5, 2.2, 2.5), 420, 1.15, 0.2, &mut rng);
            for _ in 0..4 {
                let a = rng.f() * 6.28;
                let c = crown + Vec3::new(a.cos() * 1.3, -0.4 + rng.f() * 1.4, a.sin() * 1.3);
                add_leaf_cards(&mut parts[l], c, Vec3::splat(1.3 + rng.f() * 0.4), 70, 1.0, 0.2, &mut rng);
            }
        }
        ObjectKind::Conifer => {
            let b = part(&mut parts, MAT_BARK);
            let l = part(&mut parts, MAT_CONIFER);
            add_cylinder(&mut parts[b], Vec3::ZERO, 0.2, 0.05, 8.2, 8);
            let tiers = 9;
            for i in 0..tiers {
                let t = i as f32 / tiers as f32;
                let r = 2.4 * (1.0 - t * 0.88);
                let y = 1.3 + t * 6.6;
                // Drooping layers of needle sprays around the trunk.
                let count = (95.0 * (1.0 - t * 0.6)) as u32;
                for _ in 0..count {
                    let a = rng.f() * std::f32::consts::TAU;
                    let rr = r * (0.35 + 0.65 * rng.f().sqrt());
                    let p = Vec3::new(a.cos() * rr, y - rr * 0.28 + rng.f() * 0.25, a.sin() * rr);
                    let s = 1.2 + rng.f() * 0.6;
                    let out = Vec3::new(a.cos(), 0.0, a.sin());
                    let side = Vec3::new(-a.sin(), 0.0, a.cos());
                    let down = (out * 0.85 - Vec3::Y * 0.5).normalize();
                    let nrm = (out + Vec3::Y * 0.5).normalize();
                    let m = &mut parts[l];
                    let i0 = m.positions.len() as u32;
                    for (u, v) in [(-0.5f32, -0.5f32), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
                        m.positions.push(p + side * u * s + down * v * s);
                        m.normals.push(nrm);
                        m.uvs.push(Vec2::new(u + 0.5, 0.5 - v));
                    }
                    m.indices.extend_from_slice(&[i0, i0 + 1, i0 + 2, i0, i0 + 2, i0 + 3]);
                }
            }
            add_leaf_cards(&mut parts[l], Vec3::Y * 8.0, Vec3::new(0.35, 0.6, 0.35), 14, 0.7, 0.0, &mut rng);
        }
        ObjectKind::Shrub => {
            let l = part(&mut parts, MAT_LEAVES);
            for _ in 0..3 {
                let a = rng.f() * 6.28;
                let c = Vec3::new(a.cos() * 0.3, 0.5 + rng.f() * 0.15, a.sin() * 0.3);
                add_leaf_cards(&mut parts[l], c, Vec3::new(0.65, 0.5, 0.65), 60, 0.55, 0.0, &mut rng);
            }
        }
        ObjectKind::Person => {
            let p = part(&mut parts, MAT_PERSON);
            add_cylinder(&mut parts[p], Vec3::new(-0.1, 0.0, 0.0), 0.07, 0.09, 0.85, 8);
            add_cylinder(&mut parts[p], Vec3::new(0.1, 0.0, 0.0), 0.07, 0.09, 0.85, 8);
            add_blob(&mut parts[p], Vec3::Y * 1.2, Vec3::new(0.23, 0.38, 0.15), &mut rng, 0.1);
            add_blob(&mut parts[p], Vec3::Y * 1.68, Vec3::splat(0.11), &mut rng, 0.0);
        }
        ObjectKind::StreetLamp => {
            let p = part(&mut parts, MAT_LAMP_POST);
            let l = part(&mut parts, MAT_LAMP_LIGHT);
            add_cylinder(&mut parts[p], Vec3::ZERO, 0.08, 0.05, 4.2, 10);
            add_cylinder(&mut parts[p], Vec3::Y * 4.2, 0.28, 0.05, 0.25, 12);
            add_blob(&mut parts[l], Vec3::Y * 4.12, Vec3::new(0.2, 0.08, 0.2), &mut rng, 0.0);
        }
        ObjectKind::PointLight | ObjectKind::SpotLight => {
            // Lights have no geometry; the viewport draws a marker for them.
        }
        ObjectKind::Boulder => {
            let s = part(&mut parts, MAT_STONE);
            add_blob(&mut parts[s], Vec3::Y * 0.3, Vec3::new(0.9, 0.6, 0.7), &mut rng, 2.0);
        }
        ObjectKind::Car => {
            let paint = part(&mut parts, MAT_CAR_PAINT[(obj.seed % 5) as usize]);
            let glass = part(&mut parts, MAT_CAR_GLASS);
            let tire = part(&mut parts, MAT_TIRE);
            let chrome = part(&mut parts, MAT_CHROME);
            let lamp = part(&mut parts, MAT_LAMP_LIGHT);
            let suv = obj.seed.is_multiple_of(3);
            let lift = if suv { 0.14 } else { 0.0 };
            let wr = if suv { 0.38 } else { 0.33 };
            // Lower body with rounded shoulders, slightly narrower bumpers.
            add_rounded_box(&mut parts[paint], Vec3::new(0.0, 0.58 + lift, 0.0), Vec3::new(0.9, 0.27, 2.3), 0.16, 0.06);
            // Hood and boot slope into the body.
            add_rounded_box(&mut parts[paint], Vec3::new(0.0, 0.84 + lift, 1.45), Vec3::new(0.86, 0.06, 0.8), 0.06, 0.0);
            let cab_h = if suv { 0.33 } else { 0.25 };
            let cab_len = if suv { 1.45 } else { 1.1 };
            let cab_z = if suv { -0.4 } else { -0.2 };
            // Glasshouse tapers inward toward the roof.
            add_rounded_box(&mut parts[glass], Vec3::new(0.0, 0.87 + lift + cab_h, cab_z), Vec3::new(0.84, cab_h, cab_len), 0.12, 0.16);
            add_rounded_box(&mut parts[paint], Vec3::new(0.0, 0.89 + lift + cab_h * 2.0, cab_z - 0.05), Vec3::new(0.72, 0.035, cab_len - 0.2), 0.03, 0.0);
            // Pillars
            for x in [-0.78f32, 0.78] {
                add_box(&mut parts[paint], Vec3::new(x * 0.95, 0.9 + lift + cab_h, cab_z), Vec3::new(0.03, cab_h, 0.05));
            }
            add_rounded_box(&mut parts[tire], Vec3::new(0.0, 0.42 + lift, 2.27), Vec3::new(0.88, 0.1, 0.08), 0.05, 0.0);
            add_rounded_box(&mut parts[tire], Vec3::new(0.0, 0.42 + lift, -2.27), Vec3::new(0.88, 0.1, 0.08), 0.05, 0.0);
            for x in [-0.62f32, 0.62] {
                add_rounded_box(&mut parts[lamp], Vec3::new(x, 0.68 + lift, 2.29), Vec3::new(0.16, 0.05, 0.03), 0.02, 0.0);
            }
            for (x, z) in [(-0.84, 1.45), (0.84, 1.45), (-0.84, -1.45), (0.84, -1.45)] {
                add_wheel(&mut parts[tire], Vec3::new(x, wr, z), wr, 0.24);
                let sx = if x < 0.0 { -1.0 } else { 1.0 };
                add_wheel(&mut parts[chrome], Vec3::new(x + sx * 0.005, wr, z), wr * 0.62, 0.25);
            }
        }
        ObjectKind::ArmChair | ObjectKind::Sofa => {
            let fab = part(&mut parts, MAT_FABRIC);
            let leg = part(&mut parts, MAT_METAL);
            let hw = if obj.kind == ObjectKind::Sofa { 1.0 } else { 0.45 };
            add_box(&mut parts[leg], Vec3::new(0.0, 0.04, 0.0), Vec3::new(hw - 0.05, 0.04, 0.38));
            add_rounded_box(&mut parts[fab], Vec3::new(0.0, 0.2, 0.0), Vec3::new(hw, 0.12, 0.44), 0.04, 0.0);
            // Seat cushions
            let seats = if obj.kind == ObjectKind::Sofa { 3 } else { 1 };
            let sw = (hw - 0.14) / seats as f32;
            for k in 0..seats {
                let x = -hw + 0.14 + sw * (2 * k + 1) as f32;
                add_rounded_box(&mut parts[fab], Vec3::new(x, 0.38, 0.06), Vec3::new(sw - 0.01, 0.07, 0.36), 0.06, 0.0);
                add_rounded_box(&mut parts[fab], Vec3::new(x, 0.62, -0.26), Vec3::new(sw - 0.01, 0.2, 0.08), 0.07, 0.0);
            }
            add_rounded_box(&mut parts[fab], Vec3::new(0.0, 0.55, -0.36), Vec3::new(hw, 0.3, 0.08), 0.05, 0.0);
            for x in [-(hw - 0.08), hw - 0.08] {
                add_rounded_box(&mut parts[fab], Vec3::new(x, 0.46, 0.02), Vec3::new(0.08, 0.17, 0.42), 0.06, 0.0);
            }
        }
        ObjectKind::Table => {
            let wood = part(&mut parts, MAT_WOOD);
            let leg = part(&mut parts, MAT_METAL);
            add_rounded_box(&mut parts[wood], Vec3::new(0.0, 0.74, 0.0), Vec3::new(0.9, 0.025, 0.48), 0.012, 0.0);
            add_legs(&mut parts[leg], 0.82, 0.4, 0.72, 0.025);
        }
        ObjectKind::Chair => {
            let wood = part(&mut parts, MAT_WOOD);
            let leg = part(&mut parts, MAT_METAL);
            add_box(&mut parts[wood], Vec3::new(0.0, 0.46, 0.0), Vec3::new(0.22, 0.02, 0.22));
            add_box(&mut parts[wood], Vec3::new(0.0, 0.72, -0.21), Vec3::new(0.22, 0.14, 0.015));
            add_legs(&mut parts[leg], 0.19, 0.19, 0.45, 0.015);
            add_cylinder(&mut parts[leg], Vec3::new(-0.19, 0.45, -0.2), 0.012, 0.012, 0.3, 6);
            add_cylinder(&mut parts[leg], Vec3::new(0.19, 0.45, -0.2), 0.012, 0.012, 0.3, 6);
        }
        ObjectKind::Bench => {
            let wood = part(&mut parts, MAT_WOOD);
            let leg = part(&mut parts, MAT_METAL);
            for z in [-0.14, 0.0, 0.14] {
                add_box(&mut parts[wood], Vec3::new(0.0, 0.45, z), Vec3::new(0.9, 0.02, 0.06));
            }
            for y in [0.62, 0.76] {
                add_box(&mut parts[wood], Vec3::new(0.0, y, -0.24), Vec3::new(0.9, 0.05, 0.015));
            }
            for x in [-0.75, 0.75] {
                add_box(&mut parts[leg], Vec3::new(x, 0.22, 0.0), Vec3::new(0.025, 0.22, 0.22));
                add_box(&mut parts[leg], Vec3::new(x, 0.6, -0.25), Vec3::new(0.025, 0.2, 0.02));
            }
        }
        ObjectKind::Planter => {
            let pot = part(&mut parts, MAT_POT);
            let l = part(&mut parts, MAT_LEAVES);
            add_box(&mut parts[pot], Vec3::new(0.0, 0.3, 0.0), Vec3::new(0.45, 0.3, 0.45));
            for _ in 0..3 {
                let a = rng.f() * 6.28;
                let c = Vec3::new(a.cos() * 0.15, 0.85 + rng.f() * 0.15, a.sin() * 0.15);
                add_leaf_cards(&mut parts[l], c, Vec3::new(0.4, 0.33, 0.4), 30, 0.4, 0.0, &mut rng);
            }
        }
        ObjectKind::Model => {
            if let Some(asset) = obj.model.as_deref().and_then(model_asset) {
                parts = asset.parts.clone();
            }
        }
    }
    let rot = Mat3::from_rotation_y(obj.rotation);
    let pos = Vec3::from(obj.position);
    for m in &mut parts {
        for p in &mut m.positions {
            *p = rot * (*p * obj.scale) + pos;
        }
        for n in &mut m.normals {
            *n = rot * *n;
        }
    }
    parts
}

/// Rebuild just one object's meshes (used while dragging it around).
pub fn rebuild_one(scene: &mut Scene, objects: &[PlacedObject], i: usize) {
    let prefix = format!("obj:{i}:");
    if let Some(o) = objects.get(i) {
        register_model_materials(scene, o);
    }
    let parts = objects.get(i).map(build).unwrap_or_default();
    let keep: Vec<String> = (0..parts.len()).map(|j| format!("{prefix}{j}")).collect();
    scene.meshes.retain(|k, _| !k.starts_with(&prefix) || keep.contains(k));
    for (j, m) in parts.into_iter().enumerate() {
        scene.upsert_mesh(format!("{prefix}{j}"), m);
    }
}

/// Rebuild the "obj:" meshes in the scene from the placed-object list.
pub fn sync_objects(scene: &mut Scene, objects: &[PlacedObject]) {
    let mut keep = std::collections::HashSet::new();
    for (i, o) in objects.iter().enumerate() {
        register_model_materials(scene, o);
        for (j, m) in build(o).into_iter().enumerate() {
            let id = format!("obj:{i}:{j}");
            keep.insert(id.clone());
            // Unchanged objects hash the same and are not re-uploaded.
            scene.upsert_mesh(id, m);
        }
    }
    scene.meshes.retain(|k, _| !k.starts_with("obj:") || keep.contains(k));
}
