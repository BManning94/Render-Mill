//! Model importers: FBX (Revit's built-in export), glTF/GLB and OBJ. These are
//! the files the folder watcher reloads on every re-export.

use crate::scene::{guess_params_from_name, Material, MaterialKind, MaterialParams, MeshData, TextureData};
use anyhow::{Context, Result};
use glam::{Mat3, Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpAxis {
    /// Use what the file declares (FBX), otherwise the format's convention.
    #[serde(alias = "Auto")]
    FromFile,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Units {
    FromFile,
    Meters,
    Centimeters,
    Millimeters,
    Feet,
    Inches,
}

impl Units {
    pub const ALL: [Units; 6] = [Units::FromFile, Units::Meters, Units::Centimeters, Units::Millimeters, Units::Feet, Units::Inches];
    pub fn scale(self) -> Option<f32> {
        Some(match self {
            Units::FromFile => return None,
            Units::Meters => 1.0,
            Units::Centimeters => 0.01,
            Units::Millimeters => 0.001,
            Units::Feet => 0.3048,
            Units::Inches => 0.0254,
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            Units::FromFile => "From file",
            Units::Meters => "Meters",
            Units::Centimeters => "Centimeters",
            Units::Millimeters => "Millimeters",
            Units::Feet => "Feet",
            Units::Inches => "Inches",
        }
    }
    pub fn parse(s: &str) -> Option<Units> {
        Some(match s.to_ascii_lowercase().as_str() {
            "m" | "meter" | "meters" | "metres" => Units::Meters,
            "cm" | "centimeters" => Units::Centimeters,
            "mm" | "millimeters" => Units::Millimeters,
            "ft" | "feet" | "foot" => Units::Feet,
            "in" | "inch" | "inches" => Units::Inches,
            _ => return None,
        })
    }
}

/// What a file says about itself (FBX GlobalSettings) or its format's convention.
#[derive(Clone, Copy, Debug, Default)]
pub struct FileHints {
    pub up: Option<UpAxis>,
    pub scale_to_m: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportSettings {
    pub up: UpAxis,
    pub units: Units,
    /// Move the model so it sits on the ground at the origin.
    pub recenter: bool,
    /// Offset computed on first load; kept so re-syncs don't jump around.
    pub offset: Option<[f32; 3]>,
    /// Extra folders searched for textures the model references.
    #[serde(default)]
    pub texture_dirs: Vec<std::path::PathBuf>,
}

fn ext_of(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

impl ImportSettings {
    pub fn default_for(path: &Path) -> Self {
        let ext = ext_of(path);
        let (up, units) = match ext.as_str() {
            // FBX declares its own units/axis (Revit writes feet, Z-up).
            "fbx" => (UpAxis::FromFile, Units::FromFile),
            // Revit OBJ exporters write internal units (feet), Z-up.
            "obj" => (UpAxis::Z, Units::Feet),
            // glTF is Y-up meters by spec.
            _ => (UpAxis::Y, Units::Meters),
        };
        Self { up, units, recenter: true, offset: None, texture_dirs: Vec::new() }
    }

    pub fn basis_with(&self, hints: FileHints) -> Mat3 {
        let s = self.units.scale().or(hints.scale_to_m.map(|v| v as f32)).unwrap_or(1.0);
        let up = match self.up {
            UpAxis::FromFile => hints.up.unwrap_or(UpAxis::Y),
            u => u,
        };
        let axis = match up {
            UpAxis::Z => Mat3::from_cols(Vec3::X, -Vec3::Z, Vec3::Y),
            _ => Mat3::IDENTITY,
        };
        // (x, y, z) z-up  ->  (x, z, -y) y-up
        axis * Mat3::from_diagonal(Vec3::splat(s))
    }

    pub fn basis(&self) -> Mat3 {
        self.basis_with(FileHints::default())
    }
}

pub struct ImportedModel {
    pub meshes: Vec<(String, MeshData)>,
    pub materials: Vec<Material>,
    /// Offset that was applied (to store back in settings).
    pub offset: Vec3,
    /// Texture files the model references that couldn't be found.
    pub missing_textures: Vec<String>,
}

pub fn is_supported(path: &Path) -> bool {
    matches!(ext_of(path).as_str(), "gltf" | "glb" | "obj" | "fbx")
}

/// Raw loader output: individual parts in the file's own coordinates.
type Parts = (Vec<MeshData>, Vec<Material>, FileHints);

/// Import a model. Parser bugs on malformed files are reported as errors
/// rather than taking the application down (the parsers also validate
/// sizes, offsets and nesting so this is a second line of defence).
pub fn load(path: &Path, settings: &ImportSettings) -> Result<ImportedModel> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load_unguarded(path, settings))) {
        Ok(r) => r,
        Err(p) => {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            anyhow::bail!("Could not import {}: the file appears to be corrupt or unsupported ({msg})", path.display())
        }
    }
}

fn load_unguarded(path: &Path, settings: &ImportSettings) -> Result<ImportedModel> {
    let mut textures = TextureResolver::new(path, &settings.texture_dirs);
    let (mut parts, materials, hints) = match ext_of(path).as_str() {
        "gltf" | "glb" => load_gltf(path)?,
        "obj" => load_obj(path, &mut textures)?,
        "fbx" => {
            let f = crate::fbx::load(path, &mut textures)?;
            (f.meshes.into_iter().map(|(_, m)| m).collect(), f.materials, f.hints)
        }
        _ => anyhow::bail!("Unsupported file type: {}", path.display()),
    };

    // Unit + axis conversion.
    let basis = settings.basis_with(hints);
    let nbasis = basis.inverse().transpose();
    for m in &mut parts {
        for p in &mut m.positions {
            *p = basis * *p;
        }
        for n in &mut m.normals {
            *n = (nbasis * *n).normalize_or_zero();
        }
    }

    let offset = if let Some(o) = settings.offset {
        Vec3::from(o)
    } else if settings.recenter {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for m in &parts {
            for p in &m.positions {
                lo = lo.min(*p);
                hi = hi.max(*p);
            }
        }
        if lo.x <= hi.x {
            Vec3::new(-(lo.x + hi.x) * 0.5, -lo.y, -(lo.z + hi.z) * 0.5)
        } else {
            Vec3::ZERO
        }
    } else {
        Vec3::ZERO
    };
    for m in &mut parts {
        if offset != Vec3::ZERO {
            for p in &mut m.positions {
                *p += offset;
            }
        }
        // Fill missing normals / box-projected UVs (1 tile per metre).
        m.finalize();
    }

    let transparent: std::collections::HashSet<&str> = materials.iter().filter(|m| m.params.kind.is_transparent()).map(|m| m.name.as_str()).collect();
    let meshes = chunk(parts, &transparent);
    Ok(ImportedModel { meshes, materials, offset, missing_textures: textures.missing })
}

/// Group parts by (material, spatial cell). One draw per chunk keeps draw
/// calls low, while cells keep re-syncs incremental (an edited wall only
/// re-uploads its cell), enable frustum culling, fast picking, and correct
/// back-to-front sorting of glass.
pub fn chunk(parts: Vec<MeshData>, transparent: &std::collections::HashSet<&str>) -> Vec<(String, MeshData)> {
    const MAX_TRIS: usize = 150_000;
    // Cell size adapts to the site: ~16 cells across, never smaller than 16 m.
    // Cells are 2D (plan view): splitting buildings by floor doesn't help.
    let (mut lo_all, mut hi_all) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in &parts {
        if let Some((lo, hi)) = p.bounds() {
            lo_all = lo_all.min(lo);
            hi_all = hi_all.max(hi);
        }
    }
    let extent = (hi_all - lo_all).max_element().max(1.0);
    let opaque_cell = (extent / 16.0).clamp(16.0, 250.0);
    let glass_cell = (opaque_cell / 3.0).max(6.0);
    let mut cells: BTreeMap<(String, i32, i32, i32), Vec<MeshData>> = BTreeMap::new();
    for p in parts {
        let Some((lo, hi)) = p.bounds() else { continue };
        if p.indices.is_empty() {
            continue;
        }
        let size = if transparent.contains(p.material.as_str()) { glass_cell } else { opaque_cell };
        if (hi - lo).x.max((hi - lo).z) > size * 2.0 {
            // One huge part (e.g. a whole model exported as a single object):
            // split it per triangle so it still gets spatial chunks.
            for (key, piece) in split_by_cell(&p, size) {
                cells.entry((p.material.clone(), key.0, key.1, key.2)).or_default().push(piece);
            }
            continue;
        }
        let c = ((lo + hi) * 0.5 / size).floor();
        cells.entry((p.material.clone(), c.x as i32, 0, c.z as i32)).or_default().push(p);
    }
    let mut out = Vec::new();
    for ((mat, x, y, z), list) in cells {
        let mut sub = 0;
        let mut cur = MeshData { name: mat.clone(), material: mat.clone(), ..Default::default() };
        for p in list {
            if !cur.indices.is_empty() && cur.indices.len() / 3 + p.indices.len() / 3 > MAX_TRIS {
                out.push((
                    format!("file:{mat}@{x},{y},{z}#{sub}"),
                    std::mem::replace(&mut cur, MeshData { name: mat.clone(), material: mat.clone(), ..Default::default() }),
                ));
                sub += 1;
            }
            let base = cur.positions.len() as u32;
            cur.positions.extend_from_slice(&p.positions);
            cur.normals.extend_from_slice(&p.normals);
            cur.uvs.extend_from_slice(&p.uvs);
            cur.indices.extend(p.indices.iter().map(|i| i + base));
        }
        if !cur.indices.is_empty() {
            out.push((format!("file:{mat}@{x},{y},{z}#{sub}"), cur));
        }
    }
    out
}

fn split_by_cell(p: &MeshData, size: f32) -> Vec<((i32, i32, i32), MeshData)> {
    type Cell = (MeshData, std::collections::HashMap<u32, u32>);
    let mut out: BTreeMap<(i32, i32, i32), Cell> = BTreeMap::new();
    for t in p.indices.chunks_exact(3) {
        let c = (p.positions[t[0] as usize] + p.positions[t[1] as usize] + p.positions[t[2] as usize]) / 3.0;
        let k = (c / size).floor();
        let (m, remap) = out
            .entry((k.x as i32, 0, k.z as i32))
            .or_insert_with(|| (MeshData { name: p.name.clone(), material: p.material.clone(), ..Default::default() }, Default::default()));
        for &i in t {
            let ni = *remap.entry(i).or_insert_with(|| {
                m.positions.push(p.positions[i as usize]);
                m.normals.push(p.normals[i as usize]);
                m.uvs.push(p.uvs[i as usize]);
                (m.positions.len() - 1) as u32
            });
            m.indices.push(ni);
        }
    }
    out.into_iter().map(|(k, (m, _))| (k, m)).collect()
}

// ------------------------------------------------------------------ textures

/// Finds the texture files a model references. Models (Revit FBX especially)
/// often store absolute paths from the machine that exported them, so after
/// the literal path we look next to the model, in its `.fbm` folder, in any
/// sub-folder, in user texture folders and in the Autodesk material library.
pub struct TextureResolver {
    model_dir: PathBuf,
    fbm_dir: PathBuf,
    extra: Vec<PathBuf>,
    index: Option<std::collections::HashMap<String, PathBuf>>,
    cache: std::collections::HashMap<String, Option<TextureData>>,
    pub missing: Vec<String>,
}

fn autodesk_library_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for base in ["C:/Program Files (x86)/Common Files/Autodesk Shared/Materials", "C:/Program Files/Common Files/Autodesk Shared/Materials"] {
        let p = PathBuf::from(base);
        if p.is_dir() {
            v.push(p);
        }
    }
    v
}

impl TextureResolver {
    pub fn new(model: &Path, extra: &[PathBuf]) -> Self {
        let model_dir = model.parent().unwrap_or(Path::new(".")).to_path_buf();
        let stem = model.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        Self { fbm_dir: model_dir.join(format!("{stem}.fbm")), model_dir, extra: extra.to_vec(), index: None, cache: Default::default(), missing: Vec::new() }
    }

    fn build_index(&mut self) -> &std::collections::HashMap<String, PathBuf> {
        if self.index.is_none() {
            let mut idx = std::collections::HashMap::new();
            let mut roots: Vec<(PathBuf, usize)> = vec![(self.model_dir.clone(), 4)];
            roots.extend(self.extra.iter().map(|d| (d.clone(), 6)));
            roots.extend(autodesk_library_dirs().into_iter().map(|d| (d, 6)));
            let mut budget = 200_000usize;
            for (root, depth) in roots {
                let mut stack = vec![(root, 0usize)];
                while let Some((dir, d)) = stack.pop() {
                    let Ok(rd) = std::fs::read_dir(&dir) else { continue };
                    for e in rd.flatten() {
                        if budget == 0 {
                            break;
                        }
                        budget -= 1;
                        let p = e.path();
                        if p.is_dir() {
                            if d < depth {
                                stack.push((p, d + 1));
                            }
                        } else if is_image(&p) {
                            if let Some(n) = p.file_name() {
                                idx.entry(n.to_string_lossy().to_lowercase()).or_insert(p);
                            }
                        }
                    }
                }
            }
            self.index = Some(idx);
        }
        self.index.as_ref().unwrap()
    }

    /// Locate a referenced file name/path on disk.
    pub fn resolve(&mut self, reference: &str) -> Option<PathBuf> {
        let r = reference.trim().replace('\\', "/");
        if r.is_empty() {
            return None;
        }
        let base = r.rsplit('/').next().unwrap_or(&r).to_string();
        let candidates = [
            PathBuf::from(&r),
            self.model_dir.join(&r),
            self.model_dir.join(&base),
            self.fbm_dir.join(&base),
            self.model_dir.join("textures").join(&base),
            self.model_dir.join("Textures").join(&base),
        ];
        for c in candidates {
            if c.is_file() {
                return Some(c);
            }
        }
        for d in &self.extra {
            let c = d.join(&base);
            if c.is_file() {
                return Some(c);
            }
        }
        let key = base.to_lowercase();
        if let Some(p) = self.build_index().get(&key) {
            return Some(p.clone());
        }
        // Same name, different image format (e.g. .tga referenced, .png shipped).
        let stem = key.rsplit_once('.').map(|(a, _)| a.to_string()).unwrap_or(key);
        let idx = self.build_index();
        for ext in ["png", "jpg", "jpeg", "tga", "bmp", "tif", "tiff", "dds", "webp"] {
            if let Some(p) = idx.get(&format!("{stem}.{ext}")) {
                return Some(p.clone());
            }
        }
        None
    }

    /// Resolve and decode, caching by reference; records misses.
    pub fn load(&mut self, reference: &str) -> Option<TextureData> {
        if let Some(t) = self.cache.get(reference) {
            return t.clone();
        }
        let t = self.resolve(reference).and_then(|p| load_texture_file(&p));
        if t.is_none() && !self.missing.iter().any(|m| m == reference) {
            self.missing.push(reference.to_string());
        }
        self.cache.insert(reference.to_string(), t.clone());
        t
    }
}

pub fn is_image(p: &Path) -> bool {
    matches!(ext_of(p).as_str(), "png" | "jpg" | "jpeg" | "tga" | "bmp" | "tif" | "tiff" | "gif" | "webp" | "dds")
}

/// Largest texture side kept in memory (bigger images are downscaled).
pub const MAX_TEXTURE: u32 = 4096;

pub fn texture_from_image(img: image::DynamicImage) -> TextureData {
    let img =
        if img.width() > MAX_TEXTURE || img.height() > MAX_TEXTURE { img.resize(MAX_TEXTURE, MAX_TEXTURE, image::imageops::FilterType::Triangle) } else { img };
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    TextureData::new(w, h, rgba.into_raw())
}

pub fn texture_from_bytes(bytes: &[u8]) -> Option<TextureData> {
    image::load_from_memory(bytes).ok().map(texture_from_image)
}

pub fn load_texture_file(path: &Path) -> Option<TextureData> {
    // Some exporters write .tga/.dds with the wrong extension; sniff content first.
    let bytes = std::fs::read(path).ok()?;
    if let Ok(img) = image::load_from_memory(&bytes) {
        return Some(texture_from_image(img));
    }
    image::open(path).ok().map(texture_from_image)
}

/// True when the image is (nearly) grey everywhere — i.e. a bump/height map
/// rather than a tangent-space normal map.
pub fn is_grayscale(t: &TextureData) -> bool {
    let n = (t.rgba.len() / 4).max(1);
    let step = (n / 4096).max(1);
    let (mut grey, mut total) = (0usize, 0usize);
    for (i, p) in t.rgba.chunks_exact(4).enumerate() {
        if i % step != 0 {
            continue;
        }
        total += 1;
        let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
        if (r - g).abs() < 8 && (g - b).abs() < 8 {
            grey += 1;
        }
    }
    grey * 100 >= total * 97
}

/// Convert a height (bump) map into a tangent-space normal map
/// (OpenGL convention, +Y = up in the image).
pub fn bump_to_normal(t: &TextureData, strength: f32) -> TextureData {
    let (w, h) = (t.width as i64, t.height as i64);
    let hgt = |x: i64, y: i64| -> f32 {
        let x = x.rem_euclid(w);
        let y = y.rem_euclid(h);
        t.rgba[((y * w + x) * 4) as usize] as f32 / 255.0
    };
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let dx = (hgt(x + 1, y) - hgt(x - 1, y)) * strength;
            let dy = (hgt(x, y + 1) - hgt(x, y - 1)) * strength;
            let n = Vec3::new(-dx, dy, 1.0).normalize();
            out.extend_from_slice(&[((n.x * 0.5 + 0.5) * 255.0) as u8, ((n.y * 0.5 + 0.5) * 255.0) as u8, ((n.z * 0.5 + 0.5) * 255.0) as u8, 255]);
        }
    }
    TextureData::new(t.width, t.height, out)
}

/// Use a normal map as-is, or convert it if it's actually a bump map.
pub fn as_normal_map(t: TextureData) -> TextureData {
    if is_grayscale(&t) {
        bump_to_normal(&t, 4.0)
    } else {
        t
    }
}

/// Bake an opacity/transparency map into the colour map's alpha channel.
/// `invert` for "transparency" maps (white = see-through).
pub fn merge_opacity(color: Option<TextureData>, opacity: &TextureData, invert: bool) -> TextureData {
    let (w, h) = color.as_ref().map_or((opacity.width, opacity.height), |c| (c.width, c.height));
    let op = if (opacity.width, opacity.height) == (w, h) {
        opacity.rgba.to_vec()
    } else {
        let img = image::RgbaImage::from_raw(opacity.width, opacity.height, opacity.rgba.to_vec()).unwrap();
        image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle).into_raw()
    };
    let mut out = match &color {
        Some(c) => c.rgba.to_vec(),
        None => vec![255u8; (w * h * 4) as usize],
    };
    for (o, src) in out.chunks_exact_mut(4).zip(op.chunks_exact(4)) {
        // Opacity maps are usually greyscale; if one has real alpha, use it.
        let v = if src[3] < 250 { src[3] } else { ((src[0] as u32 + src[1] as u32 + src[2] as u32) / 3) as u8 };
        o[3] = if invert { 255 - v } else { v };
    }
    TextureData::new(w, h, out)
}

/// Fraction of pixels that are mostly transparent.
pub fn transparent_fraction(t: &TextureData) -> f32 {
    let n = (t.rgba.len() / 4).max(1);
    let clear = t.rgba.chunks_exact(4).filter(|p| p[3] < 128).count();
    clear as f32 / n as f32
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn load_gltf(path: &Path) -> Result<Parts> {
    let (doc, buffers, images) = gltf::import(path).with_context(|| format!("reading {}", path.display()))?;

    // Materials
    let mat_name = |m: &gltf::Material| -> String { m.name().map(str::to_string).unwrap_or_else(|| format!("Material {}", m.index().map_or(0, |i| i))) };
    let mut materials = Vec::new();
    let mut tex_cache: BTreeMap<usize, TextureData> = BTreeMap::new();
    for m in doc.materials() {
        let pbr = m.pbr_metallic_roughness();
        let name = mat_name(&m);
        let bc = pbr.base_color_factor();
        let mut p = MaterialParams {
            base_color: bc,
            metallic: pbr.metallic_factor(),
            roughness: pbr.roughness_factor(),
            emissive: m.emissive_factor(),
            emissive_strength: if m.emissive_factor().iter().any(|c| *c > 0.0) { m.emissive_strength().unwrap_or(1.0) } else { 0.0 },
            ..Default::default()
        };
        let transmissive = m.transmission().is_some_and(|t| t.transmission_factor() > 0.1);
        if transmissive || (m.alpha_mode() == gltf::material::AlphaMode::Blend && bc[3] < 0.95) {
            p.kind = MaterialKind::Glass;
            p.roughness = p.roughness.min(0.1);
            if bc[3] > 0.6 {
                p.base_color[3] = 0.3;
            }
        } else if p.emissive_strength > 0.0 {
            p.kind = MaterialKind::Emissive;
        } else {
            let before = p.clone();
            p = guess_params_from_name(&name, p);
            // Keep the exporter's colour unless the name said glass/water/etc.
            if p.kind == MaterialKind::Standard {
                p.base_color = before.base_color;
            }
        }
        let mut get = |src: usize| -> Option<TextureData> {
            if let Some(t) = tex_cache.get(&src) {
                return Some(t.clone());
            }
            let t = gltf_image_to_rgba(images.get(src)?)?;
            tex_cache.insert(src, t.clone());
            Some(t)
        };
        let texture = pbr.base_color_texture().and_then(|info| get(info.texture().source().index()));
        let normal_texture = m.normal_texture().and_then(|n| {
            p.normal_strength = n.scale();
            get(n.texture().source().index()).map(as_normal_map)
        });
        if m.alpha_mode() == gltf::material::AlphaMode::Mask
            || (p.kind != MaterialKind::Glass && texture.as_ref().is_some_and(|t| transparent_fraction(t) > 0.05))
        {
            p.cutout = true;
        }
        // glTF packs roughness in G and metalness in B of one texture.
        let mr = pbr.metallic_roughness_texture().and_then(|info| get(info.texture().source().index()));
        let roughness_texture = mr.as_ref().map(|t| {
            let n = (t.rgba.len() / 4).max(1) as f32;
            let metal = t.rgba.chunks_exact(4).map(|px| px[2] as f32).sum::<f32>() / n / 255.0;
            p.metallic = (p.metallic * metal).clamp(0.0, 1.0);
            if metal < 0.1 {
                p.metallic = 0.0;
            }
            let mut g = Vec::with_capacity(t.rgba.len());
            for px in t.rgba.chunks_exact(4) {
                g.extend_from_slice(&[px[1], px[1], px[1], 255]);
            }
            p.roughness = 0.5 + (p.roughness - 1.0) * 0.5; // 0.5 = the map as-is
            TextureData::new(t.width, t.height, g)
        });
        materials.push(Material { name, params: p, texture, normal_texture, roughness_texture });
    }
    let default_mat = "Default".to_string();

    let mut parts: Vec<MeshData> = Vec::new();
    let scene = doc.default_scene().or_else(|| doc.scenes().next());
    let mut stack: Vec<(gltf::Node, Mat4)> = match &scene {
        Some(s) => s.nodes().map(|n| (n, Mat4::IDENTITY)).collect(),
        None => doc.nodes().map(|n| (n, Mat4::IDENTITY)).collect(),
    };
    while let Some((node, parent)) = stack.pop() {
        let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        let nmat = Mat3::from_mat4(world).inverse().transpose();
        if let Some(mesh) = node.mesh() {
            for prim in mesh.primitives() {
                if prim.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let r = prim.reader(|b| Some(&buffers[b.index()]));
                let Some(pos) = r.read_positions() else { continue };
                let pos: Vec<Vec3> = pos.map(|p| world.transform_point3(Vec3::from(p))).collect();
                let nrm: Option<Vec<Vec3>> = r.read_normals().map(|it| it.map(|n| (nmat * Vec3::from(n)).normalize_or_zero()).collect());
                let uv: Option<Vec<Vec2>> = r.read_tex_coords(0).map(|t| t.into_f32().map(Vec2::from).collect());
                let mut idx: Vec<u32> = r.read_indices().map(|i| i.into_u32().collect()).unwrap_or_else(|| (0..pos.len() as u32).collect());
                if world.determinant() < 0.0 {
                    for t in idx.chunks_exact_mut(3) {
                        t.swap(1, 2);
                    }
                }
                let mname = prim.material().index().map(|_| mat_name(&prim.material())).unwrap_or_else(|| default_mat.clone());
                parts.push(MeshData {
                    name: mesh.name().unwrap_or("mesh").to_string(),
                    material: mname,
                    positions: pos,
                    normals: nrm.unwrap_or_default(),
                    uvs: uv.unwrap_or_default(),
                    indices: idx,
                });
            }
        }
        for c in node.children() {
            stack.push((c, world));
        }
    }
    Ok((parts, materials, FileHints { up: Some(UpAxis::Y), scale_to_m: Some(1.0) }))
}

fn gltf_image_to_rgba(img: &gltf::image::Data) -> Option<TextureData> {
    use gltf::image::Format;
    let px = (img.width * img.height) as usize;
    let rgba: Vec<u8> = match img.format {
        Format::R8G8B8A8 => img.pixels.clone(),
        Format::R8G8B8 => img.pixels.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        Format::R8 => img.pixels.iter().flat_map(|&c| [c, c, c, 255]).collect(),
        Format::R8G8 => img.pixels.chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        _ => return None,
    };
    if rgba.len() != px * 4 {
        return None;
    }
    Some(TextureData::new(img.width, img.height, rgba))
}

/// MTL map statements may carry options (`-s 2 2 1 -bm 0.5 brick.jpg`).
/// Returns (file, uv scale, uv offset, bump multiplier).
fn parse_mtl_map(spec: &str) -> (String, [f32; 2], [f32; 2], f32) {
    let toks: Vec<&str> = spec.split_whitespace().collect();
    let (mut scale, mut off, mut bm) = ([1.0, 1.0], [0.0, 0.0], 1.0);
    let mut i = 0;
    let num = |t: Option<&&str>| t.and_then(|v| v.parse::<f32>().ok());
    while i < toks.len() && toks[i].starts_with('-') && toks[i].len() > 1 && !toks[i][1..].starts_with(|c: char| c.is_ascii_digit()) {
        let opt = toks[i];
        let takes = match opt {
            "-s" | "-o" | "-t" => 3,
            "-mm" => 2,
            "-bm" | "-boost" | "-texres" | "-clamp" | "-blendu" | "-blendv" | "-cc" | "-imfchan" | "-type" => 1,
            _ => 1,
        };
        if opt == "-s" {
            scale = [num(toks.get(i + 1)).unwrap_or(1.0), num(toks.get(i + 2)).unwrap_or(1.0)];
        } else if opt == "-o" {
            off = [num(toks.get(i + 1)).unwrap_or(0.0), num(toks.get(i + 2)).unwrap_or(0.0)];
        } else if opt == "-bm" {
            bm = num(toks.get(i + 1)).unwrap_or(1.0);
        }
        // -s/-o/-t take 1-3 numbers; skip only the numeric ones.
        let mut k = 0;
        while k < takes && toks.get(i + 1).is_some_and(|t| t.parse::<f32>().is_ok() || matches!(*t, "on" | "off" | "r" | "g" | "b" | "m" | "l" | "z")) {
            i += 1;
            k += 1;
        }
        i += 1;
    }
    (toks[i.min(toks.len())..].join(" "), scale, off, bm)
}

fn load_obj(path: &Path, textures: &mut TextureResolver) -> Result<Parts> {
    let opts = tobj::LoadOptions { triangulate: true, single_index: true, ..Default::default() };
    let (models, mats) = tobj::load_obj(path, &opts).with_context(|| format!("reading {}", path.display()))?;
    let mats = mats.unwrap_or_default();
    let mut materials = Vec::new();
    for m in &mats {
        let kd = m.diffuse.unwrap_or([0.8, 0.8, 0.8]);
        let opacity = m.dissolve.unwrap_or(1.0);
        let ns = m.shininess.unwrap_or(10.0).max(0.0);
        let mut p = MaterialParams {
            base_color: [srgb_to_linear(kd[0]), srgb_to_linear(kd[1]), srgb_to_linear(kd[2]), opacity],
            roughness: (2.0 / (ns + 2.0)).sqrt().clamp(0.03, 1.0),
            ..Default::default()
        };
        let before = p.clone();
        p = guess_params_from_name(&m.name, p);
        if p.kind == MaterialKind::Standard {
            p.base_color = before.base_color;
            if opacity < 0.95 {
                p.kind = MaterialKind::Glass;
            }
        }
        let mut texture = m.diffuse_texture.as_ref().and_then(|spec| {
            let (file, sc, off, _) = parse_mtl_map(spec);
            // MTL v points up; we flip v on import, so mirror the offset too.
            p.uv_transform = [sc[0], sc[1], off[0], 1.0 - sc[1] - off[1]];
            textures.load(&file)
        });
        let normal_texture = m.normal_texture.as_ref().and_then(|spec| {
            let (file, _, _, bm) = parse_mtl_map(spec);
            p.normal_strength = bm;
            textures.load(&file).map(as_normal_map)
        });
        if let Some(op) = m.dissolve_texture.as_ref().and_then(|spec| textures.load(&parse_mtl_map(spec).0)) {
            texture = Some(merge_opacity(texture, &op, false));
            if p.kind != MaterialKind::Glass {
                p.cutout = true;
            }
        }
        if texture.is_some() && p.kind == MaterialKind::Standard {
            p.base_color = [1.0, 1.0, 1.0, opacity];
        }
        if p.kind != MaterialKind::Glass && texture.as_ref().is_some_and(|t| transparent_fraction(t) > 0.05) {
            p.cutout = true;
        }
        materials.push(Material { name: m.name.clone(), params: p, texture, normal_texture, roughness_texture: None });
    }

    let mut parts: Vec<MeshData> = Vec::new();
    for model in &models {
        let mesh = &model.mesh;
        let pos: Vec<Vec3> = mesh.positions.chunks_exact(3).map(|c| Vec3::new(c[0], c[1], c[2])).collect();
        let nrm: Vec<Vec3> = mesh.normals.chunks_exact(3).map(|c| Vec3::new(c[0], c[1], c[2])).collect();
        let uv: Vec<Vec2> = mesh.texcoords.chunks_exact(2).map(|c| Vec2::new(c[0], 1.0 - c[1])).collect();
        let mname = mesh.material_id.and_then(|i| mats.get(i)).map(|m| m.name.clone()).unwrap_or_else(|| "Default".into());
        parts.push(MeshData { name: model.name.clone(), material: mname, positions: pos, normals: nrm, uvs: uv, indices: mesh.indices.clone() });
    }
    Ok((parts, materials, FileHints { up: Some(UpAxis::Z), scale_to_m: Some(0.3048) }))
}
