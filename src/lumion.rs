//! Reading Lumion files: scenes (.lsf, and Lumion's quick-save .lqf) and
//! imported-model files from Lumion's library folder (.lib, with the
//! material settings in the matching .lib.lms).
//!
//! Lumion stores these as a key/value container ("Q3D0"): a run of typed
//! values (text, numbers, matrices, raw blocks) followed by a table of
//! path-like keys ("model.surfaces.[3].surface.name",
//! "sceneitems.[12].data.base.worldMatrix", ...) that point at the values.
//! Meshes are tagged vertex streams (VPPI positions, VNNI normals, VTD0 UVs,
//! PO32 indices). The format was worked out from the user's own files; only
//! data is read - nothing of Lumion's program is used.
//!
//! What comes across: the user's imported models (geometry, Revit material
//! names and colours, the Lumion material settings applied to them, and
//! textures the user added), cameras, the sun, and the positions of Lumion's
//! own library objects - which are Lumion's licensed content, so they are
//! replaced with Render Mill's nearest equivalent rather than copied.

use crate::env::Camera;
use crate::objects::{ObjectKind, PlacedObject};
use crate::scene::{Material, MaterialKind, MaterialParams, MeshData, TextureData};
use anyhow::{Context, Result};
use glam::{Mat4, Vec2, Vec3};
use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

/// One value of a Q3D container.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Text(String),
    Float(f32),
    Int(i64),
    Floats(Vec<f32>),
    /// Raw bytes (embedded files, meshes, textures): a range of the data.
    Bytes(Range<usize>),
}

/// A parsed key/value container. Byte ranges refer to the original data.
#[derive(Default, Debug)]
pub struct Dict {
    pub keys: HashMap<String, Value>,
}

fn u64_at(d: &[u8], i: usize) -> Option<u64> {
    d.get(i..i + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()))
}

fn u32_at(d: &[u8], i: usize) -> Option<u32> {
    d.get(i..i + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
}

fn utf16(b: &[u8]) -> String {
    let w: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&w)
}

impl Dict {
    /// Parse the container whose values start at `start` (just after "Q3D0").
    pub fn parse(d: &[u8], start: usize) -> Dict {
        let mut vals: HashMap<usize, Value> = HashMap::new();
        let mut i = start;
        // Values: [flag][type][u64 length][payload]; type 0xff = raw block.
        while i + 10 <= d.len() && (d[i] == 1 || (d[i] == 0 && d[i + 1] == 0xff)) {
            let off = i - start;
            let ty = d[i + 1];
            let Some(len) = u64_at(d, i + 2).map(|l| l as usize) else { break };
            let p = i + 10;
            if p + len > d.len() {
                break;
            }
            let raw = &d[p..p + len];
            let v = match ty {
                5 => Value::Text(utf16(raw)),
                1 if len == 4 => Value::Float(f32::from_le_bytes(raw.try_into().unwrap())),
                0 if len == 8 => Value::Int(i64::from_le_bytes(raw.try_into().unwrap())),
                2 | 4 => Value::Floats(raw.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()),
                _ => Value::Bytes(p..p + len),
            };
            vals.insert(off, v);
            i = p + len;
        }
        // Keys: [u64 length][UTF-16 key][u64 value offset + 4].
        let mut keys = HashMap::new();
        while let Some(len) = u64_at(d, i) {
            let len = len as usize;
            if len == 0 || len > 4096 || i + 8 + 2 * len + 8 > d.len() {
                break;
            }
            let key = utf16(&d[i + 8..i + 8 + 2 * len]);
            i += 8 + 2 * len;
            let off = u64_at(d, i).unwrap_or(0) as usize;
            i += 8;
            if let Some(v) = off.checked_sub(4).and_then(|o| vals.get(&o)) {
                keys.insert(key, v.clone());
            }
        }
        Dict { keys }
    }

    /// Every container in the data (a scene holds several).
    pub fn all(d: &[u8]) -> Vec<Dict> {
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(p) = find(&d[from..], b"Q3D0") {
            let start = from + p + 4;
            out.push(Dict::parse(d, start));
            from = start;
        }
        out
    }

    pub fn text(&self, k: &str) -> Option<&str> {
        match self.keys.get(k) {
            Some(Value::Text(s)) => Some(s),
            _ => None,
        }
    }
    pub fn float(&self, k: &str) -> Option<f32> {
        match self.keys.get(k) {
            Some(Value::Float(f)) => Some(*f),
            Some(Value::Int(i)) => Some(*i as f32),
            _ => None,
        }
    }
    pub fn count(&self, k: &str) -> usize {
        match self.keys.get(k) {
            Some(Value::Int(i)) => (*i).max(0) as usize,
            Some(Value::Float(f)) => f.max(0.0) as usize,
            _ => 0,
        }
    }
    pub fn floats(&self, k: &str) -> Option<&[f32]> {
        match self.keys.get(k) {
            Some(Value::Floats(v)) => Some(v),
            _ => None,
        }
    }
    pub fn bytes(&self, k: &str) -> Option<Range<usize>> {
        match self.keys.get(k) {
            Some(Value::Bytes(r)) => Some(r.clone()),
            _ => None,
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Lumion is left-handed (Z away); Render Mill is right-handed.
fn flip(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.y, -v.z)
}

/// Decode one mesh block (tagged vertex streams).
pub fn parse_mesh(d: &[u8]) -> Option<MeshData> {
    let mut i = find(d, b"VRCO")?;
    let mut streams: HashMap<[u8; 4], &[u8]> = HashMap::new();
    while i + 8 <= d.len() {
        let tag: [u8; 4] = d[i..i + 4].try_into().ok()?;
        if !tag.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
            break;
        }
        let len = u32_at(d, i + 4)? as usize;
        let body = d.get(i + 8..i + 8 + len)?;
        streams.insert(tag, body);
        i += 8 + len;
        if &tag == b"PO32" || &tag == b"PO16" {
            break;
        }
    }
    let f32s = |b: &[u8]| -> Vec<f32> { b.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect() };
    let pos = f32s(streams.get(b"VPPI")?);
    let n = pos.len() / 3;
    let positions: Vec<Vec3> = pos.chunks_exact(3).map(|p| flip(Vec3::new(p[0], p[1], p[2]))).collect();
    let normals: Vec<Vec3> = match streams.get(b"VNNI") {
        Some(b) if b.len() >= n * 6 => b
            .chunks_exact(6)
            .take(n)
            .map(|c| {
                let s = |k: usize| i16::from_le_bytes([c[k], c[k + 1]]) as f32 / 32767.0;
                flip(Vec3::new(s(0), s(2), s(4))).normalize_or_zero()
            })
            .collect(),
        _ => Vec::new(),
    };
    let uvs: Vec<Vec2> = match streams.get(b"VTD0") {
        Some(b) if b.len() >= n * 8 => f32s(b).chunks_exact(2).take(n).map(|t| Vec2::new(t[0], t[1])).collect(),
        _ => Vec::new(),
    };
    let mut indices: Vec<u32> = if let Some(b) = streams.get(b"PO32") {
        b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
    } else {
        streams.get(b"PO16")?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]) as u32).collect()
    };
    indices.truncate(indices.len() / 3 * 3);
    if indices.iter().any(|&k| k as usize >= n) {
        return None;
    }
    // Lumion's "double-sided" import stores the surface twice: a second copy
    // of the vertices (normals flipped) with the triangles reversed. Render
    // Mill shades both sides itself, so keep the front copy only.
    let (positions, normals, uvs) = (positions, normals, uvs);
    let mut keep_verts = n;
    let h = indices.len() / 2;
    if n.is_multiple_of(2) && h.is_multiple_of(3) && h > 0 {
        let half = (n / 2) as u32;
        let (a, b) = indices.split_at(h);
        let disjoint = a.iter().all(|&k| k < half) && b.iter().all(|&k| k >= half);
        let centroid = |r: std::ops::Range<usize>| r.clone().map(|k| positions[k]).fold(Vec3::ZERO, |s, p| s + p) / r.len().max(1) as f32;
        if disjoint && (centroid(0..n / 2) - centroid(n / 2..n)).length() < 1e-3 {
            indices.truncate(h);
            keep_verts = n / 2;
        }
    }
    let n = keep_verts;
    let positions: Vec<Vec3> = positions.into_iter().take(n).collect();
    let normals: Vec<Vec3> = normals.into_iter().take(n).collect();
    let uvs: Vec<Vec2> = uvs.into_iter().take(n).collect();
    // Mirroring Z reverses the winding.
    for t in indices.chunks_exact_mut(3) {
        t.swap(1, 2);
    }
    let mut m = MeshData { name: String::new(), material: String::new(), positions, normals, uvs, indices };
    if m.normals.len() != n {
        m.normals.clear();
    }
    if m.uvs.len() != n {
        m.uvs.clear();
    }
    Some(m)
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Material from the colour/type JSON Lumion stores with each surface.
fn material_from_json(name: &str, json: &str) -> MaterialParams {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let rgb = v.get("albedo_color").or_else(|| v.get("color_diffuse")).and_then(|c| c.as_array()).map(|a| {
        let g = |k: usize| a.get(k).and_then(|x| x.as_f64()).unwrap_or(0.8) as f32;
        [g(0), g(1), g(2)]
    });
    let alpha = v
        .get("alpha")
        .and_then(|x| x.as_f64())
        .map(|a| a as f32)
        .or_else(|| v.get("transparency").and_then(|x| x.as_f64()).map(|t| 1.0 - t as f32))
        .unwrap_or(1.0);
    let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("").to_lowercase();
    let lname = name.to_lowercase();
    let mut p = MaterialParams::default();
    if let Some(c) = rgb {
        p.base_color = [srgb_to_linear(c[0]), srgb_to_linear(c[1]), srgb_to_linear(c[2]), alpha.clamp(0.0, 1.0)];
    }
    if alpha < 0.95 || ty == "glass" || lname.contains("glass") || lname.contains("glazing") {
        p.kind = MaterialKind::Glass;
        p.base_color[3] = alpha.min(0.6);
        p.roughness = 0.02;
    } else if ty == "metal" || lname.contains("metal") || lname.contains("steel") || lname.contains("chrome") || lname.contains("copper") {
        p.metallic = if lname.contains("paint") { 0.0 } else { 1.0 };
        p.roughness = if lname.contains("polished") || lname.contains("chrome") { 0.2 } else { 0.45 };
    } else {
        p.roughness = 0.7;
    }
    p
}

/// Lumion material settings for a surface (from the model's .lms).
#[derive(Clone, Debug, Default)]
pub struct LumionMaterial {
    /// Lumion library material it was based on ("Brick_floor_001.lmat"),
    /// or "Standard.lmat" for the user's own.
    pub base: String,
    pub color: Option<[f32; 3]>,
    pub colorize: f32,
    pub roughness: Option<f32>,
    pub metalness: Option<f32>,
    pub alpha: Option<f32>,
    pub emissive: f32,
    /// Texture blocks (DDS inside), only used for the user's own materials.
    pub color_map: Option<Range<usize>>,
    pub normal_map: Option<Range<usize>>,
}

fn lms_materials(d: &[u8]) -> HashMap<String, LumionMaterial> {
    let mut out = HashMap::new();
    for dict in Dict::all(d) {
        for i in 0..dict.count("filter.dataArray.size") {
            let k = |s: &str| format!("filter.[{i}].{s}");
            let Some(name) = dict.text(&k("materialName")) else { continue };
            let pbr = |s: &str| format!("filter.[{i}].material.data.pbr.{s}");
            let m = LumionMaterial {
                base: dict.text(&k("material.base.materialGuid")).unwrap_or_default().to_string(),
                color: dict.floats(&pbr("colorRGB")).filter(|c| c.len() >= 3).map(|c| [c[0], c[1], c[2]]),
                colorize: dict.float(&pbr("colorize")).unwrap_or(0.0),
                roughness: dict.float(&pbr("roughness")),
                metalness: dict.float(&pbr("metalness")),
                alpha: dict.float(&pbr("alpha")),
                emissive: dict.float(&pbr("emissiveness")).unwrap_or(0.0),
                color_map: dict.bytes(&pbr("texColorMap")),
                normal_map: dict.bytes(&pbr("texNormalMap")),
            };
            out.insert(name.to_string(), m);
        }
    }
    out
}

/// Decode a texture block (Lumion wraps a DDS file).
fn texture_block(d: &[u8], r: &Range<usize>) -> Option<TextureData> {
    let b = d.get(r.clone())?;
    let p = find(b, b"DDS ")?;
    decode_dds(&b[p..])
}

/// Top mip level of a block-compressed DDS (BC1-BC7, as Lumion writes them).
pub fn decode_dds(b: &[u8]) -> Option<TextureData> {
    if b.len() < 128 || &b[..4] != b"DDS " {
        return None;
    }
    let h = u32_at(b, 12)? as usize;
    let w = u32_at(b, 16)? as usize;
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return None;
    }
    let fourcc = &b[84..88];
    let (format, data_at): (&str, usize) = match fourcc {
        b"DX10" => {
            let dxgi = u32_at(b, 128)?;
            let f = match dxgi {
                70..=72 => "bc1",
                73..=75 => "bc2",
                76..=78 => "bc3",
                79..=81 => "bc4",
                82..=84 => "bc5",
                97..=99 => "bc7",
                _ => return None,
            };
            (f, 148)
        }
        b"DXT1" => ("bc1", 128),
        b"DXT3" => ("bc2", 128),
        b"DXT5" => ("bc3", 128),
        b"BC4U" | b"ATI1" => ("bc4", 128),
        b"BC5U" | b"ATI2" => ("bc5", 128),
        _ => {
            let img = image::load_from_memory_with_format(b, image::ImageFormat::Dds).ok()?;
            return Some(crate::import::texture_from_image(img));
        }
    };
    let data = b.get(data_at..)?;
    let mut px = vec![0u32; w * h];
    let ok = match format {
        "bc1" => texture2ddecoder::decode_bc1(data, w, h, &mut px),
        "bc2" => texture2ddecoder::decode_bc2(data, w, h, &mut px),
        "bc3" => texture2ddecoder::decode_bc3(data, w, h, &mut px),
        "bc4" => texture2ddecoder::decode_bc4(data, w, h, &mut px),
        "bc5" => texture2ddecoder::decode_bc5(data, w, h, &mut px),
        _ => texture2ddecoder::decode_bc7(data, w, h, &mut px),
    };
    ok.ok()?;
    // BGRA (little-endian u32) -> RGBA bytes.
    let rgba: Vec<u8> = px
        .iter()
        .flat_map(|c| {
            let [bl, g, r, a] = c.to_le_bytes();
            [r, g, bl, a]
        })
        .collect();
    Some(TextureData::new(w as u32, h as u32, rgba))
}

/// Apply Lumion's material settings on top of the surface's own colour.
fn apply_lumion_material(name: &str, p: &mut MaterialParams, lm: &LumionMaterial, d: &[u8]) -> (Option<TextureData>, Option<TextureData>, Option<String>) {
    if let Some(r) = lm.roughness {
        p.roughness = r.clamp(0.02, 1.0);
    }
    if let Some(m) = lm.metalness {
        p.metallic = m.clamp(0.0, 1.0);
    }
    if let Some(a) = lm.alpha.filter(|a| *a < 0.99) {
        p.kind = MaterialKind::Glass;
        p.base_color[3] = a;
    }
    // Lumion's colour tint: blended in by "colorize" (black means unset).
    let tint = lm.color.filter(|c| lm.colorize > 0.0 && c.iter().any(|v| *v > 0.001));
    let own = lm.base.is_empty() || lm.base.eq_ignore_ascii_case("Standard.lmat") || lm.base.to_lowercase().starts_with("custom");
    if own {
        // The user's own textures travel with the file.
        let c = lm.color_map.as_ref().and_then(|r| texture_block(d, r));
        let n = lm.normal_map.as_ref().and_then(|r| texture_block(d, r));
        if c.is_some() {
            p.use_texture = true;
            // The texture carries the colour; the tint (if any) multiplies it.
            // Colorize shifts the texture's hue but keeps its brightness:
            // normalise the tint to unit luminance before blending it in.
            let k = if tint.is_some() { lm.colorize.clamp(0.0, 1.0) } else { 0.0 };
            let t = tint.unwrap_or([1.0; 3]).map(srgb_to_linear);
            let lum = (0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2]).max(1e-3);
            let mix = |i: usize| 1.0 + ((t[i] / lum).min(4.0) - 1.0) * k;
            p.base_color = [mix(0), mix(1), mix(2), p.base_color[3]];
        } else if let Some(t) = tint {
            p.base_color = [srgb_to_linear(t[0]), srgb_to_linear(t[1]), srgb_to_linear(t[2]), p.base_color[3]];
        }
        (c, n, None)
    } else {
        // A Lumion library material: suggest Render Mill's closest one by
        // name ("Brick_floor_001.lmat" -> brick), applied by the caller.
        let _ = name;
        if let Some(t) = tint {
            p.base_color = [srgb_to_linear(t[0]), srgb_to_linear(t[1]), srgb_to_linear(t[2]), p.base_color[3]];
        }
        (None, None, Some(lm.base.trim_end_matches(".lmat").replace('_', " ")))
    }
}

/// An imported model read from Lumion.
#[derive(Default)]
pub struct LumionModel {
    pub name: String,
    pub source_file: Option<String>,
    pub meshes: Vec<MeshData>,
    pub materials: Vec<Material>,
    /// Material name -> Lumion library material to replace with a Render
    /// Mill library material of the same kind.
    pub library_hints: Vec<(String, String)>,
}

/// Read an imported model (.lib bytes) with its Lumion materials (.lms).
pub fn read_model(lib: &[u8], lms: Option<&[u8]>, name: &str) -> Result<LumionModel> {
    let dicts = Dict::all(lib);
    let lumion_mats = lms.map(lms_materials).unwrap_or_default();
    let dict = dicts.into_iter().max_by_key(|d| d.keys.len()).unwrap_or_default();
    let n = dict.count("model.surfaces.dataArray.size");
    if n == 0 {
        // The other layout Lumion uses for imported models (a graph of
        // objects): each surface's material description comes just before
        // its mesh.
        return read_model_graph(lib, lms, &lumion_mats, name);
    }
    let mut model = LumionModel { name: name.to_string(), source_file: dict.text("model.sourceFile").map(str::to_string), ..Default::default() };
    let mut seen = std::collections::HashSet::new();
    for i in 0..n {
        let k = |s: &str| format!("model.surfaces.[{i}].surface.{s}");
        let Some(r) = dict.bytes(&k("mesh")) else { continue };
        let Some(mut mesh) = parse_mesh(&lib[r]) else { continue };
        let mat_name = dict.text(&k("name")).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| format!("Surface {i}"));
        mesh.name = mat_name.clone();
        mesh.material = mat_name.clone();
        if seen.insert(mat_name.clone()) {
            let mut params = material_from_json(&mat_name, dict.text(&k("json")).unwrap_or("{}"));
            let (mut texture, mut normal_texture) = (None, None);
            if let (Some(lm), Some(lms)) = (lumion_mats.get(&mat_name), lms) {
                let (c, nrm, hint) = apply_lumion_material(&mat_name, &mut params, lm, lms);
                texture = c;
                normal_texture = nrm;
                if let Some(h) = hint {
                    model.library_hints.push((mat_name.clone(), h));
                }
            }
            model.materials.push(Material { name: mat_name, params, texture, normal_texture, roughness_texture: None });
        }
        model.meshes.push(mesh);
    }
    anyhow::ensure!(!model.meshes.is_empty(), "no readable meshes in this Lumion model");
    Ok(model)
}

/// UTF-16 JSON objects in the data: (start offset, end offset, text).
fn json_strings(d: &[u8]) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let starts: [&[u8]; 2] = [b"{\0\"\0a\0l\0b\0e\0d\0o\0", b"{\0\"\0c\0o\0l\0o\0r\0_\0d\0"];
    let mut i = 0;
    while i + 16 < d.len() {
        if starts.iter().any(|s| d[i..].starts_with(s)) {
            // Read UTF-16 up to the matching closing brace.
            let mut depth = 0i32;
            let mut j = i;
            let mut end = None;
            while j + 1 < d.len() && j - i < 65536 {
                let c = u16::from_le_bytes([d[j], d[j + 1]]);
                if c == b'{' as u16 {
                    depth += 1;
                } else if c == b'}' as u16 {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(j + 2);
                        break;
                    }
                } else if c == 0 {
                    break;
                }
                j += 2;
            }
            if let Some(e) = end {
                out.push((i, e, utf16(&d[i..e])));
                i = e;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Imported models stored as an object graph (older Lumion layout).
fn read_model_graph(lib: &[u8], lms: Option<&[u8]>, lumion_mats: &HashMap<String, LumionMaterial>, name: &str) -> Result<LumionModel> {
    let jsons = json_strings(lib);
    let mut model = LumionModel { name: name.to_string(), ..Default::default() };
    let mut seen = std::collections::HashSet::new();
    let mut from = 0;
    let mut k = 0usize;
    while let Some(p) = find(&lib[from..], b"VRCO") {
        let at = from + p;
        // Mesh blocks end at their index stream; parse from a little before.
        let Some(mut mesh) = parse_mesh(&lib[at..]) else {
            from = at + 4;
            continue;
        };
        // The material description closest before this mesh.
        let json = jsons.iter().rev().find(|(s, _, _)| *s < at).map(|(_, _, t)| t.as_str()).unwrap_or("{}");
        let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
        let mat_name = v.get("name").and_then(|n| n.as_str()).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| format!("Surface {k}"));
        mesh.name = mat_name.clone();
        mesh.material = mat_name.clone();
        if seen.insert(mat_name.clone()) {
            let mut params = material_from_json(&mat_name, json);
            let (mut texture, mut normal_texture) = (None, None);
            if let (Some(lm), Some(lms)) = (lumion_mats.get(&mat_name), lms) {
                let (c, nrm, hint) = apply_lumion_material(&mat_name, &mut params, lm, lms);
                texture = c;
                normal_texture = nrm;
                if let Some(h) = hint {
                    model.library_hints.push((mat_name.clone(), h));
                }
            }
            model.materials.push(Material { name: mat_name, params, texture, normal_texture, roughness_texture: None });
        }
        model.meshes.push(mesh);
        k += 1;
        from = at + 4;
    }
    anyhow::ensure!(!model.meshes.is_empty(), "no readable meshes in this Lumion file");
    Ok(model)
}

/// Read a model from Lumion's library folder (`<name>.lib` + `.lib.lms`).
pub fn read_model_file(path: &Path) -> Result<LumionModel> {
    let lib = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let lms_path = path.with_extension("lib.lms");
    let lms = std::fs::read(&lms_path).ok();
    let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    read_model(&lib, lms.as_deref(), &name)
}

/// Lumion's content catalogue on this computer: object file name ->
/// catalogue folder ("lumiontrees/plants/tea tree"). Only names are read,
/// to know what kind of thing each placed object is.
fn content_index() -> &'static HashMap<String, String> {
    static INDEX: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();
    INDEX.get_or_init(|| {
        let mut roots = Vec::new();
        if let Ok(p) = std::env::var("LUMION_PROJECT_DIR") {
            roots.push(std::path::PathBuf::from(p));
        }
        for base in ["C:\\Program Files", "C:\\Program Files (x86)"] {
            if let Ok(rd) = std::fs::read_dir(base) {
                for e in rd.flatten() {
                    if e.file_name().to_string_lossy().to_lowercase().starts_with("lumion") {
                        roots.push(e.path().join("ArchitectEdition").join("Project"));
                    }
                }
            }
        }
        let mut map = HashMap::new();
        for root in roots.into_iter().filter(|r| r.is_dir()) {
            let mut stack = vec![root.clone()];
            while let Some(dir) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&dir) else { continue };
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_lowercase()) {
                        if name.ends_with(".ltm") || name.ends_with(".lmod") || name.ends_with(".llm") {
                            let rel = dir.strip_prefix(&root).unwrap_or(&dir).to_string_lossy().to_lowercase().replace('\\', "/");
                            map.entry(name).or_insert(rel);
                        }
                    }
                }
            }
        }
        map
    })
}

/// Kind of object from its place in Lumion's catalogue.
fn kind_from_catalogue(name: &str, folder: &str) -> Option<Option<ObjectKind>> {
    let top = folder.split('/').next().unwrap_or("");
    // Look at the sub-folders (the top one, e.g. "people and animals", is the category).
    let f = folder.get(top.len()..).unwrap_or("");
    let has = |w: &[&str]| w.iter().any(|x| f.contains(x) || name.contains(x));
    Some(match top {
        "people and animals" => (!has(&["animal", "bird", "pet", "marine", "farm"])).then_some(ObjectKind::Person),
        "transport" => has(&["cars", "suv", "vans", "emergency", "trucks", "taxi"]).then_some(ObjectKind::Car),
        "lumiontrees" | "trees" if f.starts_with("/plants") => {
            if has(&["grass", "sedge", "dandelion", "yarrow"]) {
                None
            } else {
                Some(ObjectKind::Shrub)
            }
        }
        "lumiontrees" | "trees" => Some(if has(&["pine", "spruce", "larch", "fir", "cedar", "cypress", "conifer", "thuja", "juniper", "sequoia", "redwood"]) {
            ObjectKind::Conifer
        } else {
            ObjectKind::Tree
        }),
        "lumionnature" | "nature" => {
            if has(&["rock", "stone", "boulder"]) {
                Some(ObjectKind::Boulder)
            } else if has(&["grass", "weed", "forest wood", "leaf"]) {
                None
            } else {
                Some(ObjectKind::Shrub)
            }
        }
        "lights" => {
            if has(&["spot"]) {
                Some(ObjectKind::SpotLight)
            } else if has(&["omni", "area"]) {
                Some(ObjectKind::PointLight)
            } else {
                None
            }
        }
        "objects" => {
            if has(&["sofa", "couch"]) {
                Some(ObjectKind::Sofa)
            } else if has(&["armchair", "lounge"]) {
                Some(ObjectKind::ArmChair)
            } else if has(&["bench"]) {
                Some(ObjectKind::Bench)
            } else if has(&["seating", "chair", "stool"]) {
                Some(ObjectKind::Chair)
            } else if has(&["table"]) {
                Some(ObjectKind::Table)
            } else if has(&["pot", "planter"]) {
                Some(ObjectKind::Planter)
            } else if has(&["lighting", "lamp"]) {
                Some(ObjectKind::StreetLamp)
            } else {
                None
            }
        }
        _ => return None,
    })
}

/// What a Lumion library object becomes in Render Mill.
pub fn substitute(type_guid: &str) -> Option<ObjectKind> {
    let lower = type_guid.to_lowercase();
    if let Some(folder) = content_index().get(&lower) {
        if let Some(k) = kind_from_catalogue(&lower, folder) {
            return k;
        }
    }
    substitute_by_name(type_guid)
}

/// Fallback when Lumion isn't installed on this computer: guess from the name.
fn substitute_by_name(type_guid: &str) -> Option<ObjectKind> {
    let l = type_guid.to_lowercase();
    let has = |w: &[&str]| w.iter().any(|x| l.contains(x));
    // Landscape painting, grass clumps, text, effects: Render Mill has its own.
    // Grass clumps, landscape paint, text and effects: Render Mill has its own.
    // (".llm" is also used for Lumion's ray-traced trees, so go by name.)
    if l.ends_with(".spe")
        || has(&[
            "backdrop",
            "grass",
            "terrain",
            "landscape",
            "cloud",
            "fog",
            "fire",
            "smoke",
            "water",
            "dirt",
            "paint",
            "plaster",
            "soil",
            "gravel",
            "mud",
            "sand",
        ])
    {
        return None;
    }
    Some(if has(&["person", "people", "man_", "woman", "boy", "girl", "child", "human", "walker", "worker", "kid"]) {
        ObjectKind::Person
    } else if has(&["car", "vehicle", "suv", "sedan", "truck", "van_", "pickup", "jeep", "taxi"]) {
        ObjectKind::Car
    } else if has(&["pine", "spruce", "fir_", "cedar", "conifer", "cypress", "juniper", "thuja", "larch", "sequoia", "redwood"]) {
        ObjectKind::Conifer
    } else if has(&[
        "shrub", "bush", "hedge", "gaura", "fern", "lavender", "creeper", "flower", "plant", "grass_", "boxwood", "rose", "hosta", "agave", "tea_tree",
        "rosemary", "sage",
    ]) {
        ObjectKind::Shrub
    } else if has(&["pot", "planter"]) {
        ObjectKind::Planter
    } else if has(&["rock", "stone", "boulder"]) {
        ObjectKind::Boulder
    } else if has(&["lamp", "lantern", "streetlight", "light_pole"]) {
        ObjectKind::StreetLamp
    } else if has(&["sofa", "couch"]) {
        ObjectKind::Sofa
    } else if has(&["armchair", "lounge"]) {
        ObjectKind::ArmChair
    } else if has(&["bench"]) {
        ObjectKind::Bench
    } else if has(&["chair", "stool"]) {
        ObjectKind::Chair
    } else if has(&["table", "desk"]) {
        ObjectKind::Table
    } else if has(&[
        "tree",
        "oak",
        "maple",
        "birch",
        "olive",
        "hickory",
        "rowan",
        "ash_",
        "elm",
        "palm",
        "willow",
        "linden",
        "poplar",
        "beech",
        "magnolia",
        "cherry",
        "baobab",
        "acacia",
        "eucalyptus",
        "_rt.",
    ]) {
        ObjectKind::Tree
    } else if l.ends_with(".ltm") {
        // Other Lumion nature items with catalogue-style names: the size
        // isn't known, so a shrub is the safer stand-in.
        ObjectKind::Shrub
    } else {
        return None;
    })
}

/// Row-major Lumion matrix (translation in the last row) to Render Mill.
fn convert_matrix(m: &[f32]) -> Mat4 {
    let mut a = [0.0f32; 16];
    a.copy_from_slice(&m[..16]);
    // Lumion uses row vectors: its rows are glam's columns.
    let g = Mat4::from_cols_array(&a);
    let s = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0));
    s * g * s
}

/// A camera view saved in the Lumion scene.
#[derive(Clone, Debug)]
pub struct LumionCamera {
    pub name: String,
    pub camera: Camera,
}

pub struct LumionScene {
    pub models: Vec<LumionModel>,
    /// Imported-model instances: (model index, transform).
    pub instances: Vec<(usize, Mat4)>,
    pub objects: Vec<PlacedObject>,
    /// Lumion library objects with no Render Mill equivalent (name, count).
    pub skipped: Vec<(String, usize)>,
    pub cameras: Vec<LumionCamera>,
    /// Sun heading and height in degrees.
    pub sun: Option<(f32, f32)>,
}

fn camera_from(pos: Vec3, forward: Vec3, fov_h: f32) -> Camera {
    let mut c = Camera { position: pos.to_array(), ..Default::default() };
    c.look_at(pos + forward);
    // Lumion's zoom is the horizontal field of view; Render Mill's is vertical (16:9).
    let v = 2.0 * ((fov_h.to_radians() * 0.5).tan() * 9.0 / 16.0).atan();
    c.fov = v.to_degrees().clamp(10.0, 100.0);
    c
}

/// Read a Lumion scene (.lsf or quick-save .lqf).
pub fn read_scene(path: &Path) -> Result<LumionScene> {
    let d = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    read_scene_bytes(&d)
}

pub fn read_scene_bytes(d: &[u8]) -> Result<LumionScene> {
    let dicts = Dict::all(d);
    let dict = dicts.into_iter().find(|x| x.keys.keys().any(|k| k.starts_with("sceneitems."))).context("this doesn't look like a Lumion scene file")?;
    // Imported models embedded in the scene.
    let mut models = Vec::new();
    let mut by_guid: HashMap<String, usize> = HashMap::new();
    for i in 0..dict.count("customModels.dataArray.size") {
        let k = |s: &str| format!("customModels.[{i}].{s}");
        let Some(lib) = dict.bytes(&k("modelfiles.lib")) else { continue };
        let lms = dict.bytes(&k("modelfiles.lms"));
        let name = dict.text(&k("filename")).unwrap_or("Model").trim_end_matches(".lib").to_string();
        // Sub-slices keep texture offsets relative to the .lms block.
        match read_model(&d[lib], lms.map(|r| &d[r]), &name) {
            Ok(m) => {
                if let Some(g) = dict.text(&k("guid")) {
                    by_guid.insert(g.to_string(), models.len());
                }
                models.push(m);
            }
            Err(e) => log::warn!("Lumion model {name}: {e}"),
        }
    }
    let mut instances = Vec::new();
    let mut objects = Vec::new();
    let mut skipped: HashMap<String, usize> = HashMap::new();
    for i in 0..dict.count("sceneitems.dataArray.size") {
        let k = |s: &str| format!("sceneitems.[{i}].{s}");
        let Some(ty) = dict.text(&k("typeGuid")) else { continue };
        let Some(m) = dict.floats(&k("data.base.worldMatrix")).filter(|m| m.len() >= 16) else { continue };
        let mat = convert_matrix(m);
        if let Some(&mi) = by_guid.get(ty) {
            instances.push((mi, mat));
            continue;
        }
        match substitute(ty) {
            Some(kind) => {
                let (scale, rot, pos) = mat.to_scale_rotation_translation();
                let fwd = rot * Vec3::X;
                let yaw = (-fwd.z).atan2(fwd.x); // radians, about +Y
                let seed = ty.bytes().fold(i as u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
                objects.push(PlacedObject::new(kind, pos.to_array(), yaw, scale.y.abs().clamp(0.1, 10.0), seed));
            }
            None => *skipped.entry(ty.to_string()).or_insert(0) += 1,
        }
    }
    // Cameras: saved photo views and the first key of each movie clip.
    let mut cameras = Vec::new();
    for i in 0..dict.count("cameras.dataArray.size") {
        let k = |s: &str| format!("cameras.[{i}].data.data.{s}");
        if let (Some(p), Some(h), Some(pitch)) =
            (dict.floats(&k("cameraStatic.pos")), dict.float(&k("cameraStatic.heading")), dict.float(&k("cameraStatic.pitch")))
        {
            let fwd = Vec3::new(h.sin() * pitch.cos(), pitch.sin(), h.cos() * pitch.cos());
            // Photo views store the lens as a focal length in mm (35 mm film).
            let mm = dict.float(&k("cameraStatic.zoomfactor")).unwrap_or(30.0).max(1.0);
            let fov = 2.0 * (18.0 / mm).atan().to_degrees();
            cameras.push(LumionCamera {
                name: format!("Lumion view {}", cameras.len() + 1),
                camera: camera_from(flip(Vec3::new(p[0], p[1], p[2])), flip(fwd), fov),
            });
        } else if let Some(m) = dict.floats(&k("cameraLive.keyframe.[0].camera")).filter(|m| m.len() >= 16) {
            let fwd = flip(Vec3::new(m[8], m[9], m[10]));
            let pos = flip(Vec3::new(m[12], m[13], m[14]));
            let fov = dict.float(&k("cameraLive.keyframe.[0].zoom")).unwrap_or(60.0);
            cameras.push(LumionCamera { name: format!("Lumion clip {}", cameras.len() + 1), camera: camera_from(pos, fwd, fov) });
        }
    }
    let sun = match (dict.float("environment.sky.sunHeading"), dict.float("environment.sky.sunPitch")) {
        (Some(h), Some(p)) => Some((h.to_degrees().rem_euclid(360.0), p.to_degrees().clamp(-10.0, 90.0))),
        _ => None,
    };
    let mut skipped: Vec<(String, usize)> = skipped.into_iter().collect();
    skipped.sort_by_key(|s| std::cmp::Reverse(s.1));
    Ok(LumionScene { models, instances, objects, skipped, cameras, sun })
}

/// Is this a Lumion file Render Mill can open?
pub fn is_lumion(path: &Path) -> bool {
    let e = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    matches!(e.as_str(), "lsf" | "lqf" | "lib")
}

pub fn is_scene(path: &Path) -> bool {
    let e = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    matches!(e.as_str(), "lsf" | "lqf")
}

/// All model geometry of a scene (instances placed), as one import.
pub fn scene_meshes(s: &LumionScene) -> (Vec<MeshData>, Vec<Material>) {
    let mut parts = Vec::new();
    let mut mats: Vec<Material> = Vec::new();
    let mut names = std::collections::HashSet::new();
    for &(mi, xf) in &s.instances {
        let m = &s.models[mi];
        let nx = Mat4::from_mat3(glam::Mat3::from_mat4(xf).inverse().transpose());
        for mesh in &m.meshes {
            let mut p = mesh.clone();
            for v in &mut p.positions {
                *v = xf.transform_point3(*v);
            }
            for n in &mut p.normals {
                *n = nx.transform_vector3(*n).normalize_or_zero();
            }
            if xf.determinant() < 0.0 {
                for t in p.indices.chunks_exact_mut(3) {
                    t.swap(1, 2);
                }
            }
            parts.push(p);
        }
        for mat in &m.materials {
            if names.insert(mat.name.clone()) {
                mats.push(mat.clone());
            }
        }
    }
    (parts, mats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_value(out: &mut Vec<u8>, ty: u8, payload: &[u8]) -> usize {
        let off = out.len();
        out.push(if ty == 0xff { 0 } else { 1 });
        out.push(ty);
        out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        out.extend_from_slice(payload);
        off
    }
    fn utf16b(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|c| c.to_le_bytes()).collect()
    }

    fn mesh_block(tris: &[[u32; 3]], pos: &[[f32; 3]], double: bool) -> Vec<u8> {
        let mut b = b"junkVRCO".to_vec();
        b.extend_from_slice(&4u32.to_le_bytes());
        b.extend_from_slice(&(pos.len() as u32).to_le_bytes());
        let stream = |b: &mut Vec<u8>, tag: &[u8], data: Vec<u8>| {
            b.extend_from_slice(tag);
            b.extend_from_slice(&(data.len() as u32).to_le_bytes());
            b.extend_from_slice(&data);
        };
        stream(&mut b, b"VPPI", pos.iter().flat_map(|p| p.iter().flat_map(|f| f.to_le_bytes())).collect());
        stream(&mut b, b"VNNI", pos.iter().flat_map(|_| [0i16, 32767, 0].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>()).collect());
        let mut idx: Vec<u32> = tris.iter().flatten().copied().collect();
        if double {
            let n = pos.len() as u32 / 2;
            idx.extend(tris.iter().flat_map(|t| [t[0] + n, t[2] + n, t[1] + n]));
        }
        stream(&mut b, b"PO32", idx.iter().flat_map(|i| i.to_le_bytes()).collect());
        b
    }

    /// A synthetic model container in Lumion's layout.
    fn fake_lib() -> Vec<u8> {
        let mut d = b"rawSQ3D0".to_vec();
        let start = d.len();
        let mut vals = Vec::new();
        let off = |d: &mut Vec<u8>, ty: u8, p: &[u8]| put_value(d, ty, p) - start;
        let tri = [[0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [0.0, 1.0, 1.0]];
        let mesh = mesh_block(&[[0, 1, 2]], &[tri[0], tri[1], tri[2], tri[2], tri[1], tri[0]], true);
        vals.push(("model.surfaces.[0].surface.mesh", off(&mut d, 0xff, &mesh)));
        vals.push(("model.surfaces.[0].surface.name", off(&mut d, 5, &utf16b("Brick, Common"))));
        vals.push(("model.surfaces.[0].surface.json", off(&mut d, 5, &utf16b(r#"{"albedo_color":[0.5,0.2,0.1],"alpha":1,"type":"masonry"}"#))));
        vals.push(("model.surfaces.dataArray.size", off(&mut d, 0, &1i64.to_le_bytes())));
        for (k, o) in vals {
            d.extend_from_slice(&(k.encode_utf16().count() as u64).to_le_bytes());
            d.extend_from_slice(&utf16b(k));
            d.extend_from_slice(&((o + 4) as u64).to_le_bytes());
        }
        d
    }

    #[test]
    fn reads_a_model_container() {
        let lib = fake_lib();
        let m = read_model(&lib, None, "Test").unwrap();
        assert_eq!(m.meshes.len(), 1);
        let mesh = &m.meshes[0];
        // Double-sided copy removed, Z mirrored, winding flipped.
        assert_eq!(mesh.indices, vec![0, 2, 1]);
        assert_eq!(mesh.positions.len(), 3);
        assert_eq!(mesh.positions[0], Vec3::new(0.0, 0.0, -1.0));
        assert!((mesh.normals[0] - Vec3::Y).length() < 1e-3);
        assert_eq!(m.materials[0].name, "Brick, Common");
        let c = m.materials[0].params.base_color;
        assert!(c[0] > c[1] && c[1] > c[2]);
    }

    #[test]
    fn lumion_objects_get_render_mill_substitutes() {
        assert_eq!(substitute_by_name("tea_tree_003.ltm"), Some(ObjectKind::Shrub));
        assert_eq!(substitute_by_name("olive_002.ltm"), Some(ObjectKind::Tree));
        assert_eq!(kind_from_catalogue("tea_tree_003.ltm", "lumiontrees/plants/tea tree"), Some(Some(ObjectKind::Shrub)));
        assert_eq!(kind_from_catalogue("x.ltm", "lumiontrees/trees/european spruces"), Some(Some(ObjectKind::Conifer)));
        assert_eq!(kind_from_catalogue("x.lmod", "transport/air"), Some(None));
        assert_eq!(kind_from_catalogue("x.lmod", "people and animals/men - 3d"), Some(Some(ObjectKind::Person)));
        assert_eq!(kind_from_catalogue("x.lmod", "lights/spotlights"), Some(Some(ObjectKind::SpotLight)));
        assert_eq!(substitute_by_name("white_gaura_004.ltm"), Some(ObjectKind::Shrub));
        assert_eq!(substitute_by_name("am_260_010_pot.lmod"), Some(ObjectKind::Planter));
        assert_eq!(substitute_by_name("simple_grass_006.llm"), None);
        assert_eq!(substitute_by_name("japanese_pine-1_rt.llm"), Some(ObjectKind::Conifer));
        assert_eq!(substitute_by_name("africanbaobab_rt.llm"), Some(ObjectKind::Tree));
        assert_eq!(substitute_by_name("stone_flat_001.lmod"), Some(ObjectKind::Boulder));
        assert_eq!(substitute_by_name("dirt_002_c.llm"), None);
        assert_eq!(substitute_by_name("am250_065.ltm"), Some(ObjectKind::Shrub));
        assert_eq!(substitute_by_name("backdrop_hills_001_c.lmod"), None);
    }

    #[test]
    fn matrices_mirror_into_render_mill_space() {
        // Translation (1, 2, 3) in Lumion -> (1, 2, -3).
        let mut m = [0.0f32; 16];
        m[0] = 1.0;
        m[5] = 1.0;
        m[10] = 1.0;
        m[15] = 1.0;
        m[12] = 1.0;
        m[13] = 2.0;
        m[14] = 3.0;
        let g = convert_matrix(&m);
        assert!((g.transform_point3(Vec3::ZERO) - Vec3::new(1.0, 2.0, -3.0)).length() < 1e-5);
        assert!((g.transform_point3(Vec3::new(0.0, 0.0, -1.0)) - Vec3::new(1.0, 2.0, -4.0)).length() < 1e-5);
    }
}

#[cfg(test)]
mod real_files {
    /// Set LC_LUMION_LIB to a Lumion .lib to print what is read from it.
    #[test]
    fn dump_lib_if_given() {
        let Ok(p) = std::env::var("LC_LUMION_LIB") else { return };
        let m = super::read_model_file(std::path::Path::new(&p)).unwrap();
        for mat in &m.materials {
            eprintln!(
                "{:50} tex {:?} nrm {:?} colour {:?}",
                mat.name,
                mat.texture.as_ref().map(|t| (t.width, t.height)),
                mat.normal_texture.as_ref().map(|t| (t.width, t.height)),
                mat.params.base_color
            );
        }
        eprintln!("hints {:?}", m.library_hints);
    }
}
