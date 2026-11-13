//! LumenCraft LiveSync wire protocol (v1).
//!
//! Plain TCP, default port 47811. Every message is one frame:
//!
//! ```text
//! u32 LE  header length (bytes)
//! [..]    UTF-8 JSON header  {"type": "...", ...}
//! u32 LE  payload length (bytes, may be 0)
//! [..]    binary payload
//! ```
//!
//! A `mesh` payload is little-endian, tightly packed, in this order:
//! positions `f32 x 3 x vertex_count`, normals `f32 x 3 x vertex_count` (if
//! `has_normals`), uvs `f32 x 2 x vertex_count` (if `has_uvs`), indices
//! `u32 x index_count`. Meshes are upserted by `id`, so a host app (BimCraft,
//! a Revit add-in, a script…) can resend just the elements that changed.
//!
//! This file has no dependencies beyond serde/serde_json so it can be copied
//! straight into BimCraft.

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

pub const DEFAULT_PORT: u16 = 47811;
pub const PROTOCOL_VERSION: u32 = 1;
const MAX_HEADER: u32 = 16 * 1024 * 1024;
const MAX_PAYLOAD: u32 = 256 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Header {
    /// First message from a client. `token` is required for connections from
    /// other computers (shown in LumenCraft's LiveSync panel); local
    /// connections don't need it.
    Hello {
        client: String,
        version: u32,
        #[serde(default)]
        token: Option<String>,
    },
    /// Server refused the connection (bad token, etc.).
    Error {
        message: String,
    },
    /// Server reply to `hello`.
    Welcome {
        server: String,
        version: u32,
    },
    /// Start of a full sync. `clear` removes everything this client sent before.
    BeginScene {
        #[serde(default)]
        clear: bool,
    },
    /// End of a full sync (the viewport refreshes immediately anyway).
    EndScene,
    Material {
        name: String,
        /// "standard" | "glass" | "water" | "emissive" | "foliage" | "landscape"
        #[serde(default)]
        kind: Option<String>,
        /// Linear RGBA.
        #[serde(default)]
        base_color: Option<[f32; 4]>,
        #[serde(default)]
        metallic: Option<f32>,
        #[serde(default)]
        roughness: Option<f32>,
        #[serde(default)]
        emissive: Option<[f32; 3]>,
        #[serde(default)]
        emissive_strength: Option<f32>,
        /// Absolute path of a texture image readable by the renderer.
        #[serde(default)]
        texture: Option<String>,
    },
    Mesh {
        id: String,
        #[serde(default)]
        name: Option<String>,
        material: String,
        vertex_count: u32,
        index_count: u32,
        #[serde(default)]
        has_normals: bool,
        #[serde(default)]
        has_uvs: bool,
        /// "y" (default) or "z" — Revit/BimCraft are Z-up.
        #[serde(default)]
        up_axis: Option<String>,
        /// "m" (default), "ft", "in", "mm", "cm" — Revit internal units are feet.
        #[serde(default)]
        units: Option<String>,
    },
    Remove {
        id: String,
    },
    /// Follow the host app's camera (like Lumion's "sync camera" option).
    Camera {
        position: [f32; 3],
        target: [f32; 3],
        #[serde(default)]
        fov: Option<f32>,
        #[serde(default)]
        up_axis: Option<String>,
        #[serde(default)]
        units: Option<String>,
    },
    Ping,
}

pub fn write_frame<W: Write>(w: &mut W, header: &Header, payload: &[u8]) -> io::Result<()> {
    let h = serde_json::to_vec(header).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    w.write_all(&(h.len() as u32).to_le_bytes())?;
    w.write_all(&h)?;
    w.write_all(&(payload.len() as u32).to_le_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

pub fn read_frame<R: Read>(r: &mut R) -> io::Result<(Header, Vec<u8>)> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let hl = u32::from_le_bytes(len);
    if hl > MAX_HEADER {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "header too large"));
    }
    let mut h = vec![0u8; hl as usize];
    r.read_exact(&mut h)?;
    r.read_exact(&mut len)?;
    let pl = u32::from_le_bytes(len);
    if pl > MAX_PAYLOAD {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "payload too large"));
    }
    let mut p = vec![0u8; pl as usize];
    r.read_exact(&mut p)?;
    let header = serde_json::from_slice(&h).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok((header, p))
}

/// Decoded mesh payload.
pub struct MeshPayload {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

fn f32s(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

pub fn decode_mesh(payload: &[u8], vertex_count: u32, index_count: u32, has_normals: bool, has_uvs: bool) -> io::Result<MeshPayload> {
    let v = vertex_count as usize;
    let need = v * 12 + if has_normals { v * 12 } else { 0 } + if has_uvs { v * 8 } else { 0 } + index_count as usize * 4;
    if payload.len() != need {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("mesh payload is {} bytes, expected {need}", payload.len())));
    }
    let mut o = 0;
    let mut take = |n: usize| {
        let s = &payload[o..o + n];
        o += n;
        s
    };
    let pos = f32s(take(v * 12));
    let nrm = if has_normals { f32s(take(v * 12)) } else { vec![] };
    let uv = if has_uvs { f32s(take(v * 8)) } else { vec![] };
    let idx = take(index_count as usize * 4).chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    Ok(MeshPayload {
        positions: pos.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
        normals: nrm.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
        uvs: uv.chunks_exact(2).map(|c| [c[0], c[1]]).collect(),
        indices: idx,
    })
}

pub fn encode_mesh(positions: &[[f32; 3]], normals: &[[f32; 3]], uvs: &[[f32; 2]], indices: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(positions.len() * 32 + indices.len() * 4);
    for p in positions.iter().chain(normals) {
        for c in p {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    for u in uvs {
        for c in u {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    for i in indices {
        out.extend_from_slice(&i.to_le_bytes());
    }
    out
}

/// Minimal blocking client — what BimCraft (or any tool) uses to push models.
pub struct SyncClient {
    stream: std::net::TcpStream,
}

impl SyncClient {
    pub fn connect(addr: &str, client_name: &str) -> io::Result<Self> {
        Self::connect_with_token(addr, client_name, None)
    }

    /// Connect and wait for the server's welcome (or refusal).
    pub fn connect_with_token(addr: &str, client_name: &str, token: Option<&str>) -> io::Result<Self> {
        let stream = std::net::TcpStream::connect(addr)?;
        stream.set_nodelay(true)?;
        let mut c = Self { stream };
        c.send(&Header::Hello { client: client_name.into(), version: PROTOCOL_VERSION, token: token.map(str::to_string) }, &[])?;
        c.stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
        let (h, _) = read_frame(&mut c.stream)?;
        c.stream.set_read_timeout(None)?;
        match h {
            Header::Welcome { .. } => Ok(c),
            Header::Error { message } => Err(io::Error::new(io::ErrorKind::PermissionDenied, message)),
            other => Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected reply {other:?}"))),
        }
    }
    pub fn send(&mut self, h: &Header, payload: &[u8]) -> io::Result<()> {
        let mut w = io::BufWriter::new(&mut self.stream);
        write_frame(&mut w, h, payload)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn mesh(
        &mut self,
        id: &str,
        material: &str,
        positions: &[[f32; 3]],
        normals: &[[f32; 3]],
        uvs: &[[f32; 2]],
        indices: &[u32],
        up_axis: &str,
        units: &str,
    ) -> io::Result<()> {
        let has_normals = normals.len() == positions.len() && !normals.is_empty();
        let has_uvs = uvs.len() == positions.len() && !uvs.is_empty();
        let payload = encode_mesh(positions, if has_normals { normals } else { &[] }, if has_uvs { uvs } else { &[] }, indices);
        self.send(
            &Header::Mesh {
                id: id.into(),
                name: None,
                material: material.into(),
                vertex_count: positions.len() as u32,
                index_count: indices.len() as u32,
                has_normals,
                has_uvs,
                up_axis: Some(up_axis.into()),
                units: Some(units.into()),
            },
            &payload,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let pos = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let nrm = [[0.0, 0.0, 1.0]; 3];
        let uv = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let idx = [0, 1, 2];
        let payload = encode_mesh(&pos, &nrm, &uv, &idx);
        let h = Header::Mesh {
            id: "a".into(),
            name: None,
            material: "m".into(),
            vertex_count: 3,
            index_count: 3,
            has_normals: true,
            has_uvs: true,
            up_axis: None,
            units: None,
        };
        let mut buf = Vec::new();
        write_frame(&mut buf, &h, &payload).unwrap();
        let (h2, p2) = read_frame(&mut &buf[..]).unwrap();
        assert_eq!(h, h2);
        let m = decode_mesh(&p2, 3, 3, true, true).unwrap();
        assert_eq!(m.positions, pos);
        assert_eq!(m.indices, idx);
    }
}
