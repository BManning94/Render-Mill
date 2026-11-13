//! Texture resampling and mip-chain generation shared by the rasterizer and
//! the path tracer.
//!
//! Colour maps are stored sRGB-encoded, so they are averaged in linear light
//! (averaging encoded bytes darkens mips and shifts the contrast of fine
//! patterns such as brick mortar or shingle gaps with distance). Normal maps
//! are averaged as vectors and renormalised so relief doesn't flatten
//! unevenly. Other data maps (roughness, opacity) are averaged directly.

use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MapKind {
    /// sRGB colour (+ linear alpha).
    Srgb,
    /// Tangent-space normal map (RGB = XYZ * 0.5 + 0.5).
    Normal,
    /// Any other linear data (roughness, metallic, masks).
    Linear,
}

fn srgb_to_linear_lut() -> &'static [f32; 256] {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut t = [0.0; 256];
        for (i, v) in t.iter_mut().enumerate() {
            let c = i as f32 / 255.0;
            *v = if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        }
        t
    })
}

fn linear_to_srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

/// Halve an RGBA8 image (box filter, odd edges clamped).
pub fn downsample(w: u32, h: u32, data: &[u8], kind: MapKind) -> (u32, u32, Vec<u8>) {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let lut = srgb_to_linear_lut();
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        for x in 0..nw {
            let mut acc = [0f32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let sx = (x * 2 + dx).min(w - 1);
                let sy = (y * 2 + dy).min(h - 1);
                let i = ((sy * w + sx) * 4) as usize;
                match kind {
                    MapKind::Srgb => {
                        for c in 0..3 {
                            acc[c] += lut[data[i + c] as usize];
                        }
                        acc[3] += data[i + 3] as f32;
                    }
                    MapKind::Normal => {
                        for c in 0..3 {
                            acc[c] += data[i + c] as f32 / 127.5 - 1.0;
                        }
                        acc[3] += data[i + 3] as f32;
                    }
                    MapKind::Linear => {
                        for c in 0..4 {
                            acc[c] += data[i + c] as f32;
                        }
                    }
                }
            }
            let o = ((y * nw + x) * 4) as usize;
            match kind {
                MapKind::Srgb => {
                    for c in 0..3 {
                        out[o + c] = linear_to_srgb(acc[c] * 0.25);
                    }
                    out[o + 3] = (acc[3] * 0.25 + 0.5) as u8;
                }
                MapKind::Normal => {
                    let len = (acc[0] * acc[0] + acc[1] * acc[1] + acc[2] * acc[2]).sqrt();
                    let n = if len > 1e-6 { [acc[0] / len, acc[1] / len, acc[2] / len] } else { [0.0, 0.0, 1.0] };
                    for c in 0..3 {
                        out[o + c] = ((n[c] * 0.5 + 0.5) * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
                    }
                    out[o + 3] = (acc[3] * 0.25 + 0.5) as u8;
                }
                MapKind::Linear => {
                    for c in 0..4 {
                        out[o + c] = (acc[c] * 0.25 + 0.5) as u8;
                    }
                }
            }
        }
    }
    (nw, nh, out)
}

/// Full mip chain, level 0 first (level 0 is the input itself).
pub fn mip_chain(w: u32, h: u32, data: Vec<u8>, kind: MapKind) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = vec![(w, h, data)];
    loop {
        let (lw, lh, d) = levels.last().unwrap();
        if *lw <= 1 && *lh <= 1 {
            break;
        }
        let next = downsample(*lw, *lh, d, kind);
        levels.push(next);
    }
    levels
}

/// Resample to an exact size. Downscaling is done by repeated halving (with
/// the kind-aware filter) and a final bilinear step, so large reductions
/// don't alias; upscaling is bilinear.
pub fn resample(w: u32, h: u32, data: &[u8], tw: u32, th: u32, kind: MapKind) -> Vec<u8> {
    if (w, h) == (tw, th) {
        return data.to_vec();
    }
    let (mut cw, mut ch, mut cur) = (w, h, data.to_vec());
    while cw >= tw * 2 && ch >= th * 2 {
        let (nw, nh, nd) = downsample(cw, ch, &cur, kind);
        cw = nw;
        ch = nh;
        cur = nd;
    }
    if (cw, ch) == (tw, th) {
        return cur;
    }
    let img = image::RgbaImage::from_raw(cw, ch, cur).expect("rgba size");
    image::imageops::resize(&img, tw, th, image::imageops::FilterType::Triangle).into_raw()
}

/// Bytes of an RGBA8 texture of this size with a full mip chain.
pub fn chain_bytes(w: u32, h: u32) -> u64 {
    let (mut w, mut h, mut total) = (w as u64, h as u64, 0u64);
    loop {
        total += w * h * 4;
        if w <= 1 && h <= 1 {
            return total;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_mips_preserve_average_brightness() {
        // Black/white checker: the true average is 50% linear = 188 sRGB,
        // not the 127 a naive byte average gives (visibly too dark).
        let w = 64;
        let data: Vec<u8> = (0..w * w)
            .flat_map(|i| {
                let v = if (i % w + i / w) % 2 == 0 { 255 } else { 0 };
                [v, v, v, 255]
            })
            .collect();
        let (_, _, d) = downsample(w, w, &data, MapKind::Srgb);
        assert!((d[0] as i32 - 188).abs() <= 1, "got {}", d[0]);
        let (_, _, l) = downsample(w, w, &data, MapKind::Linear);
        assert!((l[0] as i32 - 128).abs() <= 1);
    }

    #[test]
    fn normal_mips_stay_unit_length() {
        // Two opposing tilts average to straight up, renormalised.
        let px = |x: f32, y: f32, z: f32| [((x * 0.5 + 0.5) * 255.0) as u8, ((y * 0.5 + 0.5) * 255.0) as u8, ((z * 0.5 + 0.5) * 255.0) as u8, 255];
        let a = px(0.6, 0.0, 0.8);
        let b = px(-0.6, 0.0, 0.8);
        let data: Vec<u8> = [a, b, b, a].concat();
        let (_, _, d) = downsample(2, 2, &data, MapKind::Normal);
        let n = [d[0] as f32 / 127.5 - 1.0, d[1] as f32 / 127.5 - 1.0, d[2] as f32 / 127.5 - 1.0];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!((len - 1.0).abs() < 0.02 && n[2] > 0.98, "{n:?}");
    }

    #[test]
    fn chain_and_resample_sizes() {
        let levels = mip_chain(8, 2, vec![100; 8 * 2 * 4], MapKind::Linear);
        assert_eq!(levels.iter().map(|l| (l.0, l.1)).collect::<Vec<_>>(), vec![(8, 2), (4, 1), (2, 1), (1, 1)]);
        assert_eq!(chain_bytes(4, 4), (16 + 4 + 1) * 4);
        let r = resample(300, 200, &vec![50; 300 * 200 * 4], 512, 512, MapKind::Srgb);
        assert_eq!(r.len(), 512 * 512 * 4);
        let r = resample(4096, 4096, &vec![50; 4096 * 4096 * 4], 1024, 1024, MapKind::Srgb);
        assert_eq!(r.len(), 1024 * 1024 * 4);
        assert!(r.iter().all(|&v| v == 50));
    }
}
