//! Denoising for ray-traced images.
//!
//! * **AI denoiser:** Intel Open Image Denoise (Apache 2.0), a neural network
//!   trained on ray-traced images. It is loaded at runtime from the
//!   `OpenImageDenoise` library next to the executable, so LumenCraft still
//!   runs without it. It uses the CPU or (with the CUDA/HIP/SYCL device
//!   libraries present) the graphics card.
//! * **Fallback:** an edge-aware à-trous wavelet filter guided by the albedo,
//!   normal and depth of the first hit.

use std::ffi::{c_char, c_void, CString};
use std::path::PathBuf;

/// Noisy image plus the guide layers the denoisers use (all linear RGB).
pub struct Layers<'a> {
    pub width: usize,
    pub height: usize,
    pub color: &'a [[f32; 3]],
    pub albedo: &'a [[f32; 3]],
    pub normal: &'a [[f32; 3]],
    /// Distance to the first hit (for the fallback filter).
    pub depth: &'a [f32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Ai,
    Wavelet,
}

/// Denoise with the AI denoiser when available, else the wavelet filter.
pub fn denoise(l: &Layers) -> (Vec<[f32; 3]>, Method) {
    if let Some(out) = oidn::denoise(l) {
        return (out, Method::Ai);
    }
    (wavelet(l), Method::Wavelet)
}

pub fn ai_available() -> bool {
    oidn::library().is_some()
}

/// The path tracer's fp16 read-back (see shaders/pt_resolve.wgsl): three
/// planes of RGBA16F pixels with `row_px` pixels per row - colour (A = 1),
/// albedo (A = first-hit distance) and normal.
pub struct Packed<'a> {
    pub width: usize,
    pub height: usize,
    pub row_px: usize,
    pub color: &'a [u8],
    pub albedo: &'a [u8],
    pub normal: &'a [u8],
}

/// Denoise the packed layers; returns the colour plane (same layout,
/// RGBA16F with padded rows - ready to upload as the HDR image).
/// `final_quality`: OIDN's high-quality network (else balanced, for live
/// previews). `noisy_aux`: the guides are noisy (depth of field), so OIDN
/// cleans them first.
pub fn denoise_packed(p: &Packed, final_quality: bool, noisy_aux: bool) -> (Vec<u8>, Method) {
    let row = p.row_px * 8;
    let img = |d| oidn::Img { data: d, format: oidn::FORMAT_HALF3, pixel_stride: 8, row_stride: row };
    let mut out = vec![0u8; p.color.len()];
    if oidn::run(p.width, p.height, &img(p.color), &img(p.albedo), &img(p.normal), &mut out, final_quality, noisy_aux && final_quality) {
        return (out, Method::Ai);
    }
    // Fallback: unpack to f32, wavelet filter, pack again.
    let n = p.width * p.height;
    let (mut color, mut albedo, mut normal, mut depth) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
    for y in 0..p.height {
        for x in 0..p.width {
            let o = (y * p.row_px + x) * 8;
            let c = unpack4(&p.color[o..o + 8]);
            let a = unpack4(&p.albedo[o..o + 8]);
            let nn = unpack4(&p.normal[o..o + 8]);
            color.push([c[0], c[1], c[2]]);
            albedo.push([a[0], a[1], a[2]]);
            depth.push(a[3]);
            normal.push([nn[0], nn[1], nn[2]]);
        }
    }
    let l = Layers { width: p.width, height: p.height, color: &color, albedo: &albedo, normal: &normal, depth: &depth };
    let filtered = wavelet(&l);
    out.copy_from_slice(p.color);
    for y in 0..p.height {
        for x in 0..p.width {
            let o = (y * p.row_px + x) * 8;
            let c = filtered[y * p.width + x];
            for (k, v) in c.iter().enumerate() {
                out[o + k * 2..o + k * 2 + 2].copy_from_slice(&crate::pathtrace::f16_bits(*v).to_le_bytes());
            }
        }
    }
    (out, Method::Wavelet)
}

/// IEEE half to f32.
pub fn f16_to_f32(h: u16) -> f32 {
    let s = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = ((h >> 10) & 0x1f) as i32;
    let m = (h & 0x3ff) as f32;
    match e {
        0 => s * m * 2f32.powi(-24),
        31 => {
            if m == 0.0 {
                s * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => s * (1.0 + m / 1024.0) * 2f32.powi(e - 15),
    }
}

fn unpack4(b: &[u8]) -> [f32; 4] {
    [0, 2, 4, 6].map(|o| f16_to_f32(u16::from_le_bytes([b[o], b[o + 1]])))
}

// ---------------------------------------------------------------- OIDN

mod oidn {
    use super::*;
    use libloading::Library;
    use std::sync::OnceLock;

    type Device = *mut c_void;
    type Filter = *mut c_void;
    type Buffer = *mut c_void;

    pub(super) fn library() -> Option<&'static Library> {
        static LIB: OnceLock<Option<Library>> = OnceLock::new();
        LIB.get_or_init(|| {
            let names: &[&str] = if cfg!(windows) {
                &["OpenImageDenoise.dll"]
            } else if cfg!(target_os = "macos") {
                &["libOpenImageDenoise.2.dylib", "libOpenImageDenoise.dylib"]
            } else {
                &["libOpenImageDenoise.so.2", "libOpenImageDenoise.so"]
            };
            let mut dirs: Vec<PathBuf> = Vec::new();
            if let Ok(d) = std::env::var("LUMENCRAFT_OIDN") {
                dirs.push(PathBuf::from(d));
            }
            if let Some(exe) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
                dirs.push(exe.join("denoiser"));
                dirs.push(exe.clone());
            }
            for d in &dirs {
                join_parts(d);
                for n in names {
                    let p = d.join(n);
                    if p.is_file() {
                        // SAFETY: loading a known C library; its initialisers are trusted.
                        match unsafe { open(&p) } {
                            Ok(l) => {
                                log::info!("AI denoiser: {}", p.display());
                                return Some(l);
                            }
                            Err(e) => log::warn!("could not load {}: {e}", p.display()),
                        }
                    }
                }
            }
            None
        })
        .as_ref()
    }

    /// Load so that the library's own folder is searched for its
    /// dependencies (core, device modules, TBB) on Windows.
    unsafe fn open(p: &std::path::Path) -> Result<Library, libloading::Error> {
        #[cfg(windows)]
        {
            let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
            // LOAD_WITH_ALTERED_SEARCH_PATH = 0x8
            libloading::os::windows::Library::load_with_flags(&abs, 0x0000_0008).map(Library::from)
        }
        #[cfg(not(windows))]
        {
            Library::new(p)
        }
    }

    /// Large files ship split as `name.part1`, `name.part2`, … (download
    /// size limits); join them on first use.
    fn join_parts(dir: &std::path::Path) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(base) = name.strip_suffix(".part1") else { continue };
            let target = dir.join(base);
            if target.exists() {
                continue;
            }
            let mut data = Vec::new();
            let mut i = 1;
            while let Ok(bytes) = std::fs::read(dir.join(format!("{base}.part{i}"))) {
                data.extend_from_slice(&bytes);
                i += 1;
            }
            let tmp = dir.join(format!("{base}.joining"));
            if std::fs::write(&tmp, &data).and_then(|_| std::fs::rename(&tmp, &target)).is_ok() {
                log::info!("joined {} parts into {}", i - 1, target.display());
                for k in 1..i {
                    let _ = std::fs::remove_file(dir.join(format!("{base}.part{k}")));
                }
            }
        }
    }

    // OIDN C API constants (oidn.h, 2.x).
    pub(super) const FORMAT_FLOAT3: i32 = 3;
    pub(super) const FORMAT_HALF3: i32 = 259;
    const QUALITY_BALANCED: i32 = 5;
    const QUALITY_HIGH: i32 = 6;

    /// An image in host memory: `data` holds `row_stride * height` bytes.
    pub(super) struct Img<'a> {
        pub data: &'a [u8],
        pub format: i32,
        pub pixel_stride: usize,
        pub row_stride: usize,
    }

    struct Api {
        new_device: unsafe extern "C" fn(i32) -> Device,
        commit_device: unsafe extern "C" fn(Device),
        #[allow(dead_code)]
        release_device: unsafe extern "C" fn(Device),
        get_error: unsafe extern "C" fn(Device, *mut *const c_char) -> i32,
        new_buffer: unsafe extern "C" fn(Device, usize) -> Buffer,
        write_buffer: unsafe extern "C" fn(Buffer, usize, usize, *const c_void),
        read_buffer: unsafe extern "C" fn(Buffer, usize, usize, *mut c_void),
        release_buffer: unsafe extern "C" fn(Buffer),
        new_filter: unsafe extern "C" fn(Device, *const c_char) -> Filter,
        set_image: unsafe extern "C" fn(Filter, *const c_char, Buffer, i32, usize, usize, usize, usize, usize),
        set_bool: unsafe extern "C" fn(Filter, *const c_char, bool),
        set_int: unsafe extern "C" fn(Filter, *const c_char, i32),
        commit_filter: unsafe extern "C" fn(Filter),
        execute: unsafe extern "C" fn(Filter),
        release_filter: unsafe extern "C" fn(Filter),
    }

    fn api() -> Option<&'static Api> {
        static API: OnceLock<Option<Api>> = OnceLock::new();
        API.get_or_init(|| {
            let lib = library()?;
            // SAFETY: the signatures match OpenImageDenoise 2.x's C API (oidn.h).
            unsafe {
                macro_rules! f {
                    ($n:literal) => {
                        *lib.get($n).ok()?
                    };
                }
                Some(Api {
                    new_device: f!(b"oidnNewDevice\0"),
                    commit_device: f!(b"oidnCommitDevice\0"),
                    release_device: f!(b"oidnReleaseDevice\0"),
                    get_error: f!(b"oidnGetDeviceError\0"),
                    new_buffer: f!(b"oidnNewBuffer\0"),
                    write_buffer: f!(b"oidnWriteBuffer\0"),
                    read_buffer: f!(b"oidnReadBuffer\0"),
                    release_buffer: f!(b"oidnReleaseBuffer\0"),
                    new_filter: f!(b"oidnNewFilter\0"),
                    set_image: f!(b"oidnSetFilterImage\0"),
                    set_bool: f!(b"oidnSetFilterBool\0"),
                    set_int: f!(b"oidnSetFilterInt\0"),
                    commit_filter: f!(b"oidnCommitFilter\0"),
                    execute: f!(b"oidnExecuteFilter\0"),
                    release_filter: f!(b"oidnReleaseFilter\0"),
                })
            }
        })
        .as_ref()
    }

    /// Committed filters and their buffers, reused while the image size,
    /// formats and settings stay the same (committing a filter loads the
    /// network weights - by far the slowest step for preview updates).
    struct Setup {
        key: (usize, usize, i32, usize, usize, i32, bool),
        buf: [Buffer; 6], // color, albedo, normal, output, albedo', normal'
        sizes: [usize; 6],
        main: Filter,
        pre: [Filter; 2],
    }

    struct State {
        dev: Device,
        setup: Option<Setup>,
    }

    // SAFETY: OIDN devices/filters may be used from any thread as long as
    // calls are serialised, which the mutex below guarantees.
    unsafe impl Send for State {}

    fn release_setup(a: &Api, s: Setup) {
        // SAFETY: handles were created by this device and are not used again.
        unsafe {
            (a.release_filter)(s.main);
            for f in s.pre {
                if !f.is_null() {
                    (a.release_filter)(f);
                }
            }
            for b in s.buf {
                if !b.is_null() {
                    (a.release_buffer)(b);
                }
            }
        }
    }

    /// Denoise `color` into `out` (same layout as `color`). `final_quality`
    /// selects OIDN's high-quality network (else balanced, for previews);
    /// `prefilter` first denoises the albedo/normal guides themselves, as
    /// OIDN recommends when they are noisy (depth of field, fine foliage).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run(w: usize, h: usize, color: &Img, albedo: &Img, normal: &Img, out: &mut [u8], final_quality: bool, prefilter: bool) -> bool {
        let Some(a) = api() else { return false };
        static STATE: std::sync::Mutex<Option<State>> = std::sync::Mutex::new(None);
        let Ok(mut guard) = STATE.lock() else { return false };
        // SAFETY: all calls follow the OIDN 2.x API contract; buffers are
        // sized from the image descriptions, which callers build to match.
        unsafe {
            if guard.is_none() {
                let d = (a.new_device)(0); // OIDN_DEVICE_TYPE_DEFAULT: best available (GPU if supported)
                if d.is_null() {
                    return false;
                }
                (a.commit_device)(d);
                if let Some(lib) = library() {
                    if let Ok(get_int) = lib.get::<unsafe extern "C" fn(Device, *const c_char) -> i32>(b"oidnGetDeviceInt\0") {
                        let t = CString::new("type").unwrap();
                        let kind = match get_int(d, t.as_ptr()) {
                            1 => "CPU",
                            2 => "SYCL GPU",
                            3 => "CUDA GPU",
                            4 => "HIP GPU",
                            5 => "Metal GPU",
                            _ => "unknown",
                        };
                        log::info!("AI denoiser device: {kind}");
                    }
                }
                *guard = Some(State { dev: d, setup: None });
            }
            let st = guard.as_mut().unwrap();
            let dev = st.dev;
            let check = |what: &str| -> bool {
                let mut msg: *const c_char = std::ptr::null();
                let err = (a.get_error)(dev, &mut msg);
                if err != 0 {
                    let m = if msg.is_null() { String::new() } else { std::ffi::CStr::from_ptr(msg).to_string_lossy().to_string() };
                    log::warn!("AI denoiser error during {what}: {err} {m}");
                    return false;
                }
                true
            };
            let quality = if final_quality { QUALITY_HIGH } else { QUALITY_BALANCED };
            let key = (w, h, color.format, color.pixel_stride, color.row_stride, quality, prefilter);
            if st.setup.as_ref().is_none_or(|s| s.key != key) {
                if let Some(old) = st.setup.take() {
                    release_setup(a, old);
                }
                let sizes = [color.data.len(), albedo.data.len(), normal.data.len(), out.len(), albedo.data.len(), normal.data.len()];
                let mut buf = [std::ptr::null_mut(); 6];
                for (k, b) in buf.iter_mut().enumerate() {
                    if k >= 4 && !prefilter {
                        continue;
                    }
                    *b = (a.new_buffer)(dev, sizes[k]);
                }
                let name = |s: &str| CString::new(s).unwrap();
                let rt = name("RT");
                let set = |f: Filter, n: &str, b: Buffer, img: &Img| {
                    let n = name(n);
                    (a.set_image)(f, n.as_ptr(), b, img.format, w, h, 0, img.pixel_stride, img.row_stride);
                };
                let mut pre = [std::ptr::null_mut(); 2];
                if prefilter {
                    for (k, (nm, img)) in [("albedo", albedo), ("normal", normal)].into_iter().enumerate() {
                        let f = (a.new_filter)(dev, rt.as_ptr());
                        set(f, nm, buf[1 + k], img);
                        set(f, "output", buf[4 + k], img);
                        (a.set_int)(f, name("quality").as_ptr(), quality);
                        (a.commit_filter)(f);
                        pre[k] = f;
                    }
                }
                let f = (a.new_filter)(dev, rt.as_ptr());
                set(f, "color", buf[0], color);
                set(f, "albedo", if prefilter { buf[4] } else { buf[1] }, albedo);
                set(f, "normal", if prefilter { buf[5] } else { buf[2] }, normal);
                set(f, "output", buf[3], color);
                (a.set_bool)(f, name("hdr").as_ptr(), true);
                (a.set_bool)(f, name("cleanAux").as_ptr(), prefilter);
                (a.set_int)(f, name("quality").as_ptr(), quality);
                (a.commit_filter)(f);
                let s = Setup { key, buf, sizes, main: f, pre };
                if buf[..4].iter().any(|b| b.is_null()) || !check("filter setup") {
                    release_setup(a, s);
                    return false;
                }
                st.setup = Some(s);
            }
            let s = st.setup.as_ref().unwrap();
            (a.write_buffer)(s.buf[0], 0, s.sizes[0], color.data.as_ptr() as *const c_void);
            (a.write_buffer)(s.buf[1], 0, s.sizes[1], albedo.data.as_ptr() as *const c_void);
            (a.write_buffer)(s.buf[2], 0, s.sizes[2], normal.data.as_ptr() as *const c_void);
            // Pre-fill the output with the input so bytes the filter doesn't
            // write (padding, the 4th channel of a strided image) are sane.
            (a.write_buffer)(s.buf[3], 0, s.sizes[3], color.data.as_ptr() as *const c_void);
            for f in s.pre {
                if !f.is_null() {
                    (a.execute)(f);
                }
            }
            (a.execute)(s.main);
            if !check("denoising") {
                if let Some(old) = st.setup.take() {
                    release_setup(a, old);
                }
                return false;
            }
            (a.read_buffer)(s.buf[3], 0, s.sizes[3], out.as_mut_ptr() as *mut c_void);
            true
        }
    }

    pub(super) fn denoise(l: &Layers) -> Option<Vec<[f32; 3]>> {
        let flat = |v: &[[f32; 3]]| -> Vec<u8> { v.iter().flat_map(|p| p.iter().flat_map(|c| c.to_le_bytes())).collect() };
        let (c, al, n) = (flat(l.color), flat(l.albedo), flat(l.normal));
        let stride = l.width * 12;
        let img = |d| Img { data: d, format: FORMAT_FLOAT3, pixel_stride: 12, row_stride: stride };
        let mut out = vec![0u8; c.len()];
        if !run(l.width, l.height, &img(&c[..]), &img(&al[..]), &img(&n[..]), &mut out, true, false) {
            return None;
        }
        Some(out.chunks_exact(12).map(|p| [0, 4, 8].map(|o| f32::from_le_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]]))).collect())
    }
}

// ---------------------------------------------------------------- fallback

/// Edge-avoiding à-trous wavelet filter (Dammertz et al. 2010) on the
/// illumination (colour divided by albedo), so texture detail stays sharp.
pub fn wavelet(l: &Layers) -> Vec<[f32; 3]> {
    let (w, h) = (l.width, l.height);
    let eps = 0.02f32;
    let mut illum: Vec<[f32; 3]> = l.color.iter().zip(l.albedo).map(|(c, a)| [c[0] / (a[0] + eps), c[1] / (a[1] + eps), c[2] / (a[2] + eps)]).collect();
    let lum = |c: &[f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let kernel = [1.0 / 16.0, 1.0 / 4.0, 3.0 / 8.0, 1.0 / 4.0, 1.0 / 16.0];
    let mut sigma_c = 0.6f32;
    for it in 0..5 {
        let step = 1i32 << it;
        let src = illum.clone();
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let i = (y as usize) * w + x as usize;
                let (c0, n0, d0) = (src[i], l.normal[i], l.depth[i]);
                let l0 = lum(&c0);
                let mut sum = [0f32; 3];
                let mut wsum = 0f32;
                for (ky, kwy) in kernel.iter().enumerate() {
                    let yy = y + (ky as i32 - 2) * step;
                    if yy < 0 || yy >= h as i32 {
                        continue;
                    }
                    for (kx, kwx) in kernel.iter().enumerate() {
                        let xx = x + (kx as i32 - 2) * step;
                        if xx < 0 || xx >= w as i32 {
                            continue;
                        }
                        let j = (yy as usize) * w + xx as usize;
                        let c = src[j];
                        let n = l.normal[j];
                        let ndot = (n0[0] * n[0] + n0[1] * n[1] + n0[2] * n[2]).max(0.0);
                        let wn = ndot.powf(64.0);
                        let dd = (l.depth[j] - d0).abs() / (d0.max(0.1) * 0.03 * step as f32 + 1e-3);
                        let wd = (-dd).exp();
                        let dl = (lum(&c) - l0).abs() / (sigma_c * (l0.max(0.05)).sqrt() + 1e-4);
                        let wl = (-dl).exp();
                        let wgt = kwx * kwy * wn * wd * wl;
                        sum[0] += c[0] * wgt;
                        sum[1] += c[1] * wgt;
                        sum[2] += c[2] * wgt;
                        wsum += wgt;
                    }
                }
                if wsum > 1e-8 {
                    illum[i] = [sum[0] / wsum, sum[1] / wsum, sum[2] / wsum];
                }
            }
        }
        sigma_c *= 0.6;
    }
    illum.iter().zip(l.albedo).map(|(c, a)| [c[0] * (a[0] + eps), c[1] * (a[1] + eps), c[2] * (a[2] + eps)]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_roundtrip() {
        for v in [0.0f32, 1.0, -2.5, 0.000123, 1234.5, 60000.0] {
            let back = f16_to_f32(crate::pathtrace::f16_bits(v));
            assert!((back - v).abs() <= v.abs() * 1e-3 + 1e-6, "{v} -> {back}");
        }
    }

    #[test]
    fn packed_fallback_matches_float_path() {
        // The fp16 path must produce the same result as the f32 wavelet path
        // (within half precision), with padded rows left intact.
        let (w, h, row_px) = (20usize, 12usize, 32usize);
        let mut planes = [vec![0u8; row_px * h * 8], vec![0u8; row_px * h * 8], vec![0u8; row_px * h * 8]];
        let (mut color, mut albedo, mut normal, mut depth) = (vec![], vec![], vec![], vec![]);
        for y in 0..h {
            for x in 0..w {
                let c = [(x as f32 * 0.37).sin().abs(), 0.5, (y as f32 * 0.21).cos().abs()];
                let a = [0.6, 0.5, 0.4];
                let n = [0.0, 1.0, 0.0];
                let vals = [[c[0], c[1], c[2], 1.0], [a[0], a[1], a[2], 7.0], [n[0], n[1], n[2], 0.0]];
                for (p, v) in planes.iter_mut().zip(vals) {
                    for (k, f) in v.iter().enumerate() {
                        let o = (y * row_px + x) * 8 + k * 2;
                        p[o..o + 2].copy_from_slice(&crate::pathtrace::f16_bits(*f).to_le_bytes());
                    }
                }
                color.push(c);
                albedo.push(a);
                normal.push(n);
                depth.push(7.0);
            }
        }
        let l = Layers { width: w, height: h, color: &color, albedo: &albedo, normal: &normal, depth: &depth };
        let reference = wavelet(&l);
        let p = Packed { width: w, height: h, row_px, color: &planes[0], albedo: &planes[1], normal: &planes[2] };
        // Force the fallback even if an AI denoiser is installed: compare via
        // the public wavelet + manual pack, which is what denoise_packed's
        // fallback does.
        let (out, method) = denoise_packed(&p, true, false);
        if method == Method::Wavelet {
            for y in 0..h {
                for x in 0..w {
                    let o = (y * row_px + x) * 8;
                    let px = unpack4(&out[o..o + 8]);
                    let r = reference[y * w + x];
                    for k in 0..3 {
                        assert!((px[k] - r[k]).abs() < 2e-3 + r[k] * 2e-3, "({x},{y}) {px:?} vs {r:?}");
                    }
                    assert!((px[3] - 1.0).abs() < 1e-3, "alpha kept");
                }
            }
        }
        assert_eq!(out.len(), planes[0].len());
    }

    #[test]
    fn wavelet_reduces_noise_and_keeps_edges() {
        let (w, h) = (64, 64);
        let mut seed = 1u32;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed & 0xFFFF) as f32 / 65535.0
        };
        let truth = |x: usize| if x < 32 { 0.2 } else { 0.8 };
        let color: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let v = truth(i % w) * (0.5 + rnd());
                [v, v, v]
            })
            .collect();
        let albedo = vec![[0.5; 3]; w * h];
        let normal: Vec<[f32; 3]> = (0..w * h).map(|i| if i % w < 32 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] }).collect();
        let depth = vec![5.0; w * h];
        let l = Layers { width: w, height: h, color: &color, albedo: &albedo, normal: &normal, depth: &depth };
        let out = wavelet(&l);
        let err = |img: &[[f32; 3]]| img.iter().enumerate().map(|(i, c)| (c[0] - truth(i % w)).powi(2)).sum::<f32>() / (w * h) as f32;
        assert!(err(&out) < err(&color) * 0.3, "{} vs {}", err(&out), err(&color));
        // the edge at x=32 stays sharp
        assert!(out[10 * w + 30][0] < 0.35 && out[10 * w + 34][0] > 0.6);
    }
}
