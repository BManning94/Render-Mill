//! Autodesk FBX importer (binary 7.x incl. 7.5+ 64-bit headers, and ASCII).
//!
//! FBX is Revit's built-in 3D export (File > Export > FBX), so no third-party
//! exporter add-in is needed. Handles meshes (n-gons, normals, UVs, per-polygon
//! materials), the node hierarchy with pivots/pre-rotation/geometric offsets,
//! Phong/Lambert materials, external and embedded textures, and the file's own
//! unit scale and up axis.

use crate::import::{FileHints, TextureResolver, UpAxis};
use crate::scene::{guess_params_from_name, Material, MaterialKind, MaterialParams, MeshData, TextureData};
use anyhow::{bail, Context, Result};
use glam::{DMat4, DQuat, DVec3, EulerRot, Vec2, Vec3};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

#[derive(Clone, Debug)]
pub enum Prop {
    I(i64),
    F(f64),
    S(String),
    Raw(Vec<u8>),
    ArrI(Vec<i64>),
    ArrF(Vec<f64>),
}

impl Prop {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Prop::I(v) => Some(*v),
            Prop::F(v) => Some(*v as i64),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Prop::I(v) => Some(*v as f64),
            Prop::F(v) => Some(*v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Prop::S(s) => Some(s),
            _ => None,
        }
    }
    fn floats(&self) -> Vec<f64> {
        match self {
            Prop::ArrF(v) => v.clone(),
            Prop::ArrI(v) => v.iter().map(|x| *x as f64).collect(),
            Prop::F(v) => vec![*v],
            Prop::I(v) => vec![*v as f64],
            _ => vec![],
        }
    }
    fn ints(&self) -> Vec<i64> {
        match self {
            Prop::ArrI(v) => v.clone(),
            Prop::ArrF(v) => v.iter().map(|x| *x as i64).collect(),
            Prop::I(v) => vec![*v],
            _ => vec![],
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Node {
    pub name: String,
    pub props: Vec<Prop>,
    pub children: Vec<Node>,
}

impl Node {
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
    /// Concatenated array data of a child (ASCII arrays can be split into `a:` children).
    fn array_f(&self, name: &str) -> Vec<f64> {
        self.child(name)
            .map(
                |c| {
                    if let Some(a) = c.child("a") {
                        a.props.iter().flat_map(|p| p.floats()).collect()
                    } else {
                        c.props.iter().flat_map(|p| p.floats()).collect()
                    }
                },
            )
            .unwrap_or_default()
    }
    fn array_i(&self, name: &str) -> Vec<i64> {
        self.child(name)
            .map(|c| if let Some(a) = c.child("a") { a.props.iter().flat_map(|p| p.ints()).collect() } else { c.props.iter().flat_map(|p| p.ints()).collect() })
            .unwrap_or_default()
    }
    fn str_child(&self, name: &str) -> Option<String> {
        self.child(name).and_then(|c| c.props.first()).and_then(|p| p.as_str()).map(str::to_string)
    }
    /// Properties70 lookup: `P: "name", "type", "label", "flags", values...`
    fn p70(&self) -> HashMap<String, Vec<Prop>> {
        let mut out = HashMap::new();
        if let Some(p) = self.child("Properties70") {
            for e in p.children_named("P") {
                if let Some(Prop::S(n)) = e.props.first() {
                    out.insert(n.clone(), e.props.iter().skip(4).cloned().collect());
                }
            }
        }
        out
    }
}

// ------------------------------------------------------------------ binary

struct Bin<'a> {
    d: &'a [u8],
    o: usize,
    wide: bool,
    depth: u32,
    /// Bytes of array data decoded so far (limits decompression bombs).
    decoded: u64,
}

/// Deepest node nesting accepted (real files use ~10 levels; the limit
/// keeps a crafted file from overflowing the stack, which would abort the app).
pub const MAX_NODE_DEPTH: u32 = 128;
/// Largest single array (elements) accepted - 512 M floats is a ~170 M
/// triangle mesh, far beyond any architectural model.
pub const MAX_ARRAY_ELEMENTS: u64 = 512 << 20;
/// Total decoded array data per file.
pub const MAX_DECODED_BYTES: u64 = 8 << 30;

impl<'a> Bin<'a> {
    fn need(&self, n: usize) -> Result<()> {
        // checked: n comes from the file and may be huge
        match self.o.checked_add(n) {
            Some(end) if end <= self.d.len() => Ok(()),
            _ => bail!("FBX file is truncated or corrupt (data past end of file at offset {})", self.o),
        }
    }
    fn u8(&mut self) -> Result<u8> {
        self.need(1)?;
        self.o += 1;
        Ok(self.d[self.o - 1])
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        self.need(n)?;
        self.o += n;
        Ok(&self.d[self.o - n..self.o])
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into()?))
    }
    fn uword(&mut self) -> Result<u64> {
        if self.wide {
            Ok(u64::from_le_bytes(self.bytes(8)?.try_into()?))
        } else {
            Ok(self.u32()? as u64)
        }
    }

    fn node(&mut self) -> Result<Option<Node>> {
        let start = self.o;
        let end = usize::try_from(self.uword()?).unwrap_or(usize::MAX);
        let nprops = self.uword()?;
        let _plen = self.uword()?;
        let nlen = self.u8()? as usize;
        let name = String::from_utf8_lossy(self.bytes(nlen)?).into_owned();
        if end == 0 {
            return Ok(None);
        }
        // A node must end after its own header and inside the file;
        // otherwise parsing could loop forever or jump backwards.
        if end > self.d.len() || end <= start || end < self.o {
            bail!("FBX node '{name}' has an invalid end offset {end} (corrupt file)");
        }
        if self.depth >= MAX_NODE_DEPTH {
            bail!("FBX nodes nested deeper than {MAX_NODE_DEPTH} levels (corrupt file)");
        }
        // Every property takes at least 2 bytes.
        if nprops > ((end - self.o) / 2) as u64 {
            bail!("FBX node '{name}' declares {nprops} properties, more than fit in it");
        }
        let mut node = Node { name, ..Default::default() };
        for _ in 0..nprops {
            node.props.push(self.prop()?);
        }
        if self.o > end {
            bail!("FBX node '{}' properties overrun the node (corrupt file)", node.name);
        }
        let null_len = if self.wide { 25 } else { 13 };
        self.depth += 1;
        while self.o + null_len <= end && self.o < end {
            let before = self.o;
            match self.node() {
                Ok(Some(c)) => node.children.push(c),
                Ok(None) => break,
                Err(e) => {
                    self.depth -= 1;
                    return Err(e);
                }
            }
            if self.o <= before {
                self.depth -= 1;
                bail!("FBX parse made no progress inside '{}' (corrupt file)", node.name);
            }
        }
        self.depth -= 1;
        self.o = end;
        Ok(Some(node))
    }

    fn prop(&mut self) -> Result<Prop> {
        let t = self.u8()?;
        Ok(match t {
            b'Y' => Prop::I(i16::from_le_bytes(self.bytes(2)?.try_into()?) as i64),
            b'C' => Prop::I(self.u8()? as i64),
            b'I' => Prop::I(i32::from_le_bytes(self.bytes(4)?.try_into()?) as i64),
            b'L' => Prop::I(i64::from_le_bytes(self.bytes(8)?.try_into()?)),
            b'F' => Prop::F(f32::from_le_bytes(self.bytes(4)?.try_into()?) as f64),
            b'D' => Prop::F(f64::from_le_bytes(self.bytes(8)?.try_into()?)),
            b'S' => {
                let n = self.u32()? as usize;
                Prop::S(String::from_utf8_lossy(self.bytes(n)?).into_owned())
            }
            b'R' => {
                let n = self.u32()? as usize;
                Prop::Raw(self.bytes(n)?.to_vec())
            }
            b'f' | b'd' | b'l' | b'i' | b'b' => {
                let count = self.u32()? as u64;
                let enc = self.u32()?;
                let clen = self.u32()? as usize;
                let raw = self.bytes(clen)?;
                let elem: u64 = match t {
                    b'f' | b'i' => 4,
                    b'd' | b'l' => 8,
                    _ => 1,
                };
                if count > MAX_ARRAY_ELEMENTS {
                    bail!("FBX array of {count} elements exceeds the {MAX_ARRAY_ELEMENTS} limit");
                }
                let want = count * elem; // can't overflow: count <= 2^29, elem <= 8
                self.decoded += want;
                if self.decoded > MAX_DECODED_BYTES {
                    bail!("FBX file decodes to more than {} GB of array data", MAX_DECODED_BYTES >> 30);
                }
                let want = want as usize;
                let data: Vec<u8> = match enc {
                    0 => {
                        if raw.len() < want {
                            bail!("FBX array shorter than declared");
                        }
                        raw[..want].to_vec()
                    }
                    1 => {
                        // Never trust the declared size for the allocation, and
                        // stop decompressing at exactly the declared size (zip bombs).
                        let mut out = Vec::with_capacity(want.min(64 << 20));
                        flate2::read::ZlibDecoder::new(raw).take(want as u64).read_to_end(&mut out).context("FBX zlib array is corrupt")?;
                        out
                    }
                    other => bail!("unsupported FBX array encoding {other}"),
                };
                if data.len() < want {
                    bail!("FBX array shorter than declared");
                }
                let c = &data[..want];
                match t {
                    b'f' => Prop::ArrF(c.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap()) as f64).collect()),
                    b'd' => Prop::ArrF(c.chunks_exact(8).map(|b| f64::from_le_bytes(b.try_into().unwrap())).collect()),
                    b'i' => Prop::ArrI(c.chunks_exact(4).map(|b| i32::from_le_bytes(b.try_into().unwrap()) as i64).collect()),
                    b'l' => Prop::ArrI(c.chunks_exact(8).map(|b| i64::from_le_bytes(b.try_into().unwrap())).collect()),
                    _ => Prop::ArrI(c.iter().map(|b| *b as i64).collect()),
                }
            }
            other => bail!("unknown FBX property type '{}'", other as char),
        })
    }
}

pub fn parse_binary(d: &[u8]) -> Result<(u32, Vec<Node>)> {
    if d.len() < 27 || !d.starts_with(b"Kaydara FBX Binary") {
        bail!("not a binary FBX file");
    }
    let version = u32::from_le_bytes(d[23..27].try_into()?);
    let mut b = Bin { d, o: 27, wide: version >= 7500, depth: 0, decoded: 0 };
    let mut nodes = Vec::new();
    while b.o < d.len() {
        match b.node() {
            Ok(Some(n)) => nodes.push(n),
            Ok(None) => break,
            Err(e) => {
                if nodes.iter().any(|n| n.name == "Objects") {
                    break; // footer garbage after the useful part
                }
                return Err(e);
            }
        }
    }
    Ok((version, nodes))
}

// ------------------------------------------------------------------ ascii

pub fn parse_ascii(text: &str) -> Result<Vec<Node>> {
    #[derive(Debug, PartialEq, Clone)]
    enum Tok {
        Key(String),
        Str(String),
        Num(String),
        Word(String),
        Comma,
        Open,
        Close,
        Star(usize),
    }
    let mut toks = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b';' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'"' {
            let s = i + 1;
            i += 1;
            while i < b.len() && b[i] != b'"' {
                i += 1;
            }
            toks.push(Tok::Str(text[s..i.min(b.len())].replace("&quot;", "\"")));
            i += 1;
        } else if c == b',' {
            toks.push(Tok::Comma);
            i += 1;
        } else if c == b'{' {
            toks.push(Tok::Open);
            i += 1;
        } else if c == b'}' {
            toks.push(Tok::Close);
            i += 1;
        } else if c == b'*' {
            let s = i + 1;
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            toks.push(Tok::Star(text[s..i].parse().unwrap_or(0)));
        } else {
            let s = i;
            while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b',' | b'{' | b'}' | b'"' | b':') {
                i += 1;
            }
            let w = &text[s..i];
            if i < b.len() && b[i] == b':' {
                toks.push(Tok::Key(w.to_string()));
                i += 1;
            } else if w.starts_with(|ch: char| ch.is_ascii_digit() || ch == '-' || ch == '+' || ch == '.') {
                toks.push(Tok::Num(w.to_string()));
            } else if !w.is_empty() {
                toks.push(Tok::Word(w.to_string()));
            } else {
                i += 1;
            }
        }
    }

    fn num(s: &str) -> Prop {
        if !s.contains(['.', 'e', 'E']) {
            if let Ok(v) = s.parse::<i64>() {
                return Prop::I(v);
            }
        }
        Prop::F(s.parse().unwrap_or(0.0))
    }

    fn parse_list(toks: &[Tok], pos: &mut usize, depth: usize) -> Result<Vec<Node>> {
        if depth > MAX_NODE_DEPTH as usize {
            bail!("ASCII FBX nested deeper than {MAX_NODE_DEPTH} levels (corrupt file)");
        }
        let mut out = Vec::new();
        while *pos < toks.len() {
            match &toks[*pos] {
                Tok::Close => {
                    if depth == 0 {
                        bail!("unbalanced '}}' in ASCII FBX");
                    }
                    *pos += 1;
                    return Ok(out);
                }
                Tok::Key(k) => {
                    *pos += 1;
                    let mut node = Node { name: k.clone(), ..Default::default() };
                    let mut star = false;
                    while *pos < toks.len() {
                        match &toks[*pos] {
                            Tok::Str(s) => node.props.push(Prop::S(s.clone())),
                            Tok::Num(n) => node.props.push(num(n)),
                            Tok::Word(w) => node.props.push(Prop::S(w.clone())),
                            Tok::Star(_) => star = true,
                            Tok::Comma => {}
                            _ => break,
                        }
                        *pos += 1;
                    }
                    if *pos < toks.len() && toks[*pos] == Tok::Open {
                        *pos += 1;
                        node.children = parse_list(toks, pos, depth + 1)?;
                    }
                    if star {
                        node.props.clear();
                    }
                    out.push(node);
                }
                _ => *pos += 1,
            }
        }
        Ok(out)
    }
    let mut pos = 0;
    parse_list(&toks, &mut pos, 0)
}

// ------------------------------------------------------------------ scene

fn obj_name(p: Option<&Prop>) -> String {
    let s = p.and_then(|p| p.as_str()).unwrap_or("");
    // binary: "Name\0\x01Class", ascii: "Class::Name"
    if let Some(i) = s.find("\u{0}\u{1}") {
        return s[..i].to_string();
    }
    if let Some(i) = s.find("::") {
        return s[i + 2..].to_string();
    }
    s.to_string()
}

fn v3(p: &HashMap<String, Vec<Prop>>, k: &str, d: DVec3) -> DVec3 {
    p.get(k).and_then(|v| Some(DVec3::new(v.first()?.as_f64()?, v.get(1)?.as_f64()?, v.get(2)?.as_f64()?))).unwrap_or(d)
}

fn f1(p: &HashMap<String, Vec<Prop>>, k: &str) -> Option<f64> {
    p.get(k).and_then(|v| v.first()).and_then(|x| x.as_f64())
}

fn euler(r: DVec3, order: i64) -> DQuat {
    let (x, y, z) = (r.x.to_radians(), r.y.to_radians(), r.z.to_radians());
    match order {
        1 => DQuat::from_euler(EulerRot::YZX, y, z, x), // XZY
        2 => DQuat::from_euler(EulerRot::XZY, x, z, y), // YZX
        3 => DQuat::from_euler(EulerRot::ZXY, z, x, y), // YXZ
        4 => DQuat::from_euler(EulerRot::YXZ, y, x, z), // ZXY
        5 => DQuat::from_euler(EulerRot::XYZ, x, y, z), // ZYX
        _ => DQuat::from_euler(EulerRot::ZYX, z, y, x), // XYZ (default)
    }
}

fn local_transform(p: &HashMap<String, Vec<Prop>>) -> DMat4 {
    let order = f1(p, "RotationOrder").unwrap_or(0.0) as i64;
    let t = DMat4::from_translation(v3(p, "Lcl Translation", DVec3::ZERO));
    let roff = DMat4::from_translation(v3(p, "RotationOffset", DVec3::ZERO));
    let rp = DMat4::from_translation(v3(p, "RotationPivot", DVec3::ZERO));
    let pre = DMat4::from_quat(euler(v3(p, "PreRotation", DVec3::ZERO), 0));
    let r = DMat4::from_quat(euler(v3(p, "Lcl Rotation", DVec3::ZERO), order));
    let post = DMat4::from_quat(euler(v3(p, "PostRotation", DVec3::ZERO), 0)).inverse();
    let soff = DMat4::from_translation(v3(p, "ScalingOffset", DVec3::ZERO));
    let sp = DMat4::from_translation(v3(p, "ScalingPivot", DVec3::ZERO));
    let s = DMat4::from_scale(v3(p, "Lcl Scaling", DVec3::ONE));
    t * roff * rp * pre * r * post * rp.inverse() * soff * sp * s * sp.inverse()
}

fn geometric_transform(p: &HashMap<String, Vec<Prop>>) -> DMat4 {
    DMat4::from_translation(v3(p, "GeometricTranslation", DVec3::ZERO))
        * DMat4::from_quat(euler(v3(p, "GeometricRotation", DVec3::ZERO), 0))
        * DMat4::from_scale(v3(p, "GeometricScaling", DVec3::ONE))
}

pub struct FbxScene {
    pub meshes: Vec<(String, MeshData)>,
    pub materials: Vec<Material>,
    pub hints: FileHints,
}

pub fn load(path: &Path, resolver: &mut TextureResolver) -> Result<FbxScene> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let nodes = if data.starts_with(b"Kaydara FBX Binary") { parse_binary(&data)?.1 } else { parse_ascii(&String::from_utf8_lossy(&data))? };
    build_scene(&nodes, resolver)
}

/// Mapping helper for LayerElement data.
fn layer_index(mapping: &str, reference: &str, index_arr: &[i64], pv: usize, vert: usize, poly: usize) -> Option<usize> {
    let direct = match mapping {
        "ByPolygonVertex" => pv,
        "ByVertice" | "ByVertex" => vert,
        "ByPolygon" => poly,
        "AllSame" => 0,
        _ => pv,
    };
    if reference == "IndexToDirect" || reference == "Index" {
        index_arr.get(direct).and_then(|i| usize::try_from(*i).ok())
    } else {
        Some(direct)
    }
}

pub fn build_scene(nodes: &[Node], resolver: &mut TextureResolver) -> Result<FbxScene> {
    let find = |n: &str| nodes.iter().find(|x| x.name == n);

    // Units & axes
    let mut hints = FileHints { up: Some(UpAxis::Y), scale_to_m: Some(0.01) };
    if let Some(gs) = find("GlobalSettings") {
        let p = gs.p70();
        if let Some(u) = f1(&p, "UnitScaleFactor") {
            hints.scale_to_m = Some(u * 0.01);
        }
        if let Some(a) = f1(&p, "UpAxis") {
            hints.up = Some(if a as i64 == 2 { UpAxis::Z } else { UpAxis::Y });
        }
    }

    let objects = find("Objects").context("FBX has no Objects section")?;
    let mut by_id: HashMap<i64, &Node> = HashMap::new();
    for o in &objects.children {
        if let Some(id) = o.props.first().and_then(|p| p.as_i64()) {
            by_id.insert(id, o);
        }
    }

    // Connections: child -> parents, parent -> ordered children (with property)
    let mut parents: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut children: HashMap<i64, Vec<(i64, Option<String>)>> = HashMap::new();
    if let Some(c) = find("Connections") {
        for e in c.children_named("C") {
            let kind = e.props.first().and_then(|p| p.as_str()).unwrap_or("");
            let (Some(a), Some(b)) = (e.props.get(1).and_then(|p| p.as_i64()), e.props.get(2).and_then(|p| p.as_i64())) else {
                continue;
            };
            let prop = if kind == "OP" { e.props.get(3).and_then(|p| p.as_str()).map(str::to_string) } else { None };
            parents.entry(a).or_default().push(b);
            children.entry(b).or_default().push((a, prop));
        }
    }

    // Embedded images: Video nodes carry `Content` (raw bytes in binary FBX,
    // base64 in ASCII). Exporters often embed each file once and leave later
    // Videos for the same file empty, so index content by file name too.
    let video_bytes = |v: &Node| -> Option<Vec<u8>> {
        match v.child("Content").and_then(|c| c.props.iter().find(|p| !matches!(p, Prop::S(s) if s.is_empty()))) {
            Some(Prop::Raw(b)) if !b.is_empty() => Some(b.clone()),
            Some(Prop::S(b64)) => {
                use base64::Engine;
                let clean: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
                base64::engine::general_purpose::STANDARD.decode(clean).ok().filter(|b| !b.is_empty())
            }
            _ => None,
        }
    };
    let base_name = |n: &str| n.replace('\\', "/").rsplit('/').next().unwrap_or("").to_lowercase();
    let mut embedded_by_name: HashMap<String, Vec<u8>> = HashMap::new();
    for v in objects.children_named("Video") {
        if let Some(bytes) = video_bytes(v) {
            for n in [v.str_child("RelativeFilename"), v.str_child("Filename"), v.str_child("FileName")].into_iter().flatten() {
                embedded_by_name.entry(base_name(&n)).or_insert_with(|| bytes.clone());
            }
        }
    }

    // Load a Texture object: embedded data first, then the files it names.
    let mut tex_cache: HashMap<i64, Option<TextureData>> = HashMap::new();
    let mut load_texture = |tex_id: i64, resolver: &mut TextureResolver| -> Option<TextureData> {
        if let Some(t) = tex_cache.get(&tex_id) {
            return t.clone();
        }
        let tex = by_id.get(&tex_id)?;
        let names: Vec<String> = [tex.str_child("RelativeFilename"), tex.str_child("FileName"), tex.str_child("Filename")]
            .into_iter()
            .flatten()
            .filter(|n| !n.trim().is_empty())
            .collect();
        let mut t = None;
        for (cid, _) in children.get(&tex_id).into_iter().flatten() {
            if let Some(v) = by_id.get(cid).filter(|v| v.name == "Video") {
                t = video_bytes(v).and_then(|b| crate::import::texture_from_bytes(&b));
                if t.is_none() {
                    for n in [v.str_child("RelativeFilename"), v.str_child("Filename")].into_iter().flatten() {
                        if let Some(b) = embedded_by_name.get(&base_name(&n)) {
                            t = crate::import::texture_from_bytes(b);
                        }
                    }
                }
            }
        }
        if t.is_none() {
            for n in &names {
                if let Some(b) = embedded_by_name.get(&base_name(n)) {
                    t = crate::import::texture_from_bytes(b);
                    if t.is_some() {
                        break;
                    }
                }
            }
        }
        if t.is_none() {
            // External file: try every name before recording it as missing.
            for n in &names {
                if resolver.resolve(n).is_some() {
                    t = resolver.load(n);
                    break;
                }
            }
            if t.is_none() {
                if let Some(n) = names.first() {
                    t = resolver.load(n);
                }
            }
        }
        tex_cache.insert(tex_id, t.clone());
        t
    };

    // Texture placement (tiling/offset) stored on the Texture object.
    let tex_uv = |tex_id: i64| -> [f32; 4] {
        let Some(tex) = by_id.get(&tex_id) else { return [1.0, 1.0, 0.0, 0.0] };
        let two = |n: &str| -> Option<[f64; 2]> {
            let c = tex.child(n)?;
            Some([c.props.first()?.as_f64()?, c.props.get(1)?.as_f64()?])
        };
        let p = tex.p70();
        let sc = two("ModelUVScaling").unwrap_or_else(|| {
            let v = v3(&p, "Scaling", DVec3::ONE);
            [v.x, v.y]
        });
        let tr = two("ModelUVTranslation").unwrap_or_else(|| {
            let v = v3(&p, "Translation", DVec3::ZERO);
            [v.x, v.y]
        });
        let (sx, sy) = (if sc[0].abs() < 1e-6 { 1.0 } else { sc[0] }, if sc[1].abs() < 1e-6 { 1.0 } else { sc[1] });
        // We flip v on import, so the v offset is mirrored to match.
        [sx as f32, sy as f32, tr[0] as f32, (1.0 - sy - tr[1]) as f32]
    };

    let mut materials: Vec<Material> = Vec::new();
    let mut mat_names: HashMap<i64, String> = HashMap::new();
    for o in objects.children_named("Material") {
        let Some(id) = o.props.first().and_then(|p| p.as_i64()) else { continue };
        let mut name = obj_name(o.props.get(1));
        if name.is_empty() {
            name = format!("Material {id}");
        }
        let p = o.p70();
        let diffuse = v3(&p, "DiffuseColor", v3(&p, "Diffuse", DVec3::splat(0.8)));
        let dfac = f1(&p, "DiffuseFactor").unwrap_or(1.0);
        let opacity = if let Some(op) = f1(&p, "Opacity") {
            op
        } else if let Some(tf) = f1(&p, "TransparencyFactor") {
            let tc = v3(&p, "TransparentColor", DVec3::ONE);
            1.0 - tf * (tc.x + tc.y + tc.z) / 3.0
        } else {
            1.0
        }
        .clamp(0.0, 1.0);
        let shin = f1(&p, "ShininessExponent").or_else(|| f1(&p, "Shininess")).unwrap_or(10.0).max(0.0);
        let emissive = v3(&p, "EmissiveColor", v3(&p, "Emissive", DVec3::ZERO)) * f1(&p, "EmissiveFactor").unwrap_or(1.0);
        let c = (diffuse * dfac).as_vec3();
        // FBX colours are authored in sRGB-ish space; renderer works in linear.
        let lin = |x: f32| x.clamp(0.0, 1.0).powf(2.2);
        let mut params = MaterialParams {
            base_color: [lin(c.x), lin(c.y), lin(c.z), opacity as f32],
            roughness: ((2.0 / (shin + 2.0)).sqrt() as f32).clamp(0.05, 1.0),
            ..Default::default()
        };
        if emissive.max_element() > 0.01 {
            params.emissive = emissive.as_vec3().to_array();
            params.emissive_strength = 4.0;
            params.kind = MaterialKind::Emissive;
        }
        let before = params.clone();
        params = guess_params_from_name(&name, params);
        if params.kind == MaterialKind::Standard || params.kind == MaterialKind::Emissive {
            params.base_color = before.base_color;
            if opacity < 0.95 && params.kind == MaterialKind::Standard {
                params.kind = MaterialKind::Glass;
            }
        }
        // Sort connected textures into slots by the material property they feed.
        let (mut color_id, mut normal_id, mut bump_id, mut opacity_id, mut transp_id) = (None, None, None, None, None);
        for (cid, prop) in children.get(&id).into_iter().flatten() {
            if by_id.get(cid).is_none_or(|n| n.name != "Texture") {
                continue;
            }
            let pl = prop.as_deref().unwrap_or("").to_ascii_lowercase();
            let slot = if pl.is_empty() || pl.contains("diffuse") || pl.contains("basecolor") || pl.contains("base_color") || pl.contains("albedo") {
                &mut color_id
            } else if pl.contains("normal") {
                &mut normal_id
            } else if pl.contains("bump") {
                &mut bump_id
            } else if pl.contains("opacity") || pl.contains("cutout") {
                &mut opacity_id
            } else if pl.contains("transparen") {
                &mut transp_id
            } else {
                continue;
            };
            if slot.is_none() {
                *slot = Some(*cid);
            }
        }
        let mut texture = color_id.and_then(|t| load_texture(t, resolver));
        if let Some(t) = color_id {
            params.uv_transform = tex_uv(t);
        }
        let normal_texture =
            normal_id.and_then(|t| load_texture(t, resolver)).or_else(|| bump_id.and_then(|t| load_texture(t, resolver))).map(crate::import::as_normal_map);
        if let Some(bf) = f1(&p, "BumpFactor") {
            params.normal_strength = (bf as f32).clamp(0.0, 4.0);
        }
        let opacity_map = opacity_id
            .and_then(|t| load_texture(t, resolver))
            .map(|t| (t, false))
            .or_else(|| transp_id.and_then(|t| load_texture(t, resolver)).map(|t| (t, true)));
        if let Some((op, invert)) = opacity_map {
            texture = Some(crate::import::merge_opacity(texture, &op, invert));
            if params.kind == MaterialKind::Glass && opacity < 0.95 {
                // Map drives the see-through parts; keep the material solid elsewhere.
                params.base_color[3] = 1.0;
            }
            params.kind = MaterialKind::Standard;
            params.cutout = true;
        }
        if texture.is_some() && params.kind == MaterialKind::Standard {
            params.base_color = [1.0, 1.0, 1.0, params.base_color[3]];
        }
        if params.kind != MaterialKind::Glass && texture.as_ref().is_some_and(|t| crate::import::transparent_fraction(t) > 0.05) {
            params.cutout = true;
        }
        // Avoid duplicate names (Revit sometimes repeats them) clobbering each other.
        let mut unique = name.clone();
        let mut k = 2;
        while materials.iter().any(|m| m.name == unique) {
            unique = format!("{name} ({k})");
            k += 1;
        }
        mat_names.insert(id, unique.clone());
        materials.push(Material { name: unique, params, texture, normal_texture, roughness_texture: None });
    }

    // Model world transforms (memoised walk up the parent chain)
    let mut world_cache: HashMap<i64, DMat4> = HashMap::new();
    fn model_world(id: i64, by_id: &HashMap<i64, &Node>, parents: &HashMap<i64, Vec<i64>>, cache: &mut HashMap<i64, DMat4>, depth: u32) -> DMat4 {
        if let Some(m) = cache.get(&id) {
            return *m;
        }
        let Some(node) = by_id.get(&id) else { return DMat4::IDENTITY };
        let local = local_transform(&node.p70());
        let parent = parents.get(&id).and_then(|ps| ps.iter().find(|p| by_id.get(p).is_some_and(|n| n.name == "Model"))).copied();
        let w = match parent {
            Some(p) if depth < 256 => model_world(p, by_id, parents, cache, depth + 1) * local,
            _ => local,
        };
        cache.insert(id, w);
        w
    }

    let mut meshes: Vec<(String, MeshData)> = Vec::new();
    for g in objects.children_named("Geometry") {
        let Some(gid) = g.props.first().and_then(|p| p.as_i64()) else { continue };
        if g.props.get(2).and_then(|p| p.as_str()).is_some_and(|t| t != "Mesh") {
            continue;
        }
        let verts = g.array_f("Vertices");
        let pvi = g.array_i("PolygonVertexIndex");
        if verts.len() < 9 || pvi.len() < 3 {
            continue;
        }
        let positions: Vec<DVec3> = verts.chunks_exact(3).map(|c| DVec3::new(c[0], c[1], c[2])).collect();

        // Layers
        let nrm_layer = g.child("LayerElementNormal");
        let (n_map, n_ref, n_data, n_idx) = nrm_layer
            .map(|l| {
                (
                    l.str_child("MappingInformationType").unwrap_or_default(),
                    l.str_child("ReferenceInformationType").unwrap_or_default(),
                    l.array_f("Normals"),
                    l.array_i("NormalsIndex"),
                )
            })
            .unwrap_or_default();
        let uv_layer = g.child("LayerElementUV");
        let (u_map, u_ref, u_data, u_idx) = uv_layer
            .map(|l| {
                (
                    l.str_child("MappingInformationType").unwrap_or_default(),
                    l.str_child("ReferenceInformationType").unwrap_or_default(),
                    l.array_f("UV"),
                    l.array_i("UVIndex"),
                )
            })
            .unwrap_or_default();
        let mat_layer = g.child("LayerElementMaterial");
        let (m_map, m_data) = mat_layer
            .map(|l| (l.str_child("MappingInformationType").unwrap_or_else(|| "AllSame".into()), l.array_i("Materials")))
            .unwrap_or_else(|| ("AllSame".into(), vec![0]));

        // Models instancing this geometry
        let models: Vec<i64> =
            parents.get(&gid).map(|ps| ps.iter().copied().filter(|p| by_id.get(p).is_some_and(|n| n.name == "Model")).collect()).unwrap_or_default();
        let models = if models.is_empty() { vec![i64::MIN] } else { models };

        for model in models {
            let (world, mats_for_model) = if model == i64::MIN {
                (DMat4::IDENTITY, Vec::new())
            } else {
                let w = model_world(model, &by_id, &parents, &mut world_cache, 0) * geometric_transform(&by_id[&model].p70());
                let mats: Vec<String> = children.get(&model).into_iter().flatten().filter_map(|(c, _)| mat_names.get(c).cloned()).collect();
                (w, mats)
            };
            let nmat = glam::DMat3::from_mat4(world).inverse().transpose();
            let flip = world.determinant() < 0.0;
            let model_name = if model == i64::MIN { obj_name(g.props.get(1)) } else { obj_name(by_id[&model].props.get(1)) };

            // Per material slot output
            let mut out: HashMap<usize, MeshData> = HashMap::new();
            let mut poly_start = 0usize;
            let mut poly = 0usize;
            let mut i = 0usize;
            while i < pvi.len() {
                // find polygon end (negative index)
                let mut j = i;
                while j < pvi.len() && pvi[j] >= 0 {
                    j += 1;
                }
                if j >= pvi.len() {
                    break;
                }
                let corners: Vec<(usize, usize)> = (i..=j)
                    .map(|k| {
                        let v = if pvi[k] < 0 { (!pvi[k]) as usize } else { pvi[k] as usize };
                        (k, v)
                    })
                    .collect();
                let slot = layer_index(&m_map, "Direct", &[], 0, 0, poly).and_then(|ix| m_data.get(ix)).copied().unwrap_or(0).max(0) as usize;
                let mesh = out.entry(slot).or_insert_with(|| MeshData {
                    name: model_name.clone(),
                    material: mats_for_model.get(slot).cloned().unwrap_or_else(|| "Default".into()),
                    ..Default::default()
                });
                let base = mesh.positions.len() as u32;
                for &(pv, v) in &corners {
                    let Some(p) = positions.get(v) else { continue };
                    mesh.positions.push(world.transform_point3(*p).as_vec3());
                    let n = layer_index(&n_map, &n_ref, &n_idx, pv, v, poly)
                        .and_then(|k| n_data.get(k.checked_mul(3)?..k.checked_mul(3)? + 3))
                        .map(|c| (nmat * DVec3::new(c[0], c[1], c[2])).normalize_or_zero().as_vec3())
                        .unwrap_or(Vec3::ZERO);
                    mesh.normals.push(n);
                    let uv = layer_index(&u_map, &u_ref, &u_idx, pv, v, poly)
                        .and_then(|k| u_data.get(k.checked_mul(2)?..k.checked_mul(2)? + 2))
                        .map(|c| Vec2::new(c[0] as f32, 1.0 - c[1] as f32))
                        .unwrap_or(Vec2::NAN);
                    mesh.uvs.push(uv);
                }
                let n = (mesh.positions.len() as u32) - base;
                for k in 1..n.saturating_sub(1) {
                    if flip {
                        mesh.indices.extend_from_slice(&[base, base + k + 1, base + k]);
                    } else {
                        mesh.indices.extend_from_slice(&[base, base + k, base + k + 1]);
                    }
                }
                poly += 1;
                poly_start = j + 1;
                i = j + 1;
            }
            let _ = poly_start;
            for (slot, mut m) in out {
                // Missing normals/uvs → recomputed by MeshData::finalize
                if m.normals.contains(&Vec3::ZERO) {
                    m.normals.clear();
                }
                if m.uvs.iter().any(|u| u.x.is_nan()) {
                    m.uvs.clear();
                }
                meshes.push((format!("{model}:{gid}:{slot}"), m));
            }
        }
    }
    if meshes.is_empty() {
        bail!("FBX contains no mesh geometry");
    }
    // Exact duplicates (same material, same vertices, same triangles) are
    // common in exports (e.g. an element placed twice at one spot). They
    // z-fight in the rasterizer and double the path tracer's work.
    let before = meshes.len();
    let mut seen = std::collections::HashSet::new();
    meshes.retain(|(_, m)| {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        m.material.hash(&mut h);
        for p in &m.positions {
            [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()].hash(&mut h);
        }
        m.indices.hash(&mut h);
        seen.insert((h.finish(), m.positions.len(), m.indices.len()))
    });
    if meshes.len() < before {
        log::info!("FBX: removed {} exact duplicate meshes", before - meshes.len());
    }
    Ok(FbxScene { meshes, materials, hints })
}

/// Re-encode a parsed node tree as binary FBX (used by tests to exercise the
/// 64-bit 7.5 header path that Revit 2020+ writes).
pub fn write_binary(nodes: &[Node], version: u32) -> Vec<u8> {
    let wide = version >= 7500;
    let mut out = b"Kaydara FBX Binary  \x00\x1a\x00".to_vec();
    out.extend_from_slice(&version.to_le_bytes());
    fn word(out: &mut Vec<u8>, v: u64, wide: bool) {
        if wide {
            out.extend_from_slice(&v.to_le_bytes());
        } else {
            out.extend_from_slice(&(v as u32).to_le_bytes());
        }
    }
    fn prop(out: &mut Vec<u8>, p: &Prop) {
        match p {
            Prop::I(v) => {
                out.push(b'L');
                out.extend_from_slice(&v.to_le_bytes());
            }
            Prop::F(v) => {
                out.push(b'D');
                out.extend_from_slice(&v.to_le_bytes());
            }
            Prop::S(s) => {
                out.push(b'S');
                out.extend_from_slice(&(s.len() as u32).to_le_bytes());
                out.extend_from_slice(s.as_bytes());
            }
            Prop::Raw(r) => {
                out.push(b'R');
                out.extend_from_slice(&(r.len() as u32).to_le_bytes());
                out.extend_from_slice(r);
            }
            Prop::ArrI(a) => {
                out.push(b'l');
                out.extend_from_slice(&(a.len() as u32).to_le_bytes());
                out.extend_from_slice(&0u32.to_le_bytes());
                out.extend_from_slice(&((a.len() * 8) as u32).to_le_bytes());
                for v in a {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
            Prop::ArrF(a) => {
                out.push(b'd');
                out.extend_from_slice(&(a.len() as u32).to_le_bytes());
                out.extend_from_slice(&0u32.to_le_bytes());
                out.extend_from_slice(&((a.len() * 8) as u32).to_le_bytes());
                for v in a {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
    }
    fn node(out: &mut Vec<u8>, n: &Node, wide: bool) {
        let start = out.len();
        word(out, 0, wide);
        word(out, n.props.len() as u64, wide);
        let plen_pos = out.len();
        word(out, 0, wide);
        out.push(n.name.len() as u8);
        out.extend_from_slice(n.name.as_bytes());
        let ps = out.len();
        for p in &n.props {
            prop(out, p);
        }
        let plen = (out.len() - ps) as u64;
        if !n.children.is_empty() {
            for c in &n.children {
                node(out, c, wide);
            }
            out.extend(std::iter::repeat_n(0u8, if wide { 25 } else { 13 }));
        }
        let end = out.len() as u64;
        let mut tmp = Vec::new();
        word(&mut tmp, end, wide);
        out[start..start + tmp.len()].copy_from_slice(&tmp);
        tmp.clear();
        word(&mut tmp, plen, wide);
        out[plen_pos..plen_pos + tmp.len()].copy_from_slice(&tmp);
    }
    for n in nodes {
        node(&mut out, n, wide);
    }
    out.extend(std::iter::repeat_n(0u8, if wide { 25 } else { 13 }));
    out
}
