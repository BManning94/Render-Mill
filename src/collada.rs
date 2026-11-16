//! COLLADA (.dae) import: Lumion LiveSync for Revit, SketchUp, ArchiCAD,
//! 3ds Max and most other tools write it.
//!
//! Supports triangles / polylist / polygons, node hierarchies with matrix /
//! translate / rotate / scale, instance_node, bound materials, Phong /
//! Lambert / Blinn colours and textures, transparency, and the extra data
//! Lumion's exporter adds (albedo colour and map, normal map, roughness,
//! reflectiveness, material type).

use crate::import::{FileHints, TextureResolver, UpAxis};
use crate::scene::{guess_params_from_name, Material, MaterialKind, MaterialParams, MeshData, TextureData};
use anyhow::{Context, Result};
use glam::{Mat4, Vec2, Vec3};
use roxmltree::{Document, Node};
use std::collections::HashMap;
use std::path::Path;

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn floats(s: &str) -> Vec<f32> {
    s.split_ascii_whitespace().filter_map(|t| t.parse::<f32>().ok()).collect()
}

fn ints(s: &str) -> Vec<i64> {
    s.split_ascii_whitespace().filter_map(|t| t.parse::<i64>().ok()).collect()
}

fn child<'a, 'b>(n: Node<'a, 'b>, name: &str) -> Option<Node<'a, 'b>> {
    n.children().find(|c| c.has_tag_name(name))
}

fn children<'a, 'b: 'a>(n: Node<'a, 'b>, name: &'a str) -> impl Iterator<Item = Node<'a, 'b>> + 'a {
    n.children().filter(move |c| c.has_tag_name(name))
}

fn url_id(u: &str) -> &str {
    u.trim_start_matches('#')
}

/// "file://C:\a\b%20c.png" -> "C:\a\b c.png"
fn decode_uri(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix("file:///").or_else(|| s.strip_prefix("file://")).unwrap_or(s);
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A <source>: flat floats and the stride of one element.
struct Source {
    data: Vec<f32>,
    stride: usize,
}

impl Source {
    fn get(&self, i: usize, k: usize) -> f32 {
        self.data.get(i * self.stride + k).copied().unwrap_or(0.0)
    }
    fn len(&self) -> usize {
        self.data.len() / self.stride.max(1)
    }
}

fn read_source(n: Node) -> Option<Source> {
    let arr = child(n, "float_array")?;
    let data = floats(arr.text().unwrap_or(""));
    let stride = child(n, "technique_common").and_then(|t| child(t, "accessor")).and_then(|a| a.attribute("stride")).and_then(|s| s.parse().ok()).unwrap_or(3);
    Some(Source { data, stride })
}

/// Triangles of one primitive group of a mesh, with its material symbol.
struct Prim {
    symbol: String,
    mesh: MeshData,
}

fn read_mesh(mesh: Node) -> Vec<Prim> {
    let mut sources: HashMap<&str, Source> = HashMap::new();
    for s in children(mesh, "source") {
        if let (Some(id), Some(src)) = (s.attribute("id"), read_source(s)) {
            sources.insert(id, src);
        }
    }
    // <vertices> maps its id to POSITION (and sometimes NORMAL/TEXCOORD).
    let mut vertex_inputs: Vec<(String, String)> = Vec::new();
    let mut vertices_id = String::new();
    if let Some(v) = child(mesh, "vertices") {
        vertices_id = v.attribute("id").unwrap_or("").to_string();
        for i in children(v, "input") {
            vertex_inputs.push((i.attribute("semantic").unwrap_or("").to_string(), url_id(i.attribute("source").unwrap_or("")).to_string()));
        }
    }
    let mut out = Vec::new();
    for prim in mesh.children().filter(|c| c.is_element()) {
        let kind = prim.tag_name().name();
        if !matches!(kind, "triangles" | "polylist" | "polygons") {
            continue;
        }
        // Inputs: (semantic, source id, offset, set).
        let mut inputs: Vec<(String, String, usize, u32)> = Vec::new();
        for i in children(prim, "input") {
            let sem = i.attribute("semantic").unwrap_or("").to_string();
            let src = url_id(i.attribute("source").unwrap_or("")).to_string();
            let off = i.attribute("offset").and_then(|o| o.parse().ok()).unwrap_or(0);
            let set = i.attribute("set").and_then(|o| o.parse().ok()).unwrap_or(0);
            if sem == "VERTEX" || src == vertices_id {
                for (vs, vsrc) in &vertex_inputs {
                    inputs.push((vs.clone(), vsrc.clone(), off, set));
                }
            } else {
                inputs.push((sem, src, off, set));
            }
        }
        let stride = inputs.iter().map(|i| i.2).max().map_or(1, |m| m + 1);
        let find = |sem: &str| {
            let mut c: Vec<&(String, String, usize, u32)> = inputs.iter().filter(|i| i.0 == sem).collect();
            c.sort_by_key(|i| i.3);
            c.first().and_then(|i| sources.get(i.1.as_str()).map(|s| (s, i.2)))
        };
        let pos = find("POSITION");
        let nrm = find("NORMAL");
        let uv = find("TEXCOORD");
        let Some((pos_src, pos_off)) = pos else { continue };
        // Polygons as index lists (one entry per corner, `stride` ints each).
        let mut polys: Vec<Vec<i64>> = Vec::new();
        match kind {
            "triangles" => {
                let p = child(prim, "p").map(|p| ints(p.text().unwrap_or(""))).unwrap_or_default();
                for tri in p.chunks_exact(stride * 3) {
                    polys.push(tri.to_vec());
                }
            }
            "polylist" => {
                let vc = child(prim, "vcount").map(|v| ints(v.text().unwrap_or(""))).unwrap_or_default();
                let p = child(prim, "p").map(|p| ints(p.text().unwrap_or(""))).unwrap_or_default();
                let mut at = 0;
                for n in vc {
                    let len = n as usize * stride;
                    if at + len > p.len() {
                        break;
                    }
                    polys.push(p[at..at + len].to_vec());
                    at += len;
                }
            }
            _ => {
                for p in children(prim, "p") {
                    polys.push(ints(p.text().unwrap_or("")));
                }
            }
        }
        let mut m = MeshData::default();
        let mut remap: HashMap<(i64, i64, i64), u32> = HashMap::new();
        let mut corner = |c: &[i64], m: &mut MeshData| -> u32 {
            let pi = c.get(pos_off).copied().unwrap_or(0);
            let ni = nrm.map_or(-1, |(_, o)| c.get(o).copied().unwrap_or(-1));
            let ti = uv.map_or(-1, |(_, o)| c.get(o).copied().unwrap_or(-1));
            *remap.entry((pi, ni, ti)).or_insert_with(|| {
                let k = m.positions.len() as u32;
                let i = pi.max(0) as usize;
                m.positions.push(Vec3::new(pos_src.get(i, 0), pos_src.get(i, 1), pos_src.get(i, 2)));
                if let Some((s, _)) = nrm {
                    let j = ni.max(0) as usize;
                    m.normals.push(Vec3::new(s.get(j, 0), s.get(j, 1), s.get(j, 2)));
                }
                if let Some((s, _)) = uv {
                    let j = ti.max(0) as usize;
                    // COLLADA's v runs up; Render Mill's runs down.
                    m.uvs.push(Vec2::new(s.get(j, 0), 1.0 - s.get(j, 1)));
                }
                k
            })
        };
        for poly in polys {
            let corners: Vec<&[i64]> = poly.chunks_exact(stride).collect();
            if corners.len() < 3 || corners.iter().any(|c| c.get(pos_off).is_none_or(|&i| i < 0 || i as usize >= pos_src.len())) {
                continue;
            }
            let ids: Vec<u32> = corners.iter().map(|c| corner(c, &mut m)).collect();
            for k in 1..ids.len() - 1 {
                m.indices.extend_from_slice(&[ids[0], ids[k], ids[k + 1]]);
            }
        }
        if m.indices.is_empty() {
            continue;
        }
        out.push(Prim { symbol: prim.attribute("material").unwrap_or("").to_string(), mesh: m });
    }
    out
}

/// Local transform of a <node>.
fn node_transform(n: Node) -> Mat4 {
    let mut m = Mat4::IDENTITY;
    for c in n.children().filter(|c| c.is_element()) {
        let v = floats(c.text().unwrap_or(""));
        let t = match c.tag_name().name() {
            "matrix" if v.len() >= 16 => {
                let mut a = [0.0f32; 16];
                a.copy_from_slice(&v[..16]);
                // COLLADA matrices are row-major.
                Mat4::from_cols_array(&a).transpose()
            }
            "translate" if v.len() >= 3 => Mat4::from_translation(Vec3::new(v[0], v[1], v[2])),
            "rotate" if v.len() >= 4 => {
                let axis = Vec3::new(v[0], v[1], v[2]).normalize_or(Vec3::Z);
                Mat4::from_axis_angle(axis, v[3].to_radians())
            }
            "scale" if v.len() >= 3 => Mat4::from_scale(Vec3::new(v[0], v[1], v[2])),
            _ => continue,
        };
        m *= t;
    }
    m
}

struct Effect {
    color: [f32; 4],
    color_tex: Option<String>,
    normal_tex: Option<String>,
    roughness: Option<f32>,
    reflectiveness: Option<f32>,
    kind: String,
}

/// Image path behind a <texture texture="sampler"> (sampler -> surface -> image).
fn texture_image(effect: Node, sampler: &str, images: &HashMap<String, String>) -> Option<String> {
    let params: HashMap<&str, Node> = effect.descendants().filter(|d| d.has_tag_name("newparam")).filter_map(|p| p.attribute("sid").map(|s| (s, p))).collect();
    let mut id = sampler.to_string();
    for _ in 0..4 {
        if let Some(img) = images.get(&id) {
            return Some(img.clone());
        }
        let p = params.get(id.as_str())?;
        let next = p
            .descendants()
            .find(|d| d.has_tag_name("source") || d.has_tag_name("init_from"))
            .and_then(|d| d.text().map(|t| t.trim().to_string()).or_else(|| d.attribute("ref").map(str::to_string)))?;
        id = next;
    }
    images.get(&id).cloned()
}

fn read_effect(e: Node, images: &HashMap<String, String>) -> Effect {
    let mut fx = Effect { color: [0.8, 0.8, 0.8, 1.0], color_tex: None, normal_tex: None, roughness: None, reflectiveness: None, kind: String::new() };
    let tech = e.descendants().find(|d| matches!(d.tag_name().name(), "phong" | "lambert" | "blinn" | "constant"));
    let colour_or_tex = |n: Node| -> (Option<[f32; 4]>, Option<String>) {
        if let Some(c) = child(n, "color") {
            let v = floats(c.text().unwrap_or(""));
            if v.len() >= 3 {
                return (Some([v[0], v[1], v[2], v.get(3).copied().unwrap_or(1.0)]), None);
            }
        }
        if let Some(t) = child(n, "texture").and_then(|t| t.attribute("texture")) {
            return (None, texture_image(e, t, images));
        }
        (None, None)
    };
    if let Some(t) = tech {
        if let Some(d) = child(t, "diffuse") {
            let (c, tex) = colour_or_tex(d);
            if let Some(c) = c {
                fx.color = c;
            }
            fx.color_tex = tex;
        }
        if let Some(s) = child(t, "shininess").and_then(|s| child(s, "float")).and_then(|f| f.text()).and_then(|v| v.trim().parse::<f32>().ok()) {
            // Phong exponent (or 0..1 gloss from some exporters).
            let gloss = if s > 1.0 { (s / 128.0).min(1.0) } else { s };
            fx.roughness = Some((1.0 - gloss).clamp(0.05, 1.0));
        }
        // Transparency: <transparent> colour alpha and/or <transparency> factor.
        let mut alpha = fx.color[3];
        if let Some(tr) = child(t, "transparent") {
            if let (Some(c), _) = colour_or_tex(tr) {
                let a = if tr.attribute("opaque") == Some("RGB_ZERO") { 1.0 - (c[0] + c[1] + c[2]) / 3.0 } else { c[3] };
                alpha = alpha.min(a);
            }
        }
        if let Some(f) = child(t, "transparency").and_then(|s| child(s, "float")).and_then(|f| f.text()).and_then(|v| v.trim().parse::<f32>().ok()) {
            // Exporters disagree on the meaning; values near 0 usually mean opaque.
            if f > 0.0 && f < 1.0 && alpha >= 0.999 {
                alpha = f.max(1.0 - f).min(alpha);
            }
        }
        fx.color[3] = alpha;
    }
    // Lumion's exporter adds its own, more precise description.
    if let Some(l) = e.descendants().find(|d| d.has_tag_name("technique") && d.attribute("profile") == Some("LUMION")) {
        let num = |name: &str| child(l, name).and_then(|n| n.text()).and_then(|v| v.trim().parse::<f32>().ok());
        if let Some(c) = child(l, "albedo_color").and_then(|a| child(a, "color")) {
            let v = floats(c.text().unwrap_or(""));
            if v.len() >= 3 {
                fx.color = [v[0], v[1], v[2], fx.color[3]];
            }
        }
        if let Some(a) = num("alpha") {
            fx.color[3] = a;
        }
        if let Some(r) = num("roughness") {
            fx.roughness = Some(r);
        }
        fx.reflectiveness = num("reflectiveness");
        if let Some(t) = child(l, "type").and_then(|t| t.text()) {
            fx.kind = t.trim().to_lowercase();
        }
        if let Some(t) = child(l, "albedo_map").and_then(|m| child(m, "texture")).and_then(|t| t.attribute("texture")) {
            fx.color_tex = fx.color_tex.or_else(|| texture_image(e, t, images));
        }
        if let Some(t) = child(l, "normal_map").and_then(|m| child(m, "texture")).and_then(|t| t.attribute("texture")) {
            fx.normal_tex = texture_image(e, t, images);
        }
    }
    fx
}

/// Textures resolve through the shared resolver: absolute paths first, then
/// by file name under the model's folder (which covers Lumion LiveSync's
/// `<model>_Textures` folder when the .dae has been moved).
fn load_tex(_path: &Path, textures: &mut TextureResolver, reference: &str) -> Option<TextureData> {
    textures.load(reference)
}

fn material_from(name: &str, fx: &Effect, path: &Path, textures: &mut TextureResolver) -> Material {
    let c = fx.color;
    let mut p = MaterialParams { base_color: [srgb_to_linear(c[0]), srgb_to_linear(c[1]), srgb_to_linear(c[2]), c[3].clamp(0.0, 1.0)], ..Default::default() };
    if let Some(r) = fx.roughness {
        p.roughness = r.clamp(0.03, 1.0);
    }
    if let Some(r) = fx.reflectiveness {
        p.reflectivity = r.clamp(0.0, 1.0);
    }
    let before = p.clone();
    p = guess_params_from_name(name, p);
    if p.kind == MaterialKind::Standard {
        p.base_color = before.base_color;
        p.roughness = before.roughness;
    }
    if c[3] < 0.95 || fx.kind == "glass" {
        p.kind = MaterialKind::Glass;
    } else if fx.kind == "metal" && p.metallic == 0.0 && !name.to_lowercase().contains("paint") {
        p.metallic = 1.0;
    } else if fx.kind == "water" {
        p.kind = MaterialKind::Water;
    }
    let texture = fx.color_tex.as_deref().and_then(|t| load_tex(path, textures, t));
    let normal_texture = fx.normal_tex.as_deref().and_then(|t| load_tex(path, textures, t)).map(crate::import::as_normal_map);
    if texture.is_some() && p.kind == MaterialKind::Standard {
        p.use_texture = true;
        p.base_color = [1.0, 1.0, 1.0, c[3]];
        if texture.as_ref().is_some_and(|t| crate::import::transparent_fraction(t) > 0.05) {
            p.cutout = true;
        }
    }
    Material { name: name.to_string(), params: p, texture, normal_texture, roughness_texture: None }
}

/// Read a COLLADA file: parts in the file's own axes/units, plus materials.
pub fn load(path: &Path, textures: &mut TextureResolver) -> Result<(Vec<MeshData>, Vec<Material>, FileHints)> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let doc = Document::parse_with_options(&text, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).context("not a valid COLLADA (XML) file")?;
    let root = doc.root_element();
    anyhow::ensure!(root.has_tag_name("COLLADA"), "not a COLLADA file");

    // Units and up axis.
    let mut hints = FileHints { up: Some(UpAxis::Y), scale_to_m: Some(1.0) };
    if let Some(asset) = child(root, "asset") {
        if let Some(m) = child(asset, "unit").and_then(|u| u.attribute("meter")).and_then(|m| m.parse::<f64>().ok()) {
            hints.scale_to_m = Some(m);
        }
        if let Some(up) = child(asset, "up_axis").and_then(|u| u.text()) {
            hints.up = Some(if up.trim().eq_ignore_ascii_case("Z_UP") { UpAxis::Z } else { UpAxis::Y });
        }
    }

    let lib = |name: &'static str| root.children().filter(move |c| c.has_tag_name(name)).flat_map(|l| l.children().filter(|c| c.is_element()));
    let images: HashMap<String, String> = lib("library_images")
        .filter_map(|i| {
            let id = i.attribute("id")?.to_string();
            let f = i.descendants().find(|d| d.has_tag_name("init_from") || d.has_tag_name("ref")).and_then(|d| d.text())?;
            Some((id, decode_uri(f)))
        })
        .collect();
    let effects: HashMap<String, Effect> = lib("library_effects").filter_map(|e| Some((e.attribute("id")?.to_string(), read_effect(e, &images)))).collect();
    let materials_xml: HashMap<String, (String, String)> = lib("library_materials")
        .filter_map(|m| {
            let id = m.attribute("id")?.to_string();
            let name = m.attribute("name").unwrap_or(&id).to_string();
            let fx = child(m, "instance_effect").and_then(|i| i.attribute("url")).map(url_id).unwrap_or("").to_string();
            Some((id, (name, fx)))
        })
        .collect();
    let geometries: HashMap<&str, Node> = lib("library_geometries").filter_map(|g| Some((g.attribute("id")?, g))).collect();
    let lib_nodes: HashMap<&str, Node> = lib("library_nodes").filter_map(|n| Some((n.attribute("id")?, n))).collect();

    // The scene to show.
    let scene_url = child(root, "scene").and_then(|s| child(s, "instance_visual_scene")).and_then(|i| i.attribute("url")).map(url_id);
    let scenes: Vec<Node> = lib("library_visual_scenes").collect();
    let scene = scene_url.and_then(|u| scenes.iter().find(|s| s.attribute("id") == Some(u)).copied()).or(scenes.first().copied());

    let mut parts = Vec::new();
    let mut used: HashMap<String, Material> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut mesh_cache: HashMap<&str, Vec<Prim>> = HashMap::new();
    let mut stack: Vec<(Node, Mat4, u32)> = Vec::new();
    if let Some(s) = scene {
        for n in children(s, "node") {
            stack.push((n, Mat4::IDENTITY, 0));
        }
    } else {
        // No scene: show every geometry as is.
        for g in geometries.values() {
            for prim in read_mesh(match child(*g, "mesh") {
                Some(m) => m,
                None => continue,
            }) {
                parts.push(prim.mesh);
            }
        }
    }
    while let Some((node, parent, depth)) = stack.pop() {
        if depth > 64 {
            continue;
        }
        let xf = parent * node_transform(node);
        for c in node.children().filter(|c| c.is_element()) {
            match c.tag_name().name() {
                "node" => stack.push((c, xf, depth + 1)),
                "instance_node" => {
                    if let Some(n) = c.attribute("url").and_then(|u| lib_nodes.get(url_id(u))) {
                        stack.push((*n, xf, depth + 1));
                    }
                }
                "instance_geometry" => {
                    let Some(gid) = c.attribute("url").map(url_id) else { continue };
                    let Some(g) = geometries.get(gid) else { continue };
                    // symbol -> material id
                    let binds: HashMap<&str, &str> = c
                        .descendants()
                        .filter(|d| d.has_tag_name("instance_material"))
                        .filter_map(|im| Some((im.attribute("symbol")?, url_id(im.attribute("target")?))))
                        .collect();
                    let prims = mesh_cache.entry(gid).or_insert_with(|| child(*g, "mesh").map(read_mesh).unwrap_or_default());
                    let nx = glam::Mat3::from_mat4(xf).inverse().transpose();
                    let flip = xf.determinant() < 0.0;
                    for prim in prims.iter() {
                        let mat_id = binds.get(prim.symbol.as_str()).copied().unwrap_or(prim.symbol.as_str());
                        let (mat_name, fx_id) = materials_xml.get(mat_id).cloned().unwrap_or_else(|| (if mat_id.is_empty() { "Default".into() } else { mat_id.to_string() }, String::new()));
                        if !used.contains_key(&mat_name) {
                            let fx = effects.get(&fx_id);
                            let m = match fx {
                                Some(fx) => material_from(&mat_name, fx, path, textures),
                                None => Material { name: mat_name.clone(), params: guess_params_from_name(&mat_name, MaterialParams::default()), texture: None, normal_texture: None, roughness_texture: None },
                            };
                            used.insert(mat_name.clone(), m);
                            order.push(mat_name.clone());
                        }
                        let mut m = MeshData { name: node.attribute("name").unwrap_or(gid).to_string(), material: mat_name, ..prim.mesh.clone() };
                        for p in &mut m.positions {
                            *p = xf.transform_point3(*p);
                        }
                        for n in &mut m.normals {
                            *n = (nx * *n).normalize_or_zero();
                        }
                        if flip {
                            for t in m.indices.chunks_exact_mut(3) {
                                t.swap(1, 2);
                            }
                        }
                        parts.push(m);
                    }
                }
                _ => {}
            }
        }
    }
    anyhow::ensure!(!parts.is_empty(), "this COLLADA file has no geometry Render Mill can read");
    let materials = order.into_iter().filter_map(|n| used.remove(&n)).collect();
    Ok((parts, materials, hints))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAE: &str = r##"<?xml version="1.0" encoding="utf-8"?>
<COLLADA xmlns="http://www.collada.org/2005/11/COLLADASchema" version="1.4.1">
  <asset><unit name="centimeter" meter="0.01"/><up_axis>Z_UP</up_axis></asset>
  <library_effects>
    <effect id="fx-brick"><profile_COMMON><technique sid="c"><lambert>
      <diffuse><color>0.6 0.3 0.2 1</color></diffuse></lambert></technique></profile_COMMON></effect>
    <effect id="fx-glass"><profile_COMMON><technique sid="c"><phong>
      <diffuse><color>0.8 0.9 0.9 1</color></diffuse>
      <transparent opaque="A_ONE"><color>0 0 0 0.3</color></transparent></phong></technique></profile_COMMON></effect>
  </library_effects>
  <library_materials>
    <material id="m-brick" name="Brick, Common"><instance_effect url="#fx-brick"/></material>
    <material id="m-glass" name="Glazing"><instance_effect url="#fx-glass"/></material>
  </library_materials>
  <library_geometries>
    <geometry id="quad"><mesh>
      <source id="p"><float_array id="pa" count="12">0 0 0 100 0 0 100 100 0 0 100 0</float_array>
        <technique_common><accessor source="#pa" count="4" stride="3"/></technique_common></source>
      <vertices id="v"><input semantic="POSITION" source="#p"/></vertices>
      <polylist material="mat" count="1"><input semantic="VERTEX" source="#v" offset="0"/>
        <vcount>4</vcount><p>0 1 2 3</p></polylist>
      <triangles material="glass" count="1"><input semantic="VERTEX" source="#v" offset="0"/><p>0 1 2</p></triangles>
    </mesh></geometry>
  </library_geometries>
  <library_visual_scenes><visual_scene id="s">
    <node id="n1"><translate>10 0 0</translate>
      <instance_geometry url="#quad"><bind_material><technique_common>
        <instance_material symbol="mat" target="#m-brick"/>
        <instance_material symbol="glass" target="#m-glass"/>
      </technique_common></bind_material></instance_geometry>
    </node>
  </visual_scene></library_visual_scenes>
  <scene><instance_visual_scene url="#s"/></scene>
</COLLADA>"##;

    #[test]
    fn reads_geometry_materials_and_units() {
        let dir = std::env::temp_dir().join(format!("rm-dae-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.dae");
        std::fs::write(&path, DAE).unwrap();
        let mut tex = TextureResolver::new(&path, &[]);
        let (parts, mats, hints) = load(&path, &mut tex).unwrap();
        assert_eq!(hints.up, Some(UpAxis::Z));
        assert_eq!(hints.scale_to_m, Some(0.01));
        assert_eq!(parts.len(), 2);
        let quad = parts.iter().find(|p| p.material == "Brick, Common").unwrap();
        assert_eq!(quad.indices.len(), 6, "quad triangulated");
        assert!(quad.positions.iter().all(|p| p.x >= 10.0), "node translation applied");
        let glass = mats.iter().find(|m| m.name == "Glazing").unwrap();
        assert_eq!(glass.params.kind, MaterialKind::Glass);
        let brick = mats.iter().find(|m| m.name == "Brick, Common").unwrap();
        assert!(brick.params.base_color[0] > brick.params.base_color[2]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decodes_file_uris() {
        assert_eq!(decode_uri("file://C:\\Users\\a\\My%20Textures\\t.png"), "C:\\Users\\a\\My Textures\\t.png");
        assert_eq!(decode_uri("./tex/a%20b.jpg"), "./tex/a b.jpg");
    }
}
