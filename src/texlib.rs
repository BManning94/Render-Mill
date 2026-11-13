//! The user's texture library: images imported from disk, grouped into sets
//! (colour + normal + roughness) by the usual naming conventions, ready to be
//! applied to any material.

use crate::scene::{MaterialKind, MaterialParams, TextureData};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextureSet {
    pub name: String,
    pub color: Option<String>,
    pub normal: Option<String>,
    pub roughness: Option<String>,
    /// Real-world size of one tile, in metres.
    #[serde(default = "one")]
    pub size_m: f32,
}

fn one() -> f32 {
    1.0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Color,
    Normal,
    /// DirectX-style normal map (green flipped).
    NormalDx,
    Roughness,
    /// Glossiness map (inverted roughness).
    Gloss,
    Ignore,
}

/// Classify an image by its file name. Returns (set name, role).
pub fn classify(path: &Path) -> (String, Role) {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let low = stem.to_ascii_lowercase();
    // Previews and baked-shadow variants aren't texture maps.
    if low.contains("preview") || low.contains("with_shadows") || low.contains("with shadows") || low.contains("thumbnail") || low.contains("swatch") {
        return (stem, Role::Ignore);
    }
    let tokens: Vec<String> = stem.split(['_', '-', ' ', '.']).filter(|t| !t.is_empty()).map(|t| t.to_string()).collect();
    let is_noise = |t: &str| {
        let l = t.to_ascii_lowercase();
        // resolution / format tokens: 1k, 2K, 4096, jpg, png ...
        (l.ends_with('k') && l[..l.len() - 1].chars().all(|c| c.is_ascii_digit()) && l.len() > 1)
            || matches!(l.as_str(), "jpg" | "jpeg" | "png" | "tif" | "tiff" | "tga" | "exr" | "1024" | "2048" | "4096" | "8192" | "512")
    };
    let role_of = |t: &str, next: Option<&str>| -> Option<Role> {
        let l = t.to_ascii_lowercase();
        let n = next.map(|s| s.to_ascii_lowercase());
        Some(match l.as_str() {
            "albedo" | "diffuse" | "diff" | "basecolor" | "basecolour" | "color" | "colour" | "col" | "alb" | "base" | "dif" => Role::Color,
            "normalgl" | "nrmgl" => Role::Normal,
            "normaldx" | "nrmdx" => Role::NormalDx,
            "normal" | "nrm" | "nor" | "norm" | "normals" => match n.as_deref() {
                Some("dx") => Role::NormalDx,
                _ => Role::Normal,
            },
            "roughness" | "rough" | "rgh" => Role::Roughness,
            "glossiness" | "gloss" | "gls" => Role::Gloss,
            "ao" | "ambientocclusion" | "occlusion" | "height" | "disp" | "displacement" | "bump" | "metal" | "metallic" | "metalness" | "spec"
            | "specular" | "opacity" | "mask" | "arm" | "orm" | "reflectivity" | "reflection" | "refl" | "emissive" | "emission" | "translucency" | "sheen" => {
                Role::Ignore
            }
            _ => return None,
        })
    };
    // Find the last token that names a role; everything before it is the set name.
    let mut found: Option<(usize, Role)> = None;
    for i in (0..tokens.len()).rev() {
        if let Some(r) = role_of(&tokens[i], tokens.get(i + 1).map(|s| s.as_str())) {
            found = Some((i, r));
            break;
        }
    }
    match found {
        Some((i, r)) if i > 0 => {
            let name: Vec<&str> = tokens[..i].iter().filter(|t| !is_noise(t)).map(|s| s.as_str()).collect();
            (name.join(" "), r)
        }
        Some((_, r)) => (stem.clone(), r),
        None => {
            let name: Vec<&str> = tokens.iter().filter(|t| !is_noise(t)).map(|s| s.as_str()).collect();
            (if name.is_empty() { stem } else { name.join(" ") }, Role::Color)
        }
    }
}

/// Group image files into texture sets.
pub fn group(paths: &[PathBuf]) -> Vec<TextureSet> {
    let mut sets: BTreeMap<String, TextureSet> = BTreeMap::new();
    for p in paths.iter().filter(|p| crate::import::is_image(p)) {
        let (name, role) = classify(p);
        if role == Role::Ignore {
            continue;
        }
        let key = name.to_lowercase();
        let set = sets.entry(key).or_insert_with(|| TextureSet { name: tidy(&name), color: None, normal: None, roughness: None, size_m: 1.0 });
        let path = p.to_string_lossy().to_string();
        match role {
            Role::Color => set.color = set.color.take().or(Some(path)),
            // Mark DX maps so they're converted on load.
            Role::Normal => set.normal = Some(path),
            Role::NormalDx => {
                if set.normal.is_none() {
                    set.normal = Some(format!("{path}#dx"));
                }
            }
            Role::Roughness => set.roughness = Some(path),
            Role::Gloss => {
                if set.roughness.is_none() {
                    set.roughness = Some(format!("{path}#gloss"));
                }
            }
            Role::Ignore => {}
        }
    }
    sets.into_values().filter(|s| s.color.is_some() || s.normal.is_some()).collect()
}

/// Images in a folder (and one level of sub-folders, like most downloads).
pub fn scan_folder(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 2 {
                    stack.push((p, depth + 1));
                }
            } else if crate::import::is_image(&p) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn tidy(name: &str) -> String {
    let mut s: String = name.chars().map(|c| if c == '_' || c == '-' { ' ' } else { c }).collect();
    if let Some(f) = s.get_mut(0..1) {
        f.make_ascii_uppercase();
    }
    s.trim().to_string()
}

/// Map kinds stored in `Scene::user_textures`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MapKind {
    Color,
    Normal,
    Roughness,
}

/// Load an image for a map slot, applying the conversions its name implies
/// (`#dx` normal maps get green flipped, `#gloss` maps are inverted, grey bump
/// maps become normal maps).
pub fn load_map(spec: &str, kind: MapKind) -> Option<TextureData> {
    let (path, tag) = match spec.rsplit_once('#') {
        Some((p, t)) if matches!(t, "dx" | "gloss") => (p, Some(t)),
        _ => (spec, None),
    };
    let t = crate::import::load_texture_file(Path::new(path))?;
    Some(match kind {
        MapKind::Color => t,
        MapKind::Normal => {
            let t = crate::import::as_normal_map(t);
            if tag == Some("dx") {
                let mut px = t.rgba.to_vec();
                for p in px.chunks_exact_mut(4) {
                    p[1] = 255 - p[1];
                }
                TextureData::new(t.width, t.height, px)
            } else {
                t
            }
        }
        MapKind::Roughness => {
            // Keep a single grey channel; invert gloss maps.
            let mut px = t.rgba.to_vec();
            for p in px.chunks_exact_mut(4) {
                let v = if tag == Some("gloss") { 255 - p[0] } else { p[0] };
                p[0] = v;
                p[1] = v;
                p[2] = v;
                p[3] = 255;
            }
            TextureData::new(t.width, t.height, px)
        }
    })
}

/// Material parameters for a material using this set (keeps the old kind
/// unless it was glass/water/glow).
pub fn apply(set: &TextureSet, old: &MaterialParams) -> MaterialParams {
    let mut p = old.clone();
    if matches!(p.kind, MaterialKind::Glass | MaterialKind::Water | MaterialKind::Emissive) {
        p.kind = MaterialKind::Standard;
        p.emissive_strength = 0.0;
    }
    if let Some(c) = &set.color {
        p.color_map = Some(c.clone());
        p.use_texture = true;
        p.base_color = [1.0, 1.0, 1.0, 1.0];
    }
    p.normal_map = set.normal.clone();
    if set.normal.is_some() && p.normal_strength <= 0.0 {
        p.normal_strength = 1.0;
    }
    p.roughness_map = set.roughness.clone();
    if set.roughness.is_some() {
        p.roughness = 0.5; // 0.5 = use the map as-is; the slider offsets it
    }
    p.uv_scale = 1.0 / set.size_m.max(0.01);
    // Real textures carry their own detail; keep only a hint of procedural breakup.
    p.variation = p.variation.min(0.15);
    p.uv_transform = [1.0, 1.0, 0.0, 0.0];
    // Library textures have a real-world size: place them in metres.
    p.box_mapping = true;
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    fn c(n: &str) -> (String, Role) {
        classify(Path::new(n))
    }
    #[test]
    fn naming_conventions() {
        assert_eq!(c("brick_wall_001_diff_4k.jpg"), ("brick wall 001".into(), Role::Color));
        assert_eq!(c("brick_wall_001_nor_gl_4k.jpg").1, Role::Normal);
        assert_eq!(c("brick_wall_001_nor_dx_4k.jpg").1, Role::NormalDx);
        assert_eq!(c("brick_wall_001_rough_4k.jpg"), ("brick wall 001".into(), Role::Roughness));
        assert_eq!(c("Bricks076C_4K-JPG_Color.jpg"), ("Bricks076C".into(), Role::Color));
        assert_eq!(c("Bricks076C_4K-JPG_NormalGL.jpg"), ("Bricks076C".into(), Role::Normal));
        assert_eq!(c("Bricks076C_4K-JPG_NormalDX.jpg").1, Role::NormalDx);
        assert_eq!(c("Bricks076C_4K-JPG_Roughness.jpg").1, Role::Roughness);
        assert_eq!(c("Bricks076C_4K-JPG_AmbientOcclusion.jpg").1, Role::Ignore);
        assert_eq!(c("oak floor.png"), ("oak floor".into(), Role::Color));
        assert_eq!(c("Wood_Gloss.png").1, Role::Gloss);
    }
    #[test]
    fn grouping() {
        let files: Vec<PathBuf> = [
            "a/brick_wall_001_diff_4k.jpg",
            "a/brick_wall_001_nor_gl_4k.jpg",
            "a/brick_wall_001_rough_4k.jpg",
            "a/brick_wall_001_ao_4k.jpg",
            "a/Bricks076C_4K-JPG_Color.jpg",
            "a/Bricks076C_4K-JPG_NormalDX.jpg",
            "a/marble.png",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        let sets = group(&files);
        assert_eq!(sets.len(), 3, "{sets:?}");
        let b = sets.iter().find(|s| s.name == "Brick wall 001").unwrap();
        assert!(b.color.is_some() && b.normal.is_some() && b.roughness.is_some());
        let b2 = sets.iter().find(|s| s.name == "Bricks076C").unwrap();
        assert!(b2.normal.as_deref().unwrap().ends_with("#dx"));
    }
}
