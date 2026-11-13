//! Scene model shared by the importers, the live-sync bridges and the renderer.
//!
//! All geometry stored here is already in *render space*: Y-up, meters.

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MaterialKind {
    #[default]
    Standard,
    Glass,
    Water,
    Emissive,
    Foliage,
    Ground,
    /// Distant mountains: rock, forest and snow by height and slope.
    Terrain,
}

impl MaterialKind {
    pub const ALL: [MaterialKind; 7] = [
        MaterialKind::Standard,
        MaterialKind::Glass,
        MaterialKind::Water,
        MaterialKind::Emissive,
        MaterialKind::Foliage,
        MaterialKind::Ground,
        MaterialKind::Terrain,
    ];
    pub fn label(self) -> &'static str {
        match self {
            MaterialKind::Standard => "Standard",
            MaterialKind::Glass => "Glass",
            MaterialKind::Water => "Water",
            MaterialKind::Emissive => "Emissive",
            MaterialKind::Foliage => "Foliage",
            MaterialKind::Ground => "Landscape",
            MaterialKind::Terrain => "Mountains",
        }
    }
    pub fn id(self) -> u32 {
        match self {
            MaterialKind::Standard => 0,
            MaterialKind::Glass => 1,
            MaterialKind::Water => 2,
            MaterialKind::Emissive => 3,
            MaterialKind::Foliage => 4,
            MaterialKind::Ground => 5,
            MaterialKind::Terrain => 6,
        }
    }
    pub fn is_transparent(self) -> bool {
        matches!(self, MaterialKind::Glass | MaterialKind::Water)
    }
}

/// RGBA8 texture decoded from an imported file.
#[derive(Clone, Debug)]
pub struct TextureData {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
    /// Content hash: identical images across re-syncs share one GPU texture.
    pub key: u64,
}

impl TextureData {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        let mut h: u64 = 0x9E3779B97F4A7C15 ^ ((width as u64) << 32 | height as u64);
        for c in rgba.chunks(8) {
            let mut w = [0u8; 8];
            w[..c.len()].copy_from_slice(c);
            h = (h ^ u64::from_le_bytes(w)).wrapping_mul(0x100000001b3).rotate_left(31);
        }
        Self { width, height, rgba: Arc::new(rgba), key: h | 1 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MaterialParams {
    pub kind: MaterialKind,
    /// Linear RGB + opacity.
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 3],
    pub emissive_strength: f32,
    /// Texture coordinate scale (Lumion's "scale" slider).
    pub uv_scale: f32,
    pub use_texture: bool,
    /// Strength of the procedural "weathering"/variation noise.
    pub variation: f32,
    /// Texture placement from the source file: (scale u, scale v, offset u, offset v).
    #[serde(default = "uv_identity")]
    pub uv_transform: [f32; 4],
    /// Normal/bump map strength (Lumion's "Relief").
    #[serde(default = "one")]
    pub normal_strength: f32,
    /// Alpha-tested cut-outs (leaves, fences, perforated panels).
    #[serde(default)]
    pub cutout: bool,
    /// User-assigned colour map (image path) replacing the model's texture.
    #[serde(default)]
    pub color_map: Option<String>,
    /// User-assigned normal map (image path).
    #[serde(default)]
    pub normal_map: Option<String>,
    /// User-assigned roughness map (image path; `#gloss` = inverted).
    #[serde(default)]
    pub roughness_map: Option<String>,
    /// How much the colour map shows through the colour (Lumion's "Texture" slider).
    #[serde(default = "one")]
    pub tex_influence: f32,
    /// Strength of the specular reflection on non-metals (0.5 = physical default).
    #[serde(default = "half")]
    pub reflectivity: f32,
    /// Texture placement: offset in tiles (u, v).
    #[serde(default)]
    pub uv_offset: [f32; 2],
    /// Texture placement: rotation in degrees ("Heading").
    #[serde(default)]
    pub uv_rotation: f32,
    /// Break up visible tiling repetition on large surfaces.
    #[serde(default)]
    pub anti_tiling: bool,
    /// Hide surfaces with this material entirely.
    #[serde(default)]
    pub invisible: bool,
    /// Real-world (box) mapping: texture placed by world position in metres,
    /// ignoring the model's own UVs (which are often missing, stretched or in
    /// feet/inches in Revit exports).
    #[serde(default)]
    pub box_mapping: bool,
}

fn uv_identity() -> [f32; 4] {
    [1.0, 1.0, 0.0, 0.0]
}
fn one() -> f32 {
    1.0
}
fn half() -> f32 {
    0.5
}

impl Default for MaterialParams {
    fn default() -> Self {
        Self {
            kind: MaterialKind::Standard,
            base_color: [0.8, 0.8, 0.8, 1.0],
            metallic: 0.0,
            roughness: 0.6,
            emissive: [0.0; 3],
            emissive_strength: 0.0,
            uv_scale: 1.0,
            use_texture: true,
            variation: 0.0,
            uv_transform: uv_identity(),
            normal_strength: 1.0,
            cutout: false,
            color_map: None,
            normal_map: None,
            roughness_map: None,
            tex_influence: 1.0,
            reflectivity: 0.5,
            uv_offset: [0.0, 0.0],
            uv_rotation: 0.0,
            anti_tiling: false,
            invisible: false,
            box_mapping: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Material {
    pub name: String,
    pub params: MaterialParams,
    /// Colour (albedo) map; alpha = opacity.
    pub texture: Option<TextureData>,
    /// Tangent-space normal map (bump maps are converted on import).
    pub normal_texture: Option<TextureData>,
    /// Roughness map (grey, linear).
    pub roughness_texture: Option<TextureData>,
}

#[derive(Clone, Debug, Default)]
pub struct MeshData {
    pub name: String,
    pub material: String,
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
    pub indices: Vec<u32>,
}

impl MeshData {
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut it = self.positions.iter();
        let first = *it.next()?;
        let (mut lo, mut hi) = (first, first);
        for p in it {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        Some((lo, hi))
    }

    /// Fill in missing normals/uvs so every mesh is renderable.
    pub fn finalize(&mut self) {
        let n = self.positions.len();
        if self.indices.is_empty() {
            self.indices = (0..n as u32).collect();
        }
        self.indices.truncate(self.indices.len() / 3 * 3);
        self.indices.retain(|&i| (i as usize) < n);
        self.indices.truncate(self.indices.len() / 3 * 3);
        if self.normals.len() != n {
            self.normals = vec![Vec3::ZERO; n];
            for t in self.indices.chunks_exact(3) {
                let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
                let fnrm = (self.positions[b] - self.positions[a]).cross(self.positions[c] - self.positions[a]);
                self.normals[a] += fnrm;
                self.normals[b] += fnrm;
                self.normals[c] += fnrm;
            }
            for nrm in &mut self.normals {
                *nrm = nrm.try_normalize().unwrap_or(Vec3::Y);
            }
        }
        if self.uvs.len() != n {
            // Box-projected world UVs (1 tile per meter) — the same fallback Lumion uses.
            self.uvs = self
                .positions
                .iter()
                .zip(&self.normals)
                .map(|(p, nrm)| {
                    let a = nrm.abs();
                    if a.y >= a.x && a.y >= a.z {
                        Vec2::new(p.x, p.z)
                    } else if a.x >= a.z {
                        Vec2::new(p.z, p.y)
                    } else {
                        Vec2::new(p.x, p.y)
                    }
                })
                .collect();
        }
    }
}

#[derive(Clone, Debug)]
pub struct SceneMesh {
    pub data: Arc<MeshData>,
    pub rev: u64,
    pub bounds: (Vec3, Vec3),
    /// Content hash — re-syncs that produce identical geometry are skipped.
    pub hash: u64,
}

/// Key in `Scene::user_textures` for an image assigned in the material editor.
pub fn user_texture_key(path: &str, kind: crate::texlib::MapKind) -> String {
    match kind {
        crate::texlib::MapKind::Color => path.to_string(),
        crate::texlib::MapKind::Normal => format!("{path}#normal"),
        crate::texlib::MapKind::Roughness => format!("{path}#rough"),
    }
}

/// Fast non-cryptographic hash over the mesh contents.
pub fn mesh_hash(m: &MeshData) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325 ^ m.material.len() as u64;
    let mut eat = |bytes: &[u8]| {
        for c in bytes.chunks(8) {
            let mut w = [0u8; 8];
            w[..c.len()].copy_from_slice(c);
            h = (h ^ u64::from_le_bytes(w)).wrapping_mul(0x100000001b3).rotate_left(29);
        }
    };
    eat(m.material.as_bytes());
    eat(bytemuck::cast_slice(&m.positions));
    eat(bytemuck::cast_slice(&m.normals));
    eat(bytemuck::cast_slice(&m.uvs));
    eat(bytemuck::cast_slice(&m.indices));
    h
}

/// The live scene. Meshes are keyed by a stable id so bridges can update
/// individual elements (e.g. a single Revit/BimCraft element) in place.
#[derive(Default)]
pub struct Scene {
    pub meshes: BTreeMap<String, SceneMesh>,
    /// Materials as delivered by the source model.
    pub imported_materials: BTreeMap<String, Material>,
    /// User edits, keyed by material name; survive every re-sync.
    pub overrides: BTreeMap<String, MaterialParams>,
    next_rev: u64,
    pub material_rev: u64,
    /// Images the user assigned in the material editor, keyed by file path.
    pub user_textures: std::collections::HashMap<String, TextureData>,
}

impl Scene {
    /// Insert or replace a mesh. Returns false if the content was unchanged.
    pub fn upsert_mesh(&mut self, id: String, mut data: MeshData) -> bool {
        data.finalize();
        let Some(bounds) = data.bounds() else {
            self.meshes.remove(&id);
            return true;
        };
        let hash = mesh_hash(&data);
        if self.meshes.get(&id).is_some_and(|m| m.hash == hash) {
            return false;
        }
        self.next_rev += 1;
        if !self.imported_materials.contains_key(&data.material) {
            self.imported_materials.insert(
                data.material.clone(),
                Material {
                    name: data.material.clone(),
                    params: guess_params_from_name(&data.material, MaterialParams::default()),
                    texture: None,
                    normal_texture: None,
                    roughness_texture: None,
                },
            );
            self.material_rev += 1;
        }
        self.meshes.insert(id, SceneMesh { data: Arc::new(data), rev: self.next_rev, bounds, hash });
        true
    }

    pub fn remove_mesh(&mut self, id: &str) {
        self.meshes.remove(id);
    }

    pub fn remove_prefix(&mut self, prefix: &str) {
        self.meshes.retain(|k, _| !k.starts_with(prefix));
    }

    pub fn set_material(&mut self, m: Material) {
        self.imported_materials.insert(m.name.clone(), m);
        self.material_rev += 1;
    }

    pub fn effective(&self, name: &str) -> MaterialParams {
        if let Some(o) = self.overrides.get(name) {
            return o.clone();
        }
        self.imported_materials.get(name).map(|m| m.params.clone()).unwrap_or_default()
    }

    /// Bounds of everything except helper/procedural content.
    pub fn model_bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut out: Option<(Vec3, Vec3)> = None;
        for m in self.meshes.values() {
            out = Some(match out {
                None => m.bounds,
                Some((lo, hi)) => (lo.min(m.bounds.0), hi.max(m.bounds.1)),
            });
        }
        out
    }

    /// Union of the bounds of all meshes whose id starts with `prefix`.
    pub fn bounds_with_prefix(&self, prefix: &str) -> Option<(Vec3, Vec3)> {
        self.meshes.range(prefix.to_string()..).take_while(|(k, _)| k.starts_with(prefix)).map(|(_, m)| m.bounds).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
    }

    pub fn triangle_count(&self) -> usize {
        self.meshes.values().map(|m| m.data.indices.len() / 3).sum()
    }

    /// Ray cast against all triangles. Returns (distance, mesh id, material name).
    pub fn raycast(&self, origin: Vec3, dir: Vec3) -> Option<(f32, String, String)> {
        self.raycast_filtered(origin, dir, |_| true)
    }

    /// Ray cast against meshes whose id passes `keep`.
    pub fn raycast_filtered(&self, origin: Vec3, dir: Vec3, keep: impl Fn(&str) -> bool) -> Option<(f32, String, String)> {
        let mut best: Option<(f32, String, String)> = None;
        // Nearest boxes first: once a hit is closer than the next box, stop.
        let mut cands: Vec<(f32, &String, &SceneMesh)> = self
            .meshes
            .iter()
            .filter(|(id, _)| keep(id))
            .filter_map(|(id, m)| ray_aabb(origin, dir, m.bounds.0 - 1e-3, m.bounds.1 + 1e-3).map(|t| (t, id, m)))
            .collect();
        cands.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (tb, id, m) in cands {
            if best.as_ref().is_some_and(|b| tb > b.0) {
                break;
            }
            let p = &m.data.positions;
            for t in m.data.indices.chunks_exact(3) {
                if let Some(d) = ray_tri(origin, dir, p[t[0] as usize], p[t[1] as usize], p[t[2] as usize]) {
                    if best.as_ref().is_none_or(|b| d < b.0) {
                        best = Some((d, id.clone(), m.data.material.clone()));
                    }
                }
            }
        }
        best
    }
}

fn ray_aabb(o: Vec3, d: Vec3, lo: Vec3, hi: Vec3) -> Option<f32> {
    let inv = d.recip();
    let t0 = (lo - o) * inv;
    let t1 = (hi - o) * inv;
    let tmin = t0.min(t1).max_element();
    let tmax = t0.max(t1).min_element();
    (tmax >= tmin.max(0.0)).then_some(tmin.max(0.0))
}

fn ray_tri(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
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
    (t > 1e-4).then_some(t)
}

/// Revit material names are descriptive ("Glass", "Concrete - Cast-in-Place",
/// "Water"), so give sensible starting looks before the user touches anything.
pub fn guess_params_from_name(name: &str, mut p: MaterialParams) -> MaterialParams {
    let n = name.to_lowercase();
    let has = |k: &str| n.contains(k);
    if has("glass") || has("glazing") || has("window pane") {
        p.kind = MaterialKind::Glass;
        p.base_color = [0.55, 0.65, 0.68, 0.25];
        p.roughness = 0.05;
        p.metallic = 0.0;
    } else if has("water") || has("pool") {
        p.kind = MaterialKind::Water;
        p.base_color = [0.02, 0.12, 0.14, 0.85];
        p.roughness = 0.02;
    } else if has("grass") || has("lawn") || has("turf") {
        p.kind = MaterialKind::Ground;
        p.base_color = [0.13, 0.25, 0.05, 1.0];
        p.roughness = 0.9;
        p.variation = 0.6;
    } else if has("metal") || has("steel") || has("aluminum") || has("aluminium") {
        p.metallic = 1.0;
        p.roughness = 0.35;
    } else if has("concrete") || has("cement") {
        p.roughness = 0.85;
        p.variation = 0.3;
    } else if has("wood") || has("timber") || has("oak") {
        p.roughness = 0.55;
        p.variation = 0.25;
    } else if has("brick") || has("masonry") {
        p.roughness = 0.9;
        p.variation = 0.35;
    } else if has("light") && (has("lamp") || has("emissive") || has("bulb")) {
        p.kind = MaterialKind::Emissive;
        p.emissive = [1.0, 0.85, 0.6];
        p.emissive_strength = 8.0;
    }
    p
}

/// Material presets offered in the material editor (Lumion-style library).
pub fn presets() -> Vec<(&'static str, MaterialParams)> {
    let d = MaterialParams::default();
    let mk = |kind, c: [f32; 4], m, r, v| MaterialParams { kind, base_color: c, metallic: m, roughness: r, variation: v, ..d.clone() };
    vec![
        ("White plaster", mk(MaterialKind::Standard, [0.85, 0.84, 0.80, 1.0], 0.0, 0.85, 0.08)),
        ("Concrete", mk(MaterialKind::Standard, [0.45, 0.44, 0.42, 1.0], 0.0, 0.85, 0.35)),
        ("Red brick", mk(MaterialKind::Standard, [0.42, 0.13, 0.07, 1.0], 0.0, 0.9, 0.4)),
        ("Oak wood", mk(MaterialKind::Standard, [0.45, 0.28, 0.13, 1.0], 0.0, 0.5, 0.3)),
        ("Dark wood", mk(MaterialKind::Standard, [0.15, 0.08, 0.04, 1.0], 0.0, 0.45, 0.3)),
        ("Brushed steel", mk(MaterialKind::Standard, [0.62, 0.62, 0.64, 1.0], 1.0, 0.3, 0.05)),
        ("Black metal", mk(MaterialKind::Standard, [0.03, 0.03, 0.03, 1.0], 1.0, 0.4, 0.0)),
        ("Gold", mk(MaterialKind::Standard, [1.0, 0.77, 0.34, 1.0], 1.0, 0.25, 0.0)),
        ("Asphalt", mk(MaterialKind::Standard, [0.05, 0.05, 0.05, 1.0], 0.0, 0.95, 0.4)),
        ("Clear glass", mk(MaterialKind::Glass, [0.6, 0.7, 0.72, 0.2], 0.0, 0.03, 0.0)),
        ("Tinted glass", mk(MaterialKind::Glass, [0.15, 0.25, 0.3, 0.55], 0.0, 0.03, 0.0)),
        ("Pool water", mk(MaterialKind::Water, [0.02, 0.25, 0.3, 0.75], 0.0, 0.02, 0.0)),
        ("Lake water", mk(MaterialKind::Water, [0.02, 0.06, 0.05, 0.9], 0.0, 0.02, 0.0)),
        ("Grass", mk(MaterialKind::Ground, [0.075, 0.13, 0.035, 1.0], 0.0, 0.9, 0.7)),
        ("Gravel", mk(MaterialKind::Ground, [0.35, 0.33, 0.30, 1.0], 0.0, 0.95, 0.9)),
        ("Sand", mk(MaterialKind::Ground, [0.62, 0.52, 0.36, 1.0], 0.0, 0.95, 0.3)),
        (
            "Warm light",
            MaterialParams { kind: MaterialKind::Emissive, base_color: [1.0, 0.9, 0.7, 1.0], emissive: [1.0, 0.8, 0.55], emissive_strength: 10.0, ..d.clone() },
        ),
    ]
}
