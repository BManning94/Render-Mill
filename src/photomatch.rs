//! Material match from a photo: split a photo of a building into its
//! materials (siding, brick, roof, trim...) and find the closest textures in
//! the material library for each one.
//!
//! This runs entirely on this computer with classic image analysis - no
//! cloud service, no model download:
//! 1. The photo is split into regions by k-means clustering of small
//!    patches on colour (CIE Lab) and texture (edge strength, the direction
//!    of lines such as siding courses, and fine-scale variation).
//! 2. Every library texture gets the same description (at three scales,
//!    because the photo's scale is unknown), cached on disk.
//! 3. Each region is compared with every library texture; the nearest ones
//!    are offered, best first.

use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Texture statistics at one scale.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TexStats {
    /// Mean gradient magnitude of L (edge strength).
    pub energy: f32,
    /// (sum |d/dy| - sum |d/dx|) / sum: +1 = horizontal lines (lap siding,
    /// brick courses), -1 = vertical (board & batten), 0 = no direction.
    pub aniso: f32,
    /// Standard deviation of L.
    pub sd: f32,
}

/// What a surface looks like: average colour and texture.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Desc {
    /// Mean CIE L*a*b*.
    pub lab: [f32; 3],
    /// Library textures: stats at 256, 128 and 64 px per tile; photo
    /// regions: one entry at the analysis resolution.
    pub tex: Vec<TexStats>,
}

fn srgb_to_lin(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn rgb_to_lab(r: u8, g: u8, b: u8) -> [f32; 3] {
    let (r, g, b) = (srgb_to_lin(r), srgb_to_lin(g), srgb_to_lin(b));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f32| if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// L channel and Lab image of an RGBA image.
fn lab_image(img: &RgbaImage) -> Vec<[f32; 3]> {
    img.pixels().map(|p| rgb_to_lab(p[0], p[1], p[2])).collect()
}

/// Texture stats over the pixels where `mask` is true (None = all).
fn tex_stats(lab: &[[f32; 3]], w: usize, h: usize, mask: Option<&dyn Fn(usize, usize) -> bool>) -> TexStats {
    tex_stats_in(lab, w, h, (0, 0, w, h), mask)
}

/// `tex_stats` limited to the rectangle (x0, y0, x1, y1).
fn tex_stats_in(lab: &[[f32; 3]], w: usize, h: usize, r: (usize, usize, usize, usize), mask: Option<&dyn Fn(usize, usize) -> bool>) -> TexStats {
    let (mut gx_sum, mut gy_sum, mut n) = (0.0f64, 0.0f64, 0.0f64);
    let (mut l_sum, mut l2_sum) = (0.0f64, 0.0f64);
    for y in r.1.max(1)..r.3.min(h.saturating_sub(1)) {
        for x in r.0.max(1)..r.2.min(w.saturating_sub(1)) {
            if let Some(m) = mask {
                if !(m(x, y) && m(x + 1, y) && m(x - 1, y) && m(x, y + 1) && m(x, y - 1)) {
                    continue;
                }
            }
            let l = |x: usize, y: usize| lab[y * w + x][0] as f64;
            let gx = (l(x + 1, y) - l(x - 1, y)) * 0.5;
            let gy = (l(x, y + 1) - l(x, y - 1)) * 0.5;
            gx_sum += gx.abs();
            gy_sum += gy.abs();
            let v = l(x, y);
            l_sum += v;
            l2_sum += v * v;
            n += 1.0;
        }
    }
    if n < 1.0 {
        return TexStats::default();
    }
    let mean = l_sum / n;
    TexStats {
        energy: ((gx_sum + gy_sum) / n) as f32,
        aniso: ((gy_sum - gx_sum) / (gx_sum + gy_sum).max(1e-6)) as f32,
        sd: (l2_sum / n - mean * mean).max(0.0).sqrt() as f32,
    }
}

/// Texture stats as the median over small patches that lie inside `mask`:
/// local texture (courses, joints, grain), not large-scale shading.
fn patch_stats(lab: &[[f32; 3]], w: usize, h: usize, patch: usize, mask: &dyn Fn(usize, usize) -> bool) -> TexStats {
    let mut es = Vec::new();
    let mut sds = Vec::new();
    let (mut an_w, mut an_sum) = (0.0f32, 0.0f32);
    for y0 in (0..h.saturating_sub(patch)).step_by(patch) {
        for x0 in (0..w.saturating_sub(patch)).step_by(patch) {
            // Mostly inside the region (corners and centre).
            let pts = [(x0, y0), (x0 + patch - 1, y0), (x0, y0 + patch - 1), (x0 + patch - 1, y0 + patch - 1), (x0 + patch / 2, y0 + patch / 2)];
            if pts.iter().filter(|(x, y)| mask(*x, *y)).count() < 5 {
                continue;
            }
            // Include one pixel around the patch so its edge pixels count too.
            let t = tex_stats_in(lab, w, h, (x0, y0, x0 + patch, y0 + patch), None);
            es.push(t.energy);
            sds.push(t.sd);
            an_sum += t.aniso * t.energy;
            an_w += t.energy;
        }
    }
    if es.is_empty() {
        return tex_stats(lab, w, h, Some(mask));
    }
    // Trimmed mean (middle 80 %): robust to a few windows or downpipes in
    // the region, but still counts sparse edges such as mortar joints.
    let med = |v: &mut Vec<f32>| {
        v.sort_by(|a, b| a.total_cmp(b));
        let (lo, hi) = (v.len() / 10, v.len() - v.len() / 10);
        let s = &v[lo..hi.max(lo + 1)];
        s.iter().sum::<f32>() / s.len() as f32
    };
    TexStats { energy: med(&mut es), aniso: an_sum / an_w.max(1e-6), sd: med(&mut sds) }
}

/// Describe a library texture (its colour map).
pub fn describe_texture(img: &image::DynamicImage) -> Desc {
    let mut tex = Vec::new();
    let mut lab_mean = [0.0f32; 3];
    for (k, size) in [256u32, 128, 64, 32].into_iter().enumerate() {
        let small = img.resize_exact(size, size, image::imageops::FilterType::Triangle).to_rgba8();
        let lab = lab_image(&small);
        if k == 0 {
            for p in &lab {
                for c in 0..3 {
                    lab_mean[c] += p[c] / lab.len() as f32;
                }
            }
        }
        tex.push(patch_stats(&lab, size as usize, size as usize, 16, &|_, _| true));
    }
    Desc { lab: lab_mean, tex }
}

/// Kind of region, from simple cues (used to skip sky and plants).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionKind {
    Surface,
    Sky,
    Plants,
}

#[derive(Clone, Debug)]
pub struct Region {
    pub desc: Desc,
    /// Share of the photo (0..1).
    pub share: f32,
    /// Centre in photo coordinates (0..1).
    pub center: [f32; 2],
    /// Average colour (sRGB) for swatches.
    pub color: [u8; 3],
    pub kind: RegionKind,
}

pub struct Analysis {
    /// Photo at analysis resolution.
    pub image: RgbaImage,
    /// Region index per pixel of `image`.
    pub labels: Vec<u8>,
    pub regions: Vec<Region>,
}

struct Rng(u64);
impl Rng {
    fn f(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as u32) as f32 / 16_777_216.0
    }
}

const PATCH: usize = 8;
/// Longest side of the photo during analysis.
const ANALYSIS_SIZE: u32 = 384;
/// Longest side when measuring texture.
const DETAIL_SIZE: u32 = 1024;

/// Split a photo into material regions.
pub fn analyze(photo: &image::DynamicImage, max_regions: usize) -> Analysis {
    let img = photo.resize(ANALYSIS_SIZE, ANALYSIS_SIZE, image::imageops::FilterType::Triangle).to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let lab = lab_image(&img);
    let (pw, ph) = (w / PATCH, h / PATCH);
    // Patch features: L, a, b, edge energy, line direction, L variation, x, y.
    let mut feats: Vec<[f32; 8]> = Vec::with_capacity(pw * ph);
    for py in 0..ph {
        for px in 0..pw {
            let (x0, y0) = (px * PATCH, py * PATCH);
            let t = tex_stats_in(&lab, w, h, (x0, y0, x0 + PATCH, y0 + PATCH), None);
            let mut m = [0.0f32; 3];
            for y in y0..y0 + PATCH {
                for x in x0..x0 + PATCH {
                    for c in 0..3 {
                        m[c] += lab[y * w + x][c] / (PATCH * PATCH) as f32;
                    }
                }
            }
            feats.push([
                m[0] / 10.0,
                m[1] / 4.0,
                m[2] / 4.0,
                t.energy / 3.0,
                t.aniso / 0.35 * (t.energy / (t.energy + 1.5)),
                t.sd / 6.0,
                px as f32 / pw as f32 * 2.0,
                py as f32 / ph as f32 * 2.0,
            ]);
        }
    }
    let k = max_regions.clamp(2, 16).min(feats.len().max(2));
    let assign = kmeans(&feats, k, 20);
    // Merge tiny clusters into their nearest neighbour.
    let mut counts = vec![0usize; k];
    for &a in &assign {
        counts[a] += 1;
    }
    let mut centroids = centroids_of(&feats, &assign, k);
    let min_patches = (feats.len() as f32 * 0.015).ceil() as usize;
    let alive: Vec<usize> = (0..k).filter(|&c| counts[c] >= min_patches.max(1)).collect();
    let remap: Vec<usize> = (0..k)
        .map(|c| {
            if alive.contains(&c) {
                c
            } else {
                *alive.iter().min_by(|&&a, &&b| dist2(&centroids[c], &centroids[a]).total_cmp(&dist2(&centroids[c], &centroids[b]))).unwrap_or(&c)
            }
        })
        .collect();
    let assign: Vec<usize> = assign.iter().map(|&a| remap[a]).collect();
    centroids = centroids_of(&feats, &assign, k);
    let order: Vec<usize> = alive.clone();
    let index_of = |c: usize| order.iter().position(|&o| o == c).unwrap_or(0);
    // Per-pixel labels: each pixel takes the nearest centroid among its own
    // patch's region and its neighbours' (cleaner borders than patch blocks).
    let mut labels = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let (px, py) = ((x / PATCH).min(pw.saturating_sub(1)), (y / PATCH).min(ph.saturating_sub(1)));
            let p = lab[y * w + x];
            let mut best = (f32::MAX, assign.get(py * pw + px).copied().unwrap_or(0));
            for (dx, dy) in [(0i32, 0i32), (-1, 0), (1, 0), (0, -1), (0, 1)] {
                let (nx, ny) = (px as i32 + dx, py as i32 + dy);
                if nx < 0 || ny < 0 || nx >= pw as i32 || ny >= ph as i32 {
                    continue;
                }
                let c = assign[ny as usize * pw + nx as usize];
                let f = &centroids[c];
                let own = &feats[py * pw + px];
                let d = (p[0] / 10.0 - f[0]).powi(2) + (p[1] / 4.0 - f[1]).powi(2) + (p[2] / 4.0 - f[2]).powi(2) + (own[3] - f[3]).powi(2) * 0.5;
                if d < best.0 {
                    best = (d, c);
                }
            }
            labels[y * w + x] = index_of(best.1) as u8;
        }
    }
    // Texture is measured at a higher resolution (siding courses and brick
    // joints are only a pixel or two apart at the analysis size).
    let detail = photo.resize(DETAIL_SIZE, DETAIL_SIZE, image::imageops::FilterType::Triangle).to_rgba8();
    let (dw, dh) = (detail.width() as usize, detail.height() as usize);
    let dlab = lab_image(&detail);
    // Describe each region from its pixels.
    let mut regions = Vec::new();
    for (ri, _) in order.iter().enumerate() {
        let mask = |x: usize, y: usize| labels[y * w + x] as usize == ri;
        let mut n = 0.0f32;
        let (mut lm, mut cx, mut cy, mut rgb) = ([0.0f32; 3], 0.0f32, 0.0f32, [0.0f32; 3]);
        for y in 0..h {
            for x in 0..w {
                if mask(x, y) {
                    let p = lab[y * w + x];
                    for c in 0..3 {
                        lm[c] += p[c];
                    }
                    let px = img.get_pixel(x as u32, y as u32);
                    for c in 0..3 {
                        rgb[c] += px[c] as f32;
                    }
                    cx += x as f32;
                    cy += y as f32;
                    n += 1.0;
                }
            }
        }
        let n1 = n.max(1.0);
        let lab_mean = [lm[0] / n1, lm[1] / n1, lm[2] / n1];
        let dmask = |x: usize, y: usize| labels[((y * h / dh).min(h - 1)) * w + (x * w / dw).min(w - 1)] as usize == ri;
        let tex = patch_stats(&dlab, dw, dh, 16, &dmask);
        let center = [cx / n1 / w as f32, cy / n1 / h as f32];
        // Sky: bright, flat, upper part. Plants: clearly green.
        let kind = if center[1] < 0.45 && lab_mean[0] > 55.0 && tex.energy < 2.2 && lab_mean[2] < 6.0 {
            RegionKind::Sky
        } else if lab_mean[1] < -5.0 && lab_mean[2] > 4.0 && tex.energy > 1.5 {
            RegionKind::Plants
        } else {
            RegionKind::Surface
        };
        regions.push(Region {
            desc: Desc { lab: lab_mean, tex: vec![tex] },
            share: n / (w * h) as f32,
            center,
            color: [(rgb[0] / n1) as u8, (rgb[1] / n1) as u8, (rgb[2] / n1) as u8],
            kind,
        });
    }
    Analysis { image: img, labels, regions }
}

fn dist2(a: &[f32; 8], b: &[f32; 8]) -> f32 {
    // Position counts a little, so regions stay mostly connected.
    let mut d = 0.0;
    for i in 0..8 {
        let w = if i >= 6 { 0.35 } else { 1.0 };
        d += (a[i] - b[i]).powi(2) * w;
    }
    d
}

fn centroids_of(feats: &[[f32; 8]], assign: &[usize], k: usize) -> Vec<[f32; 8]> {
    let mut c = vec![[0.0f32; 8]; k];
    let mut n = vec![0.0f32; k];
    for (f, &a) in feats.iter().zip(assign) {
        for i in 0..8 {
            c[a][i] += f[i];
        }
        n[a] += 1.0;
    }
    for (ci, ni) in c.iter_mut().zip(&n) {
        if *ni > 0.0 {
            for v in ci.iter_mut() {
                *v /= ni;
            }
        }
    }
    c
}

/// k-means with k-means++ seeding (deterministic).
fn kmeans(feats: &[[f32; 8]], k: usize, iters: usize) -> Vec<usize> {
    let mut rng = Rng(0x5eed);
    let mut cents: Vec<[f32; 8]> = vec![feats[(rng.f() * feats.len() as f32) as usize % feats.len()]];
    while cents.len() < k {
        let d: Vec<f32> = feats.iter().map(|f| cents.iter().map(|c| dist2(f, c)).fold(f32::MAX, f32::min)).collect();
        let total: f32 = d.iter().sum();
        let mut t = rng.f() * total;
        let mut pick = feats.len() - 1;
        for (i, v) in d.iter().enumerate() {
            t -= v;
            if t <= 0.0 {
                pick = i;
                break;
            }
        }
        cents.push(feats[pick]);
    }
    let mut assign = vec![0usize; feats.len()];
    for _ in 0..iters {
        let mut changed = false;
        for (i, f) in feats.iter().enumerate() {
            let best = (0..k).min_by(|&a, &b| dist2(f, &cents[a]).total_cmp(&dist2(f, &cents[b]))).unwrap();
            if assign[i] != best {
                assign[i] = best;
                changed = true;
            }
        }
        cents = centroids_of(feats, &assign, k);
        if !changed {
            break;
        }
    }
    assign
}

/// How different a photo region is from a library texture (lower = closer).
/// Lightness counts less than hue/chroma: the photo's lighting changes it.
pub fn distance(region: &Desc, lib: &Desc) -> f32 {
    let (r, l) = (&region.lab, &lib.lab);
    let chroma = ((r[1] - l[1]).powi(2) + (r[2] - l[2]).powi(2)) / 36.0;
    let light = (r[0] - l[0]).powi(2) / 400.0;
    let rt = region.tex.first().copied().unwrap_or_default();
    // The photo's scale is unknown: take the best of the library's scales.
    let tex = lib
        .tex
        .iter()
        .map(|t| {
            let e = (rt.energy.ln_1p() - t.energy.ln_1p()).powi(2) / 0.12;
            // Direction only matters where there are edges to have one.
            let strength = (rt.energy.min(t.energy) / 2.0).min(1.0);
            let a = (rt.aniso - t.aniso).powi(2) / 0.04 * strength;
            let s = (rt.sd.ln_1p() - t.sd.ln_1p()).powi(2) / 0.25;
            e + a + s
        })
        .fold(f32::MAX, f32::min);
    chroma + light + tex * 0.8
}

/// The `n` closest library entries: (index into `lib`, similarity 0..1).
pub fn best_matches(region: &Desc, lib: &[Option<Desc>], n: usize) -> Vec<(usize, f32)> {
    let mut v: Vec<(usize, f32)> = lib.iter().enumerate().filter_map(|(i, d)| d.as_ref().map(|d| (i, distance(region, d)))).collect();
    v.sort_by(|a, b| a.1.total_cmp(&b.1));
    v.truncate(n);
    v.into_iter().map(|(i, d)| (i, (-d / 6.0).exp())).collect()
}

/// On-disk cache of library descriptors, keyed by file path, size and time.
#[derive(Default, Serialize, Deserialize)]
pub struct DescCache {
    pub entries: HashMap<String, Desc>,
}

fn cache_key(path: &Path) -> Option<String> {
    let m = std::fs::metadata(path).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(format!("{}|{}|{}", path.display(), m.len(), t))
}

impl DescCache {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
    pub fn save(&self, path: &Path) {
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(t) = serde_json::to_string(self) {
            let _ = std::fs::write(path, t);
        }
    }
}

/// Describe every colour map (in parallel), using and updating the cache.
/// Returns one entry per input path (None when it can't be read).
pub fn describe_library(paths: &[PathBuf], cache: &mut DescCache, progress: &(dyn Fn(usize, usize) + Sync)) -> Vec<Option<Desc>> {
    let keys: Vec<Option<String>> = paths.iter().map(|p| cache_key(p)).collect();
    let mut out: Vec<Option<Desc>> = keys.iter().map(|k| k.as_ref().and_then(|k| cache.entries.get(k).cloned())).collect();
    let todo: Vec<usize> = (0..paths.len()).filter(|&i| out[i].is_none() && keys[i].is_some()).collect();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
    let done = std::sync::atomic::AtomicUsize::new(0);
    let results: Vec<(usize, Option<Desc>)> = std::thread::scope(|s| {
        let chunks: Vec<Vec<usize>> = (0..threads).map(|t| todo.iter().copied().skip(t).step_by(threads).collect()).collect();
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                let done = &done;
                let total = todo.len();
                s.spawn(move || {
                    chunk
                        .into_iter()
                        .map(|i| {
                            let d = image::open(&paths[i]).ok().map(|img| describe_texture(&img));
                            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                            progress(n, total);
                            (i, d)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    for (i, d) in results {
        if let (Some(d), Some(k)) = (&d, &keys[i]) {
            cache.entries.insert(k.clone(), d.clone());
        }
        out[i] = d;
    }
    out
}

/// Model materials whose names suggest this library category (Revit names
/// like "Siding - Lap", "Roof Shingles", "Brick, Common").
pub fn suggest_target<'a>(category: &str, model_materials: &'a [String]) -> Option<&'a String> {
    let cat = category.to_lowercase();
    model_materials.iter().find(|m| crate::library::guess_category(m).to_lowercase() == cat)
}

/// Region colours for overlays.
pub const PALETTE: [[u8; 3]; 16] = [
    [230, 25, 75],
    [60, 180, 75],
    [255, 225, 25],
    [0, 130, 200],
    [245, 130, 48],
    [145, 30, 180],
    [70, 240, 240],
    [240, 50, 230],
    [210, 245, 60],
    [250, 190, 212],
    [0, 128, 128],
    [220, 190, 255],
    [170, 110, 40],
    [255, 250, 200],
    [128, 0, 0],
    [170, 255, 195],
];

/// The photo with each region tinted in its palette colour.
pub fn overlay(a: &Analysis, highlight: Option<usize>, strength: f32) -> RgbaImage {
    let mut out = a.image.clone();
    for (i, p) in out.pixels_mut().enumerate() {
        let r = a.labels[i] as usize;
        let c = PALETTE[r % PALETTE.len()];
        let k = match highlight {
            Some(h) if h == r => strength * 1.6,
            Some(_) => strength * 0.25,
            None => strength,
        }
        .min(0.9);
        for ch in 0..3 {
            p[ch] = (p[ch] as f32 * (1.0 - k) + c[ch] as f32 * k) as u8;
        }
    }
    out
}

/// Command-line check: `--match-photo <photo> <folder> [overlay.png]`.
pub fn cli(photo: &Path, dir: &Path, overlay_out: Option<&Path>) -> anyhow::Result<()> {
    let img = image::open(photo)?;
    let lib = crate::library::scan_user_folder(dir);
    let paths: Vec<PathBuf> = lib.iter().filter_map(|m| m.set.color.as_ref().map(PathBuf::from)).collect();
    let names: Vec<(String, String)> = lib.iter().filter(|m| m.set.color.is_some()).map(|m| (m.name.clone(), m.category.clone())).collect();
    let mut cache = DescCache::default();
    let t0 = std::time::Instant::now();
    let descs = describe_library(&paths, &mut cache, &|_, _| {});
    let t1 = std::time::Instant::now();
    let a = analyze(&img, 8);
    println!("library: {} textures described in {:.2}s; photo analysed in {:.2}s", paths.len(), (t1 - t0).as_secs_f32(), t1.elapsed().as_secs_f32());
    for (i, r) in a.regions.iter().enumerate() {
        let m = best_matches(&r.desc, &descs, 3);
        let ms: Vec<String> = m.iter().map(|(j, s)| format!("{} [{}] {:.0}%", names[*j].0, names[*j].1, s * 100.0)).collect();
        println!(
            "region {i} {:?} {:.0}% at ({:.2},{:.2}) lab ({:.0},{:.0},{:.0}) tex {:?}: {}",
            r.kind,
            r.share * 100.0,
            r.center[0],
            r.center[1],
            r.desc.lab[0],
            r.desc.lab[1],
            r.desc.lab[2],
            r.desc.tex[0],
            ms.join(", ")
        );
    }
    if let Some(o) = overlay_out {
        overlay(&a, None, 0.45).save(o)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stripes(horizontal: bool, a: [u8; 3], b: [u8; 3]) -> image::DynamicImage {
        let img = RgbaImage::from_fn(256, 256, |x, y| {
            let v = if horizontal { y } else { x };
            let c = if (v / 12) % 2 == 0 { a } else { b };
            image::Rgba([c[0], c[1], c[2], 255])
        });
        image::DynamicImage::ImageRgba8(img)
    }

    #[test]
    fn line_direction_is_measured() {
        let h = describe_texture(&stripes(true, [200, 200, 200], [120, 120, 120]));
        let v = describe_texture(&stripes(false, [200, 200, 200], [120, 120, 120]));
        assert!(h.tex[0].aniso > 0.8, "{:?}", h.tex[0]);
        assert!(v.tex[0].aniso < -0.8, "{:?}", v.tex[0]);
    }

    #[test]
    fn photo_regions_match_their_own_textures() {
        // A "photo": left half red brick-like stripes, right half grey siding.
        let brick = stripes(true, [150, 60, 45], [110, 40, 30]).to_rgba8();
        let siding = stripes(true, [170, 172, 175], [140, 142, 146]).to_rgba8();
        let roof = image::DynamicImage::ImageRgba8(RgbaImage::from_fn(256, 256, |x, y| {
            let v = ((x * 7 + y * 13) % 23) as u8 * 3 + 40;
            image::Rgba([v, v, v + 4, 255])
        }));
        let photo = RgbaImage::from_fn(512, 384, |x, y| {
            if y < 120 {
                *roof.to_rgba8().get_pixel(x % 256, y % 256)
            } else if x < 256 {
                *brick.get_pixel(x % 256, y % 256)
            } else {
                *siding.get_pixel(x % 256, y % 256)
            }
        });
        let a = analyze(&image::DynamicImage::ImageRgba8(photo), 6);
        let lib = vec![
            Some(describe_texture(&image::DynamicImage::ImageRgba8(brick))),
            Some(describe_texture(&image::DynamicImage::ImageRgba8(siding))),
            Some(describe_texture(&roof)),
        ];
        let mut found = [false; 3];
        for r in a.regions.iter().filter(|r| r.share > 0.1) {
            let m = best_matches(&r.desc, &lib, 1);
            let want = if r.center[1] < 0.3 {
                2
            } else if r.center[0] < 0.5 {
                0
            } else {
                1
            };
            assert_eq!(m[0].0, want, "region at {:?} ({:?}) matched {:?}", r.center, r.desc, m);
            found[want] = true;
        }
        assert!(found.iter().all(|f| *f), "{found:?}");
    }

    #[test]
    fn suggests_model_materials_by_name() {
        let mats = vec!["Glass".to_string(), "Siding - Lap 6in".to_string(), "Roof Shingles".to_string()];
        assert_eq!(suggest_target("Siding", &mats).map(String::as_str), Some("Siding - Lap 6in"));
        assert_eq!(suggest_target("Roofing", &mats).map(String::as_str), Some("Roof Shingles"));
        assert!(suggest_target("Brick", &mats).is_none());
    }
}
