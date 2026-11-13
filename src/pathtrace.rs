//! GPU path tracer for final-quality photos, movies and the live preview.
//!
//! * Geometry is flattened into vertex/triangle buffers with a SAH BVH
//!   (`bvh.rs`, depth-bounded so the shader's traversal stack can't overflow).
//! * Textures keep their own resolution: one `texture_2d_array` per size
//!   class (512 / 1024 / 2048 / 4096) for colour (sRGB) and data (linear).
//!   A VRAM budget demotes only the largest textures when exceeded.
//! * Geometry, textures and the material table are cached separately, so
//!   editing a material doesn't rebuild the BVH and moving an object doesn't
//!   re-process textures.
//! * Samples accumulate over several asynchronous passes (up to two in
//!   flight); `step` never drains the GPU queue beyond its time budget.
//! * A GPU kernel averages and packs the result to fp16 for read-back
//!   (24 bytes/pixel), which goes straight to Open Image Denoise and back
//!   up as the renderer's HDR image for its post-processing chain.

use crate::bvh;
use crate::denoise;
use crate::mips::{self, MapKind};
use crate::renderer::{FrameParams, Renderer, Targets};
use crate::scene::{Scene, TextureData};
use glam::Vec3;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Device limits the path tracer needs (large storage buffers for 4K AOVs,
/// many texture layers).
pub fn wanted_limits(adapter: &wgpu::Limits) -> wgpu::Limits {
    wgpu::Limits {
        max_texture_dimension_2d: 8192.min(adapter.max_texture_dimension_2d),
        max_texture_array_layers: adapter.max_texture_array_layers.min(2048),
        max_storage_buffer_binding_size: adapter.max_storage_buffer_binding_size.min(1 << 30),
        max_buffer_size: adapter.max_buffer_size.min(1 << 31),
        max_storage_buffers_per_shader_stage: adapter.max_storage_buffers_per_shader_stage.min(16),
        ..wgpu::Limits::default()
    }
}

/// Why an output size can't be rendered on this GPU, and the largest size
/// with the same aspect ratio that can.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolutionLimit {
    pub requested: (u32, u32),
    pub reason: String,
    pub max_supported: (u32, u32),
}

impl std::fmt::Display for ResolutionLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} x {} can't be rendered on this graphics card: {}. The highest supported size at this aspect ratio is {} x {}.",
            self.requested.0, self.requested.1, self.reason, self.max_supported.0, self.max_supported.1
        )
    }
}

/// GPU buffer sizes (bytes) a path-traced image of w x h needs.
pub fn job_buffer_sizes(w: u32, h: u32) -> [(&'static str, u64); 4] {
    let n = w as u64 * h as u64;
    let plane = (w.div_ceil(32) * 32) as u64 * h as u64 * 8;
    [("sample accumulation", n * 16), ("albedo/normal layers", n * 32), ("denoiser read-back", plane * 3), ("HDR image", plane)]
}

/// First limit that w x h breaks (None = fits). `path_traced` adds the
/// path tracer's buffers to the rasterizer's render targets.
fn limit_issue(limits: &wgpu::Limits, w: u32, h: u32, path_traced: bool) -> Option<String> {
    let mb = |b: u64| format!("{} MB", b.div_ceil(1 << 20));
    if w.max(h) > limits.max_texture_dimension_2d {
        return Some(format!("the image would be {} px wide/high, but this card's largest texture is {} px", w.max(h), limits.max_texture_dimension_2d));
    }
    // Read-back of the finished 8-bit image.
    let out = w as u64 * h as u64 * 4;
    if out > limits.max_buffer_size {
        return Some(format!("the finished image needs a {} buffer, but this card allows {} per buffer", mb(out), mb(limits.max_buffer_size)));
    }
    if path_traced {
        let storage_max = (limits.max_storage_buffer_binding_size as u64).min(limits.max_buffer_size);
        for (what, size) in job_buffer_sizes(w, h) {
            if size > storage_max {
                return Some(format!("ray tracing needs a {} buffer for the {what}, but this card allows {} per buffer", mb(size), mb(storage_max)));
            }
        }
    }
    None
}

/// Check an output size against the selected GPU's limits before rendering.
pub fn check_resolution(limits: &wgpu::Limits, w: u32, h: u32, path_traced: bool) -> Result<(), ResolutionLimit> {
    let Some(reason) = limit_issue(limits, w, h, path_traced) else { return Ok(()) };
    Err(ResolutionLimit { requested: (w, h), reason, max_supported: max_resolution(limits, w, h, path_traced) })
}

/// Largest size with the aspect ratio of w x h (even dimensions) that fits
/// within the GPU limits.
pub fn max_resolution(limits: &wgpu::Limits, w: u32, h: u32, path_traced: bool) -> (u32, u32) {
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    let size = |s: f64| (((w as f64 * s) as u32 / 2 * 2).max(2), ((h as f64 * s) as u32 / 2 * 2).max(2));
    if limit_issue(limits, w, h, path_traced).is_none() {
        return (w, h);
    }
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        let (a, b) = size(mid);
        if limit_issue(limits, a, b, path_traced).is_none() {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    size(lo)
}

/// Optional features used when the adapter has them (never required):
/// GPU timestamps for accurate pass timing.
pub fn wanted_features(adapter: wgpu::Features) -> wgpu::Features {
    adapter & wgpu::Features::TIMESTAMP_QUERY
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PtOpts {
    /// Samples per pixel.
    pub spp: u32,
    pub bounces: u32,
    pub denoise: bool,
}

impl PtOpts {
    pub fn preset(level: u32) -> Self {
        match level {
            // Bounce limits sized for interiors: white rooms lose ~18% of
            // their light at 5 bounces vs 14 (tests/pathtrace_portals.rs);
            // Russian roulette ends low-energy paths early, so deep limits
            // cost little where they aren't needed.
            0 => PtOpts { spp: 16, bounces: 4, denoise: true },
            1 => PtOpts { spp: 64, bounces: 8, denoise: true },
            2 => PtOpts { spp: 256, bounces: 10, denoise: true },
            _ => PtOpts { spp: 1024, bounces: 14, denoise: true },
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    size: [u32; 4],
    tile: [u32; 4],
    opts: [f32; 4],
    lens: [f32; 4],
    portal: [u32; 4],
    _pad: [[f32; 4]; 11], // 256-byte stride for dynamic offsets
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Mat {
    base: [f32; 4],
    p0: [f32; 4],
    p1: [f32; 4],
    uvt: [f32; 4],
    p3: [f32; 4],
    p4: [f32; 4],
    p5: [f32; 4],
}

const TILE: u32 = 384;
const MAX_TILES: usize = 128;
/// Texture size classes; must match `map_level` in pathtrace.wgsl
/// (class c is 128 << c pixels square). Small maps stay small, and over
/// budget maps can shrink down to 128 px.
pub const CLASS_SIZES: [u32; 6] = [128, 256, 512, 1024, 2048, 4096];
const NCLASS: usize = CLASS_SIZES.len();
/// Passes queued on the GPU at once: one running, one waiting, so the GPU
/// never idles between UI frames while the CPU never waits on more than
/// one pass.
const MAX_IN_FLIGHT: usize = 2;
/// Upper bound for one dispatch, well under Windows' 2 s GPU timeout even
/// when a Thunderbolt eGPU is also driving displays.
const MAX_DISPATCH_MS: f32 = 120.0;
const TS_SLOTS: usize = 4;

/// Default budget for path-tracer textures; `LUMENCRAFT_PT_TEXTURE_MB`
/// overrides it.
pub const DEFAULT_TEXTURE_BUDGET_MB: u64 = 3072;

/// Per-pixel colour, albedo, normal and first-hit distance (see `PathTracer::read_layers`).
pub type PixelLayers = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<f32>);

/// GPU memory the path tracer has allocated (bytes), for logs and benchmarks.
#[derive(Clone, Copy, Debug, Default)]
pub struct PtMemory {
    pub geometry: u64,
    pub textures: u64,
    pub job_buffers: u64,
    /// Layers per size class (colour, data), 128 px .. 4096 px.
    pub color_layers: [u32; NCLASS],
    pub data_layers: [u32; NCLASS],
    /// Maps stored smaller than their own resolution to fit the budget.
    pub demoted: u32,
    /// Maps left out entirely (the budget couldn't hold even 128 px copies).
    pub dropped: u32,
    /// Smallest size a reduced map was stored at (px), if any were reduced.
    pub smallest_reduced: Option<u32>,
    /// The texture budget in force (bytes).
    pub budget: u64,
}

impl PtMemory {
    /// One-line description of any texture-quality reduction, for the UI.
    pub fn reduction_note(&self) -> Option<String> {
        if self.demoted == 0 && self.dropped == 0 {
            return None;
        }
        let mut s = format!("Ray tracing: textures reduced to fit the {} MB texture budget", self.budget >> 20);
        if self.demoted > 0 {
            s += &format!(" - {} map(s) scaled down", self.demoted);
            if let Some(px) = self.smallest_reduced {
                s += &format!(" (smallest {px} px)");
            }
        }
        if self.dropped > 0 {
            s += &format!(", {} map(s) left out", self.dropped);
        }
        Some(s)
    }
}

struct GeomCache {
    sig: u64,
    buffers: [wgpu::Buffer; 4], // positions, attrs, tris, nodes (+ portal table)
    bytes: u64,
    triangles: usize,
    depth: u32,
    /// Window glass used to sample sky light: entries in the node buffer
    /// from `portal_base` (lo.x = cumulative area, a = triangle index).
    portal_base: u32,
    portal_count: u32,
    portal_area: f32,
}

struct TexCache {
    sig: u64,
    views: Vec<wgpu::TextureView>, // 4 colour classes, then 4 data classes
    codes: HashMap<(u64, MapKind), f32>,
    bytes: u64,
    color_layers: [u32; NCLASS],
    data_layers: [u32; NCLASS],
    demoted: u32,
    dropped: u32,
    smallest_reduced: Option<u32>,
}

struct MatCache {
    bytes: Vec<u8>,
    buf: wgpu::Buffer,
}

/// Per-image buffers, pooled across jobs of the same size (the live
/// preview restarts a job every time the camera stops).
struct JobBuffers {
    w: u32,
    h: u32,
    accum: wgpu::Buffer,
    aux: wgpu::Buffer,
    packed: wgpu::Buffer,
    readback: wgpu::Buffer,
    resolve_params: wgpu::Buffer,
}

impl JobBuffers {
    fn bytes(&self) -> u64 {
        self.accum.size() + self.aux.size() + self.packed.size() * 2
    }
}

/// Stages of an asynchronous preview update.
enum PreviewState {
    /// Resolve + copy queued behind the in-flight passes; waiting for the map.
    Reading { mapped: Arc<AtomicBool>, spp: u32, t0: Instant },
    /// Bytes copied out; denoising on a worker thread.
    Denoising { spp: u32, rx: std::sync::mpsc::Receiver<(Vec<u8>, Option<denoise::Method>, f32)>, t0: Instant, read_ms: f32 },
}

/// A finished preview image (HDR, fp16 rows) and how long it took.
pub struct PreviewImage {
    pub hdr: Vec<u8>,
    pub spp: u32,
    pub method: Option<denoise::Method>,
    /// From request until the GPU copy was readable.
    pub read_ms: f32,
    /// Denoising on the worker thread.
    pub denoise_ms: f32,
    /// From request until ready.
    pub latency_ms: f32,
}

struct Pass {
    index: wgpu::SubmissionIndex,
    /// Samples per pixel this pass completes (0 for all but the last slice
    /// of a sliced round).
    spp: u32,
    /// Full-frame samples' worth of work in this pass (batch x fraction of tiles).
    work: f32,
    done: Arc<AtomicBool>,
    submitted: Instant,
    ts_slot: Option<usize>,
}

struct Timestamps {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: Vec<wgpu::Buffer>,
    /// Slot is mapped or being mapped (can't be reused until unmapped).
    busy: Vec<Option<Arc<AtomicBool>>>,
    next: usize,
    period_ns: f32,
}

pub struct PathTracer {
    pipe: wgpu::ComputePipeline,
    resolve_pipe: wgpu::ComputePipeline,
    bgl0: wgpu::BindGroupLayout,
    bgl1: wgpu::BindGroupLayout,
    bgl2: wgpu::BindGroupLayout,
    resolve_bgl: wgpu::BindGroupLayout,
    params_buf: wgpu::Buffer,
    sampler: wgpu::Sampler,
    geom: Option<GeomCache>,
    tex: Option<TexCache>,
    mats: Option<MatCache>,
    bg1: Option<wgpu::BindGroup>,
    pool: Vec<JobBuffers>,
    timestamps: Option<Timestamps>,
    /// GPU milliseconds per sample per megapixel, learned across jobs so a
    /// new job starts with a sensible batch size.
    ms_per_mpix_sample: f32,
    texture_budget: u64,
    /// Sample sky light through windows (glass portals). Unbiased and ~43%
    /// less variance indoors per sample, but each sample costs ~2.3x as
    /// much (an extra long shadow ray), which is a net loss in time to a
    /// given quality on current hardware - so off by default; enable with
    /// `LUMENCRAFT_WINDOW_SAMPLING=1` (see docs/v0.11-engineering-report.md).
    pub window_sampling: bool,
    /// Passes kept queued. Two keeps a GPU busy between UI frames; drivers
    /// whose submit blocks until the previous work finishes (software
    /// Vulkan, some eGPU setups) drop to one so the UI never waits a pass.
    max_in_flight: usize,
}

/// A ray-traced image being accumulated.
pub struct PtJob {
    pub width: u32,
    pub height: u32,
    pub opts: PtOpts,
    /// Samples per pixel completed on the GPU.
    pub done_spp: u32,
    /// Samples per pixel submitted (completed or in flight).
    pub submitted_spp: u32,
    bufs: Option<JobBuffers>,
    bg0: wgpu::BindGroup,
    bg2: wgpu::BindGroup,
    resolve_bg: wgpu::BindGroup,
    spread: f32,
    seed: u32,
    /// Aperture radius (m) and focus distance (m) for real depth of field.
    lens: [f32; 2],
    /// Samples per pixel per dispatch, tuned to the GPU's speed.
    batch: u32,
    /// A sample over the whole image is split across this many passes when
    /// one would take longer than `target_ms` (large images in the live
    /// preview), so the GPU is never tied up long enough to stall the UI.
    slices: u32,
    slice: u32,
    /// Target GPU time of one pass (all tiles), ms. Small for the live
    /// preview (the viewport shares the GPU), larger for final renders.
    pub target_ms: f32,
    /// Un-jittered pixel-centre primary rays (tests compare against the CPU).
    pub centre_rays: bool,
    pub started: Instant,
    in_flight: VecDeque<Pass>,
    /// Asynchronous preview snapshot in progress (see `request_preview`).
    preview: Option<PreviewState>,
    last_retire: Instant,
    pass_counter: u32,
    /// Measured GPU time of the last pass, ms.
    pub last_pass_ms: f32,
}

impl PtJob {
    pub fn progress(&self) -> f32 {
        self.done_spp as f32 / self.opts.spp.max(1) as f32
    }
    pub fn finished(&self) -> bool {
        self.done_spp >= self.opts.spp
    }
    /// Passes one sample over the image is currently split into.
    pub fn passes_per_sample(&self) -> u32 {
        self.slices
    }
}

fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

fn tex_entry(binding: u32, dim: wgpu::TextureViewDimension) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: dim, multisampled: false },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn oct_encode(n: Vec3) -> [f32; 2] {
    let n = n / (n.x.abs() + n.y.abs() + n.z.abs()).max(1e-20);
    if n.z >= 0.0 {
        [n.x, n.y]
    } else {
        let s = |v: f32| if v >= 0.0 { 1.0 } else { -1.0 };
        [(1.0 - n.y.abs()) * s(n.x), (1.0 - n.x.abs()) * s(n.y)]
    }
}

fn pack2x16snorm(v: [f32; 2]) -> u32 {
    let q = |x: f32| ((x.clamp(-1.0, 1.0) * 32767.0).round() as i32 as i16) as u16 as u32;
    q(v[0]) | (q(v[1]) << 16)
}

/// IEEE half from f32 (round to nearest, clamps to the f16 range).
pub fn f16_bits(v: f32) -> u16 {
    let v = if v.is_finite() { v.clamp(-65000.0, 65000.0) } else { 0.0 };
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let man = b & 0x7f_ffff;
    if exp <= 0 {
        if exp < -10 {
            return sign;
        }
        let m = (man | 0x80_0000) >> (1 - exp);
        return sign | ((m + 0x1000) >> 13) as u16;
    }
    let r = ((exp as u32) << 10) | (man >> 13);
    let r = r + ((man >> 12) & 1);
    sign | r.min(0x7bff) as u16
}

fn hash_of<T: Hash>(v: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

/// Smallest class whose size covers the texture's larger side (so detail is
/// never thrown away), at most the 4096 class.
fn native_class(t: &TextureData) -> usize {
    let side = t.width.max(t.height);
    CLASS_SIZES.iter().position(|&s| s >= side).unwrap_or(CLASS_SIZES.len() - 1)
}

/// Result of fitting textures into the budget.
#[derive(Debug, Clone, PartialEq)]
pub struct TexturePlan {
    /// Size class per texture, None = left out.
    pub class: Vec<Option<usize>>,
    /// Maps stored below their native class.
    pub demoted: u32,
    /// Maps left out.
    pub dropped: u32,
    pub bytes: u64,
}

/// Assign every texture a size class so the total fits `budget` and each
/// class fits `max_layers`. The budget is enforced:
/// 1. the largest maps shrink one class at a time (last-added first), down
///    to 128 px - smaller maps are only touched once all larger ones have
///    come down to their size;
/// 2. if even that doesn't fit, data maps (normal, roughness) are left out,
///    then colour maps, so the renderer never allocates past the budget.
pub fn plan_classes(native: &[usize], is_data: &[bool], budget: u64, max_layers: u32) -> TexturePlan {
    let layer_bytes = |c: usize| mips::chain_bytes(CLASS_SIZES[c], CLASS_SIZES[c]);
    let mut class: Vec<Option<usize>> = native.iter().map(|&c| Some(c)).collect();
    // Layer limit per class: push overflow down a class (or out).
    for c in (0..NCLASS).rev() {
        let mut count = 0u32;
        for slot in class.iter_mut() {
            if *slot == Some(c) {
                count += 1;
                if count > max_layers {
                    *slot = if c > 0 { Some(c - 1) } else { None };
                }
            }
        }
    }
    let count = |class: &[Option<usize>], c: usize| class.iter().filter(|&&k| k == Some(c)).count() as u32;
    let mut bytes: u64 = class.iter().flatten().map(|&c| layer_bytes(c)).sum();
    while bytes > budget {
        // Highest class that can still move down one step.
        let Some(top) = (1..NCLASS).rev().find(|&c| class.contains(&Some(c)) && count(&class, c - 1) < max_layers) else { break };
        let i = class.iter().rposition(|&k| k == Some(top)).unwrap();
        class[i] = Some(top - 1);
        bytes = bytes - layer_bytes(top) + layer_bytes(top - 1);
    }
    for want_data in [true, false] {
        while bytes > budget {
            let Some(i) = (0..class.len()).rev().find(|&i| class[i].is_some() && is_data.get(i).copied().unwrap_or(false) == want_data) else { break };
            bytes -= layer_bytes(class[i].unwrap());
            class[i] = None;
        }
    }
    let demoted = class.iter().zip(native).filter(|(c, n)| matches!(c, Some(k) if k < n)).count() as u32;
    let dropped = class.iter().filter(|c| c.is_none()).count() as u32;
    TexturePlan { class, demoted, dropped, bytes }
}

impl PathTracer {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pathtrace"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/pathtrace.wgsl").into()),
        });
        let resolve_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pt_resolve"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/pt_resolve.wgsl").into()),
        });
        let uniform = |binding: u32, dynamic: bool, min: u64| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: dynamic, min_binding_size: wgpu::BufferSize::new(min) },
            count: None,
        };
        let bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pt0"),
            entries: &[uniform(0, false, 0), tex_entry(1, wgpu::TextureViewDimension::D2), sampler_entry(2), storage(3, true), uniform(4, true, 80)],
        });
        let mut e1 = vec![storage(0, true), storage(1, true), storage(2, true), storage(3, true), storage(4, true)];
        for b in 5..5 + 2 * NCLASS as u32 {
            e1.push(tex_entry(b, wgpu::TextureViewDimension::D2Array));
        }
        e1.push(sampler_entry(5 + 2 * NCLASS as u32));
        let bgl1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("pt1"), entries: &e1 });
        let bgl2 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("pt2"), entries: &[storage(0, false), storage(1, false)] });
        let resolve_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pt_resolve"),
            entries: &[storage(0, true), storage(1, true), storage(2, false), uniform(3, false, 16)],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pt_layout"),
            bind_group_layouts: &[&bgl0, &bgl1, &bgl2],
            push_constant_ranges: &[],
        });
        let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("pathtrace"),
            layout: Some(&layout),
            module: &module,
            entry_point: "main",
            compilation_options: Default::default(),
            cache: None,
        });
        let resolve_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pt_resolve_layout"),
            bind_group_layouts: &[&resolve_bgl],
            push_constant_ranges: &[],
        });
        let resolve_pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("pt_resolve"),
            layout: Some(&resolve_layout),
            module: &resolve_module,
            entry_point: "resolve",
            compilation_options: Default::default(),
            cache: None,
        });
        let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pt_params"),
            size: (std::mem::size_of::<Params>() * MAX_TILES) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pt_tex"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let timestamps = device.features().contains(wgpu::Features::TIMESTAMP_QUERY).then(|| Timestamps {
            set: device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("pt_ts"), ty: wgpu::QueryType::Timestamp, count: (TS_SLOTS * 2) as u32 }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pt_ts_resolve"),
                size: TS_SLOTS as u64 * wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            read: (0..TS_SLOTS)
                .map(|_| {
                    device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("pt_ts_read"),
                        size: 16,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    })
                })
                .collect(),
            busy: vec![None; TS_SLOTS],
            next: 0,
            period_ns: 1.0,
        });
        let texture_budget = std::env::var("LUMENCRAFT_PT_TEXTURE_MB").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(DEFAULT_TEXTURE_BUDGET_MB) << 20;
        Self {
            pipe,
            resolve_pipe,
            bgl0,
            bgl1,
            bgl2,
            resolve_bgl,
            params_buf,
            sampler,
            geom: None,
            tex: None,
            mats: None,
            bg1: None,
            pool: Vec::new(),
            timestamps,
            ms_per_mpix_sample: 0.0,
            texture_budget,
            window_sampling: std::env::var_os("LUMENCRAFT_WINDOW_SAMPLING").is_some_and(|v| v != "0"),
            max_in_flight: MAX_IN_FLIGHT,
        }
    }

    /// Override the texture memory budget (bytes); takes effect on the next sync.
    pub fn set_texture_budget(&mut self, bytes: u64) {
        if bytes != self.texture_budget {
            self.texture_budget = bytes;
            self.tex = None;
        }
    }

    /// Number of triangles in the last built acceleration structure.
    pub fn triangles(&self) -> usize {
        self.geom.as_ref().map_or(0, |g| g.triangles)
    }

    /// Depth of the last built BVH.
    pub fn bvh_depth(&self) -> u32 {
        self.geom.as_ref().map_or(0, |g| g.depth)
    }

    /// GPU memory currently held by the path tracer.
    pub fn memory(&self) -> PtMemory {
        let mut m = PtMemory::default();
        if let Some(g) = &self.geom {
            m.geometry = g.bytes;
        }
        if let Some(t) = &self.tex {
            m.textures = t.bytes;
            m.color_layers = t.color_layers;
            m.data_layers = t.data_layers;
            m.demoted = t.demoted;
            m.dropped = t.dropped;
            m.smallest_reduced = t.smallest_reduced;
        }
        if let Some(mc) = &self.mats {
            m.geometry += mc.buf.size();
        }
        m.job_buffers = self.pool.iter().map(|b| b.bytes()).sum();
        m.budget = self.texture_budget;
        m
    }

    /// The texture budget in force (bytes).
    pub fn texture_budget(&self) -> u64 {
        self.texture_budget
    }

    /// Bring geometry, textures and materials up to date with the scene.
    /// Each part is rebuilt only when what it depends on changed.
    pub fn sync(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, scene: &Scene) {
        let t0 = Instant::now();
        // 1. Materials in mesh order (deterministic indices) and their maps.
        struct Src {
            p: crate::scene::MaterialParams,
            maps: [Option<TextureData>; 3], // colour, normal, roughness
        }
        let mut mat_index: HashMap<&str, u32> = HashMap::new();
        let mut srcs: Vec<Src> = Vec::new();
        let mut mesh_mat: Vec<u32> = Vec::with_capacity(scene.meshes.len());
        for sm in scene.meshes.values() {
            let name = sm.data.material.as_str();
            let mi = *mat_index.entry(name).or_insert_with(|| {
                let p = scene.effective(name);
                if p.invisible {
                    return u32::MAX;
                }
                let src = scene.imported_materials.get(name);
                let user = |path: &Option<String>, kind| path.as_ref().and_then(|p| scene.user_textures.get(&crate::scene::user_texture_key(p, kind)).cloned());
                let color = match &p.color_map {
                    Some(_) => user(&p.color_map, crate::texlib::MapKind::Color),
                    None if p.use_texture => src.and_then(|m| m.texture.clone()),
                    None => None,
                };
                let normal = match &p.normal_map {
                    Some(_) => user(&p.normal_map, crate::texlib::MapKind::Normal),
                    None => src.and_then(|m| m.normal_texture.clone()),
                };
                let rough = match &p.roughness_map {
                    Some(_) => user(&p.roughness_map, crate::texlib::MapKind::Roughness),
                    None => src.and_then(|m| m.roughness_texture.clone()),
                };
                srcs.push(Src { p, maps: [color, normal, rough] });
                srcs.len() as u32 - 1
            });
            mesh_mat.push(mi);
        }

        // 2. Textures (unique by content key and use).
        let kinds = [MapKind::Srgb, MapKind::Normal, MapKind::Linear];
        let mut tex_list: Vec<(TextureData, MapKind)> = Vec::new();
        let mut seen: HashMap<(u64, MapKind), ()> = HashMap::new();
        for s in &srcs {
            for (k, m) in s.maps.iter().enumerate() {
                if let Some(t) = m {
                    if seen.insert((t.key, kinds[k]), ()).is_none() {
                        tex_list.push((t.clone(), kinds[k]));
                    }
                }
            }
        }
        let tex_sig = hash_of(&(tex_list.iter().map(|(t, k)| (t.key, *k as u8)).collect::<Vec<_>>(), self.texture_budget));
        let mut rebind = self.bg1.is_none();
        if self.tex.as_ref().is_none_or(|t| t.sig != tex_sig) {
            self.tex = Some(self.build_textures(device, queue, &tex_list, tex_sig));
            rebind = true;
        }
        let tex = self.tex.as_ref().unwrap();

        // 3. Material table.
        let code = |t: &Option<TextureData>, k: MapKind| -> f32 { t.as_ref().and_then(|t| tex.codes.get(&(t.key, k)).copied()).unwrap_or(-1.0) };
        let mut mats: Vec<Mat> = srcs
            .iter()
            .map(|s| {
                let p = &s.p;
                let cl = code(&s.maps[0], MapKind::Srgb);
                let e = p.emissive_strength;
                let cutout = p.cutout && cl >= 0.0 && !p.kind.is_transparent();
                Mat {
                    base: p.base_color,
                    p0: [p.metallic, p.roughness, p.kind.id() as f32, p.uv_scale.max(0.001)],
                    p1: [cl, code(&s.maps[1], MapKind::Normal), code(&s.maps[2], MapKind::Linear), p.normal_strength],
                    uvt: p.uv_transform,
                    p3: [p.uv_rotation.to_radians(), p.uv_offset[0], p.uv_offset[1], p.tex_influence],
                    p4: [p.emissive[0] * e, p.emissive[1] * e, p.emissive[2] * e, p.reflectivity],
                    p5: [p.variation, if cutout { 1.0 } else { 0.0 }, if cl >= 0.0 { 1.0 } else { 0.0 }, if p.box_mapping { 1.0 } else { 0.0 }],
                }
            })
            .collect();
        if mats.is_empty() {
            mats.push(Mat {
                base: [0.8, 0.8, 0.8, 1.0],
                p0: [0.0, 0.6, 0.0, 1.0],
                p1: [-1.0, -1.0, -1.0, 1.0],
                uvt: [1.0, 1.0, 0.0, 0.0],
                p3: [0.0, 0.0, 0.0, 1.0],
                p4: [0.0, 0.0, 0.0, 0.5],
                p5: [0.0; 4],
            });
        }
        let mat_bytes: &[u8] = bytemuck::cast_slice(&mats);
        if self.mats.as_ref().is_none_or(|m| m.bytes != mat_bytes) {
            use wgpu::util::DeviceExt;
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("pt_mats"),
                contents: mat_bytes,
                usage: wgpu::BufferUsages::STORAGE,
            });
            self.mats = Some(MatCache { bytes: mat_bytes.to_vec(), buf });
            rebind = true;
        }

        // 4. Geometry + BVH.
        let kinds: Vec<u32> = mats.iter().map(|m| m.p0[2] as u32).collect();
        let geom_sig = hash_of(&(scene.meshes.iter().zip(&mesh_mat).map(|((id, m), mi)| (id.as_str(), m.rev, m.hash, *mi)).collect::<Vec<_>>(), &kinds));
        if self.geom.as_ref().is_none_or(|g| g.sig != geom_sig) {
            self.geom = Some(Self::build_geometry(device, scene, &mesh_mat, &kinds, geom_sig));
            rebind = true;
        }

        if rebind {
            let g = self.geom.as_ref().unwrap();
            let tex = self.tex.as_ref().unwrap();
            let mut entries: Vec<wgpu::BindGroupEntry> =
                g.buffers.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() }).collect();
            entries.push(wgpu::BindGroupEntry { binding: 4, resource: self.mats.as_ref().unwrap().buf.as_entire_binding() });
            for (i, v) in tex.views.iter().enumerate() {
                entries.push(wgpu::BindGroupEntry { binding: 5 + i as u32, resource: wgpu::BindingResource::TextureView(v) });
            }
            entries.push(wgpu::BindGroupEntry { binding: 5 + 2 * NCLASS as u32, resource: wgpu::BindingResource::Sampler(&self.sampler) });
            self.bg1 = Some(device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("pt1"), layout: &self.bgl1, entries: &entries }));
            let m = self.memory();
            log::info!(
                "path tracer scene: {} triangles (BVH depth {}), {} materials, textures {}+{} layers by class 128/256/512/1K/2K/4K {:?}/{:?}, {} scaled down, {:.0} MB textures + {:.0} MB geometry, synced in {:.2}s",
                g.triangles,
                g.depth,
                mats.len(),
                m.color_layers.iter().sum::<u32>(),
                m.data_layers.iter().sum::<u32>(),
                m.color_layers,
                m.data_layers,
                m.demoted,
                m.textures as f64 / 1048576.0,
                m.geometry as f64 / 1048576.0,
                t0.elapsed().as_secs_f32()
            );
        }
    }

    fn build_geometry(device: &wgpu::Device, scene: &Scene, mesh_mat: &[u32], kinds: &[u32], sig: u64) -> GeomCache {
        let mut positions: Vec<[f32; 4]> = Vec::new();
        let mut attrs: Vec<[f32; 4]> = Vec::new();
        let mut tris: Vec<[u32; 4]> = Vec::new();
        let mut corners: Vec<[Vec3; 3]> = Vec::new();
        let mut skipped_bad = 0usize;
        for (sm, &mi) in scene.meshes.values().zip(mesh_mat) {
            let md = &sm.data;
            if mi == u32::MAX || md.positions.is_empty() {
                continue;
            }
            let base = positions.len() as u32;
            for (i, p) in md.positions.iter().enumerate() {
                positions.push([p.x, p.y, p.z, 1.0]);
                let n = md.normals.get(i).copied().unwrap_or(Vec3::Y);
                let n = if n.is_finite() && n.length_squared() > 1e-12 { n.normalize() } else { Vec3::Y };
                let uv = md.uvs.get(i).copied().filter(|u| u.is_finite()).unwrap_or_default();
                attrs.push([f32::from_bits(pack2x16snorm(oct_encode(n))), uv.x, uv.y, 0.0]);
            }
            let nv = md.positions.len();
            for t in md.indices.chunks_exact(3) {
                let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
                if a >= nv || b >= nv || c >= nv {
                    skipped_bad += 1;
                    continue;
                }
                let (pa, pb, pc) = (md.positions[a], md.positions[b], md.positions[c]);
                // A NaN/inf vertex would poison the bounds of a whole BVH
                // subtree (hiding unrelated geometry); exactly degenerate
                // triangles can never be hit. Everything else is kept, however
                // small (fine trim, mullions, hardware).
                let area2 = (pb - pa).cross(pc - pa).length_squared();
                if !(pa.is_finite() && pb.is_finite() && pc.is_finite()) || area2.is_nan() || area2 <= 0.0 {
                    skipped_bad += 1;
                    continue;
                }
                tris.push([base + t[0], base + t[1], base + t[2], mi]);
                corners.push([pa, pb, pc]);
            }
        }
        if skipped_bad > 0 {
            log::info!("path tracer: skipped {skipped_bad} degenerate or invalid triangles");
        }
        if tris.is_empty() {
            // A tiny triangle far below the world keeps the buffers valid.
            let b = positions.len() as u32;
            for p in [[0.0, -1e5, 0.0], [0.001, -1e5, 0.0], [0.0, -1e5, 0.001]] {
                positions.push([p[0], p[1], p[2], 1.0]);
                attrs.push([0.0; 4]);
            }
            tris.push([b, b + 1, b + 2, 0]);
            corners.push([Vec3::new(0.0, -1e5, 0.0), Vec3::new(0.001, -1e5, 0.0), Vec3::new(0.0, -1e5, 0.001)]);
        }
        let tree = bvh::build(&corners);
        assert!(tree.depth <= bvh::MAX_DEPTH, "BVH deeper than the shader's traversal stack");
        let ordered: Vec<[u32; 4]> = tree.order.iter().map(|&i| tris[i as usize]).collect();
        // Window portals: every glass triangle, area-weighted (cumulative).
        let mut nodes = tree.nodes.clone();
        let portal_base = nodes.len() as u32;
        let mut portal_area = 0.0f32;
        for (k, t) in ordered.iter().enumerate() {
            if kinds.get(t[3] as usize) == Some(&1) {
                let [a, b, c] = corners[tree.order[k] as usize];
                let area = 0.5 * (b - a).cross(c - a).length();
                if area > 0.0 && area.is_finite() {
                    portal_area += area;
                    nodes.push(bvh::Node { lo: [portal_area, 0.0, 0.0], a: k as u32, hi: [0.0; 3], b: 0 });
                }
            }
        }
        let portal_count = nodes.len() as u32 - portal_base;
        use wgpu::util::DeviceExt;
        let mk = |label: &str, bytes: &[u8]| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: bytes, usage: wgpu::BufferUsages::STORAGE })
        };
        let buffers = [
            mk("pt_positions", bytemuck::cast_slice(&positions)),
            mk("pt_attrs", bytemuck::cast_slice(&attrs)),
            mk("pt_tris", bytemuck::cast_slice(&ordered)),
            mk("pt_nodes", bytemuck::cast_slice(&nodes)),
        ];
        let bytes = buffers.iter().map(|b| b.size()).sum();
        if portal_count > 0 {
            log::info!("path tracer: {portal_count} window triangles ({portal_area:.1} m2) used to sample sky light");
        }
        GeomCache { sig, buffers, bytes, triangles: ordered.len(), depth: tree.depth, portal_base, portal_count, portal_area }
    }

    fn build_textures(&self, device: &wgpu::Device, queue: &wgpu::Queue, list: &[(TextureData, MapKind)], sig: u64) -> TexCache {
        let max_layers = device.limits().max_texture_array_layers.min(1024);
        let native: Vec<usize> = list.iter().map(|(t, _)| native_class(t)).collect();
        let is_data: Vec<bool> = list.iter().map(|(_, k)| *k != MapKind::Srgb).collect();
        // Colour and data maps live in separate arrays but share the budget.
        let plan = plan_classes(&native, &is_data, self.texture_budget, max_layers);
        let classes = plan.class.clone();
        let smallest_reduced = classes.iter().zip(&native).filter_map(|(c, n)| c.filter(|k| k < n).map(|k| CLASS_SIZES[k])).min();
        let mut codes = HashMap::new();
        let mut views = Vec::with_capacity(2 * NCLASS);
        let mut bytes = 0u64;
        let mut color_layers = [0u32; NCLASS];
        let mut data_layers = [0u32; NCLASS];
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
        // queue.write_texture stages data in host memory until the next
        // submission; flush regularly so a large model's textures aren't all
        // held in RAM at once on top of the finished GPU copies.
        let mut staged = 0u64;
        for srgb in [true, false] {
            for (c, &size) in CLASS_SIZES.iter().enumerate() {
                let members: Vec<usize> = (0..list.len()).filter(|&i| classes[i] == Some(c) && (list[i].1 == MapKind::Srgb) == srgb).collect();
                let layers = members.len() as u32;
                if srgb {
                    color_layers[c] = layers;
                } else {
                    data_layers[c] = layers;
                }
                let (tex_size, mip_count) = if layers == 0 { (1, 1) } else { (size, size.ilog2() + 1) };
                let tex = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("pt_textures"),
                    size: wgpu::Extent3d { width: tex_size, height: tex_size, depth_or_array_layers: layers.max(1) },
                    mip_level_count: mip_count,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: if srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm },
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                if layers == 0 {
                    queue.write_texture(
                        tex.as_image_copy(),
                        &[255, 255, 255, 255],
                        wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
                        wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    );
                } else {
                    bytes += mips::chain_bytes(size, size) * layers as u64;
                    // Resample + mips in parallel, a few textures at a time so
                    // peak RAM stays bounded, uploading as each batch finishes.
                    for (batch_i, batch) in members.chunks(threads).enumerate() {
                        let chains: Vec<Vec<(u32, u32, Vec<u8>)>> = std::thread::scope(|s| {
                            let hs: Vec<_> = batch
                                .iter()
                                .map(|&i| {
                                    let (t, kind) = &list[i];
                                    s.spawn(move || {
                                        let base = mips::resample(t.width, t.height, &t.rgba, size, size, *kind);
                                        mips::mip_chain(size, size, base, *kind)
                                    })
                                })
                                .collect();
                            hs.into_iter().map(|h| h.join().unwrap_or_default()).collect()
                        });
                        for (k, chain) in chains.iter().enumerate() {
                            let layer = (batch_i * threads + k) as u32;
                            for (level, (w, h, data)) in chain.iter().enumerate() {
                                queue.write_texture(
                                    wgpu::ImageCopyTexture {
                                        texture: &tex,
                                        mip_level: level as u32,
                                        origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                                        aspect: wgpu::TextureAspect::All,
                                    },
                                    data,
                                    wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(*h) },
                                    wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
                                );
                                staged += data.len() as u64;
                            }
                        }
                        drop(chains);
                        if staged > 256 << 20 {
                            let idx = queue.submit(std::iter::empty());
                            device.poll(wgpu::Maintain::WaitForSubmissionIndex(idx));
                            staged = 0;
                        }
                    }
                    for (layer, &i) in members.iter().enumerate() {
                        codes.insert((list[i].0.key, list[i].1), (c * 1024 + layer) as f32);
                    }
                }
                views.push(tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() }));
            }
        }
        if plan.demoted > 0 || plan.dropped > 0 {
            log::warn!(
                "path tracer: texture budget {} MB enforced - {} map(s) scaled down (smallest {} px), {} left out",
                self.texture_budget >> 20,
                plan.demoted,
                smallest_reduced.unwrap_or(0),
                plan.dropped
            );
        }
        TexCache { sig, views, codes, bytes, color_layers, data_layers, demoted: plan.demoted, dropped: plan.dropped, smallest_reduced }
    }

    fn take_buffers(&mut self, device: &wgpu::Device, w: u32, h: u32) -> JobBuffers {
        if let Some(i) = self.pool.iter().position(|b| b.w == w && b.h == h) {
            return self.pool.swap_remove(i);
        }
        let n = (w * h) as u64;
        let row_px = w.div_ceil(32) * 32; // 256-byte rows of 8-byte pixels
        let plane = row_px as u64 * h as u64 * 8;
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST;
        let mk = |label: &str, size: u64, usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
        let resolve_params = mk("pt_resolve_params", 16, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        JobBuffers {
            w,
            h,
            accum: mk("pt_accum", n * 16, usage),
            aux: mk("pt_aux", n * 32, usage),
            packed: mk("pt_packed", plane * 3, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC),
            readback: mk("pt_readback", plane * 3, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST),
            resolve_params,
        }
    }

    /// Return a finished or abandoned job's buffers for reuse.
    pub fn recycle(&mut self, mut job: PtJob) {
        if let (Some(PreviewState::Reading { .. }), Some(b)) = (&job.preview, &job.bufs) {
            // Cancel the pending map so the pooled buffer can be reused.
            b.readback.unmap();
        }
        if let Some(b) = job.bufs.take() {
            self.pool.push(b);
            // Keep the two most recent sizes (viewport preview + photo size).
            while self.pool.len() > 2 {
                self.pool.remove(0);
            }
        }
    }

    /// `begin` that reports instead of crashing when the image is too large
    /// for this GPU: checks the limits first, then catches allocation
    /// failures (out of memory) while the job's buffers are created.
    pub fn try_begin(&mut self, r: &mut Renderer, scene: &Scene, fp: &FrameParams, targets: &Targets, opts: PtOpts) -> Result<PtJob, ResolutionLimit> {
        let (w, h) = targets.size;
        let limits = r.device.limits();
        check_resolution(&limits, w, h, true)?;
        let device = r.device.clone();
        device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let job = self.begin(r, scene, fp, targets, opts);
        let validation = pollster::block_on(device.pop_error_scope());
        let oom = pollster::block_on(device.pop_error_scope());
        if let Some(e) = oom.or(validation) {
            let need: u64 = job_buffer_sizes(w, h).iter().map(|(_, b)| b).sum();
            self.pool.clear();
            // Assume half the pixels fit when the card ran out of memory.
            let half = ((w as f64 * std::f64::consts::FRAC_1_SQRT_2) as u32 / 2 * 2, (h as f64 * std::f64::consts::FRAC_1_SQRT_2) as u32 / 2 * 2);
            return Err(ResolutionLimit {
                requested: (w, h),
                reason: format!("the card couldn't allocate the {} MB of ray-tracing buffers ({e})", need >> 20),
                max_supported: half,
            });
        }
        Ok(job)
    }

    /// Start a ray-traced render. Runs the rasterizer once so the frame
    /// uniforms, sky and lights for this view are current, and keeps private
    /// copies of them so the interactive viewport can keep drawing meanwhile.
    pub fn begin(&mut self, r: &mut Renderer, scene: &Scene, fp: &FrameParams, targets: &Targets, opts: PtOpts) -> PtJob {
        let device = r.device.clone();
        let queue = r.queue.clone();
        self.sync(&device, &queue, scene);
        r.render(scene, fp, targets);
        let (width, height) = targets.size;
        let frame = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pt_frame"),
            size: r.frame_buf.size(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let lights = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pt_lights"),
            size: r.light_buf.size(),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let lut_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pt_sky"),
            size: r.sky_lut_tex.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: r.sky_lut_tex.format(),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let bufs = self.take_buffers(&device, width, height);
        let row_px = width.div_ceil(32) * 32;
        queue.write_buffer(&bufs.resolve_params, 0, bytemuck::cast_slice(&[width, height, row_px, 0u32]));
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&r.frame_buf, 0, &frame, 0, r.frame_buf.size());
        enc.copy_buffer_to_buffer(&r.light_buf, 0, &lights, 0, r.light_buf.size());
        enc.copy_texture_to_texture(r.sky_lut_tex.as_image_copy(), lut_tex.as_image_copy(), r.sky_lut_tex.size());
        enc.clear_buffer(&bufs.accum, 0, None);
        enc.clear_buffer(&bufs.aux, 0, None);
        queue.submit([enc.finish()]);
        let lut_view = lut_tex.create_view(&Default::default());
        let bg0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pt0"),
            layout: &self.bgl0,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: frame.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&lut_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&r.sky_samp) },
                wgpu::BindGroupEntry { binding: 3, resource: lights.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.params_buf, offset: 0, size: wgpu::BufferSize::new(80) }),
                },
            ],
        });
        let bg2 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pt2"),
            layout: &self.bgl2,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: bufs.accum.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: bufs.aux.as_entire_binding() },
            ],
        });
        let resolve_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pt_resolve"),
            layout: &self.resolve_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: bufs.accum.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: bufs.aux.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: bufs.packed.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: bufs.resolve_params.as_entire_binding() },
            ],
        });
        // Angle covered by one pixel, for texture filtering along rays.
        let spread = 2.0 * (1.0 / fp.proj.y_axis.y).atan() / height as f32;
        // Depth of field through a real lens: the photo effect's amount maps
        // to an aperture; focus is the set distance or what's in the centre.
        let lens = if fp.lens[0] > 0.0 {
            let fwd = fp.view.inverse().transform_vector3(Vec3::NEG_Z).normalize();
            let focus = if fp.lens[1] > 0.0 {
                fp.lens[1]
            } else {
                scene.raycast_filtered(fp.cam_pos, fwd, |id| !id.starts_with("env:sky")).map_or(10.0, |h| h.0).clamp(0.3, 500.0)
            };
            [fp.lens[0].clamp(0.0, 1.0) * 0.035, focus]
        } else {
            [0.0, 10.0]
        };
        // Initial batch from what this GPU managed before (~target per pass).
        let mpix = (width * height) as f32 / 1e6;
        let target_ms = 60.0;
        let batch = if self.ms_per_mpix_sample > 0.0 { ((target_ms / (self.ms_per_mpix_sample * mpix)) as u32).clamp(1, 16) } else { 1 };
        PtJob {
            width,
            height,
            opts,
            done_spp: 0,
            submitted_spp: 0,
            bufs: Some(bufs),
            bg0,
            bg2,
            resolve_bg,
            spread,
            lens,
            batch,
            slices: 1,
            slice: 0,
            target_ms,
            centre_rays: false,
            seed: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos()) & 0xffff,
            started: Instant::now(),
            in_flight: VecDeque::new(),
            preview: None,
            last_retire: Instant::now(),
            pass_counter: 0,
            last_pass_ms: 0.0,
        }
    }

    /// Queue one pass (every tile, `batch` samples per pixel each).
    fn submit_pass(&mut self, r: &Renderer, job: &mut PtJob) {
        let Some(bg1) = &self.bg1 else { return };
        let tiles_x = job.width.div_ceil(TILE);
        let tiles_y = job.height.div_ceil(TILE);
        let mut tiles: Vec<(u32, u32)> = (0..tiles_y).flat_map(|y| (0..tiles_x).map(move |x| (x, y))).collect();
        let n_tiles = tiles.len();
        if job.slice == 0 {
            // Start of a round: decide how many passes one sample takes.
            let full_ms = self.ms_per_mpix_sample * (job.width * job.height) as f32 / 1e6;
            job.slices = if self.ms_per_mpix_sample > 0.0 && full_ms > job.target_ms * 1.25 {
                ((full_ms / job.target_ms).ceil() as u32).clamp(1, n_tiles as u32)
            } else {
                1
            };
        }
        let slices = job.slices;
        if slices > 1 {
            // Interleaved tiles so each slice costs about the same.
            let k = job.slice as usize;
            tiles = tiles.into_iter().enumerate().filter(|(i, _)| i % slices as usize == k).map(|(_, t)| t).collect();
        }
        let frac = tiles.len() as f32 / n_tiles.max(1) as f32;
        let batch = if slices > 1 { 1 } else { job.batch.min(job.opts.spp - job.submitted_spp).max(1) };
        let (portal_base, portal_count, portal_area) =
            self.geom.as_ref().filter(|_| self.window_sampling).map_or((0, 0, 0.0), |g| (g.portal_base, g.portal_count, g.portal_area));
        let mut enc = r.device.create_command_encoder(&Default::default());
        let mut ts_slot = None;
        if let Some(ts) = &mut self.timestamps {
            let s = ts.next;
            if ts.busy[s].is_none() {
                ts_slot = Some(s);
                ts.next = (s + 1) % TS_SLOTS;
            }
        }
        // More than MAX_TILES tiles (8K+) needs several parameter uploads,
        // i.e. several submissions; the common case is one.
        let chunks: Vec<&[(u32, u32)]> = tiles.chunks(MAX_TILES).collect();
        let n_chunks = chunks.len();
        let mut index = None;
        for (ci, chunk) in chunks.into_iter().enumerate() {
            let params: Vec<Params> = chunk
                .iter()
                .map(|&(tx, ty)| Params {
                    size: [job.width, job.height, job.submitted_spp.wrapping_mul(7919).wrapping_add(job.seed), job.submitted_spp],
                    tile: [tx * TILE, ty * TILE, TILE.min(job.width - tx * TILE), TILE.min(job.height - ty * TILE)],
                    opts: [job.opts.bounces as f32, 24.0, job.spread, portal_area],
                    lens: [job.lens[0], job.lens[1], batch as f32, if job.centre_rays { 1.0 } else { 0.0 }],
                    portal: [portal_base, portal_count, 0, 0],
                    _pad: [[0.0; 4]; 11],
                })
                .collect();
            r.queue.write_buffer(&self.params_buf, 0, bytemuck::cast_slice(&params));
            {
                let tw = self.timestamps.as_ref().zip(ts_slot).map(|(ts, s)| wgpu::ComputePassTimestampWrites {
                    query_set: &ts.set,
                    beginning_of_pass_write_index: (ci == 0).then_some((s * 2) as u32),
                    end_of_pass_write_index: (ci + 1 == n_chunks).then_some((s * 2 + 1) as u32),
                });
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("pathtrace"), timestamp_writes: tw });
                pass.set_pipeline(&self.pipe);
                pass.set_bind_group(1, bg1, &[]);
                pass.set_bind_group(2, &job.bg2, &[]);
                for (i, p) in params.iter().enumerate() {
                    pass.set_bind_group(0, &job.bg0, &[(i * std::mem::size_of::<Params>()) as u32]);
                    pass.dispatch_workgroups(p.tile[2].div_ceil(8), p.tile[3].div_ceil(8), 1);
                }
            }
            if ci + 1 == n_chunks {
                if let (Some(ts), Some(s)) = (&self.timestamps, ts_slot) {
                    let off = s as u64 * wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;
                    enc.resolve_query_set(&ts.set, (s * 2) as u32..(s * 2 + 2) as u32, &ts.resolve, off);
                    enc.copy_buffer_to_buffer(&ts.resolve, off, &ts.read[s], 0, 16);
                }
            }
            let cmd = std::mem::replace(&mut enc, r.device.create_command_encoder(&Default::default())).finish();
            let busy = !job.in_flight.is_empty();
            let t = Instant::now();
            index = Some(r.queue.submit([cmd]));
            if busy && self.max_in_flight > 1 && t.elapsed() > Duration::from_millis(30) {
                log::info!("GPU driver blocks on queued work: keeping one path-tracing pass in flight");
                self.max_in_flight = 1;
            }
        }
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        r.queue.on_submitted_work_done(move || flag.store(true, Ordering::Release));
        if let (Some(ts), Some(s)) = (&mut self.timestamps, ts_slot) {
            ts.period_ns = r.queue.get_timestamp_period();
            ts.busy[s] = Some(Arc::new(AtomicBool::new(false)));
        }
        let last_slice = job.slice + 1 >= slices;
        let spp = if last_slice { batch } else { 0 };
        if last_slice {
            job.submitted_spp += batch;
            job.slice = 0;
        } else {
            job.slice += 1;
        }
        job.pass_counter += 1;
        job.in_flight.push_back(Pass { index: index.expect("one submission"), spp, work: batch as f32 * frac, done, submitted: Instant::now(), ts_slot });
    }

    /// Account for passes the GPU has finished and retune the batch size.
    fn retire(&mut self, r: &Renderer, job: &mut PtJob) {
        while let Some(p) = job.in_flight.front() {
            if !p.done.load(Ordering::Acquire) {
                break;
            }
            let p = job.in_flight.pop_front().unwrap();
            let now = Instant::now();
            // Wall-clock estimate (exact when we waited for it).
            let mut ms = now.duration_since(p.submitted.max(job.last_retire)).as_secs_f32() * 1000.0;
            job.last_retire = now;
            if let (Some(ts), Some(s)) = (&mut self.timestamps, p.ts_slot) {
                // The pass (and its timestamp copy) completed, so mapping the
                // tiny read buffer resolves on this poll.
                let mapped = Arc::new(AtomicBool::new(false));
                let m2 = mapped.clone();
                ts.read[s].slice(..).map_async(wgpu::MapMode::Read, move |res| m2.store(res.is_ok(), Ordering::Release));
                r.device.poll(wgpu::Maintain::Poll);
                if mapped.load(Ordering::Acquire) {
                    let v: [u64; 2] = {
                        let data = ts.read[s].slice(..).get_mapped_range();
                        let mut v = [0u64; 2];
                        v[0] = u64::from_le_bytes(data[0..8].try_into().unwrap());
                        v[1] = u64::from_le_bytes(data[8..16].try_into().unwrap());
                        v
                    };
                    ts.read[s].unmap();
                    if v[1] > v[0] {
                        ms = (v[1] - v[0]) as f32 * ts.period_ns / 1e6;
                    }
                    ts.busy[s] = None;
                } else {
                    // Still mapping: unmap on the next retire of this slot is
                    // not possible without its callback, so wait for it now.
                    r.device.poll(wgpu::Maintain::Wait);
                    if mapped.load(Ordering::Acquire) {
                        ts.read[s].unmap();
                    }
                    ts.busy[s] = None;
                }
            }
            job.done_spp += p.spp;
            job.last_pass_ms = ms;
            if ms > 0.0 {
                let mpix = (job.width * job.height) as f32 / 1e6;
                // ms for one sample over the whole image.
                let per_sample = ms / p.work.max(1e-3);
                self.ms_per_mpix_sample = per_sample / mpix.max(1e-3);
                // Aim for target_ms per pass, keep each dispatch short, grow
                // at most 2x per pass to stay stable.
                let tiles = (job.width.div_ceil(TILE) * job.height.div_ceil(TILE)).max(1) as f32;
                let by_target = job.target_ms / per_sample.max(1e-4);
                let by_dispatch = MAX_DISPATCH_MS * tiles / per_sample.max(1e-4);
                let want = by_target.min(by_dispatch).clamp(1.0, 64.0) as u32;
                job.batch = want.min(job.batch * 2).max(1);
            }
        }
    }

    /// Trace for about `budget`: keep up to two passes queued, retire the
    /// finished ones, and return - work keeps running on the GPU after this
    /// returns, so callers (the UI) stay responsive. Waits for a pass only
    /// when it is expected to finish within the remaining budget.
    pub fn step(&mut self, r: &Renderer, job: &mut PtJob, budget: Duration) {
        let deadline = Instant::now() + budget;
        loop {
            r.device.poll(wgpu::Maintain::Poll);
            self.retire(r, job);
            if job.finished() {
                break;
            }
            if job.in_flight.len() < self.max_in_flight && job.submitted_spp < job.opts.spp {
                self.submit_pass(r, job);
                continue;
            }
            let Some(front) = job.in_flight.front() else { break };
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let remaining = (deadline - now).as_secs_f32() * 1000.0;
            let expected = job.last_pass_ms - now.duration_since(front.submitted).as_secs_f32() * 1000.0;
            if job.last_pass_ms > 0.0 && expected > remaining {
                break;
            }
            r.device.poll(wgpu::Maintain::WaitForSubmissionIndex(front.index.clone()));
        }
    }

    /// Wait for everything queued for this job.
    fn drain(&mut self, r: &Renderer, job: &mut PtJob) {
        if let Some(last) = job.in_flight.back() {
            r.device.poll(wgpu::Maintain::WaitForSubmissionIndex(last.index.clone()));
        }
        r.device.poll(wgpu::Maintain::Poll);
        self.retire(r, job);
    }

    /// Run the GPU resolve and read back the packed fp16 planes
    /// (colour, albedo+depth, normal). Returns (bytes, row stride in pixels).
    fn read_packed(&mut self, r: &Renderer, job: &mut PtJob) -> (Vec<u8>, usize) {
        // A blocking read supersedes any asynchronous preview in progress.
        if let (Some(PreviewState::Reading { .. }), Some(b)) = (job.preview.take(), &job.bufs) {
            b.readback.unmap();
        }
        self.drain(r, job);
        let bufs = job.bufs.as_ref().expect("job buffers");
        let mut enc = r.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("pt_resolve"), timestamp_writes: None });
            pass.set_pipeline(&self.resolve_pipe);
            pass.set_bind_group(0, &job.resolve_bg, &[]);
            pass.dispatch_workgroups(job.width.div_ceil(8), job.height.div_ceil(8), 1);
        }
        enc.copy_buffer_to_buffer(&bufs.packed, 0, &bufs.readback, 0, bufs.packed.size());
        let idx = r.queue.submit([enc.finish()]);
        let slice = bufs.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        r.device.poll(wgpu::Maintain::WaitForSubmissionIndex(idx));
        let data = slice.get_mapped_range().to_vec();
        bufs.readback.unmap();
        (data, job.width.div_ceil(32) as usize * 32)
    }

    /// Averaged per-pixel layers: (colour, albedo, normal, first-hit distance).
    pub fn read_layers(&mut self, r: &Renderer, job: &mut PtJob) -> PixelLayers {
        let (data, row_px) = self.read_packed(r, job);
        let (w, h) = (job.width as usize, job.height as usize);
        let plane = row_px * h * 8;
        let px = |p: usize, x: usize, y: usize| -> [f32; 4] {
            let o = p * plane + (y * row_px + x) * 8;
            [0, 2, 4, 6].map(|k| denoise::f16_to_f32(u16::from_le_bytes([data[o + k], data[o + k + 1]])))
        };
        let (mut c, mut a, mut n, mut d) = (Vec::with_capacity(w * h), Vec::with_capacity(w * h), Vec::with_capacity(w * h), Vec::with_capacity(w * h));
        for y in 0..h {
            for x in 0..w {
                let (cc, aa, nn) = (px(0, x, y), px(1, x, y), px(2, x, y));
                c.push([cc[0], cc[1], cc[2]]);
                a.push([aa[0], aa[1], aa[2]]);
                d.push(aa[3]);
                n.push([nn[0], nn[1], nn[2]]);
            }
        }
        (c, a, n, d)
    }

    /// Denoised (if enabled) HDR colour plane, RGBA16F with 256-byte rows.
    fn hdr_plane(&mut self, r: &Renderer, job: &mut PtJob) -> (Vec<u8>, Option<denoise::Method>) {
        let (data, row_px) = self.read_packed(r, job);
        let plane = row_px * job.height as usize * 8;
        if !job.opts.denoise {
            return (data[..plane].to_vec(), None);
        }
        let p = denoise::Packed {
            width: job.width as usize,
            height: job.height as usize,
            row_px,
            color: &data[..plane],
            albedo: &data[plane..plane * 2],
            normal: &data[plane * 2..plane * 3],
        };
        // Previews use the faster network until the job is complete.
        let (out, m) = denoise::denoise_packed(&p, job.finished(), job.lens[0] > 0.0);
        (out, Some(m))
    }

    /// Read back the samples and denoise. Returns the HDR image in linear
    /// RGB and which denoiser ran.
    pub fn resolve(&mut self, r: &Renderer, job: &mut PtJob) -> (Vec<[f32; 3]>, Option<denoise::Method>) {
        let (plane, m) = self.hdr_plane(r, job);
        let row_px = job.width.div_ceil(32) as usize * 32;
        let mut out = Vec::with_capacity((job.width * job.height) as usize);
        for y in 0..job.height as usize {
            for x in 0..job.width as usize {
                let o = (y * row_px + x) * 8;
                out.push([0, 2, 4].map(|k| denoise::f16_to_f32(u16::from_le_bytes([plane[o + k], plane[o + k + 1]]))));
            }
        }
        (out, m)
    }

    /// Whether an asynchronous preview update is in progress.
    pub fn preview_pending(&self, job: &PtJob) -> bool {
        job.preview.is_some()
    }

    /// Start capturing the samples traced so far for display, without
    /// waiting for anything: the resolve and copy are queued behind the
    /// passes already submitted, and the read-back buffer is mapped
    /// asynchronously. Returns false if an update is already in progress.
    pub fn request_preview(&mut self, r: &Renderer, job: &mut PtJob) -> bool {
        if job.preview.is_some() {
            return false;
        }
        let bufs = job.bufs.as_ref().expect("job buffers");
        let mut enc = r.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("pt_resolve"), timestamp_writes: None });
            pass.set_pipeline(&self.resolve_pipe);
            pass.set_bind_group(0, &job.resolve_bg, &[]);
            pass.dispatch_workgroups(job.width.div_ceil(8), job.height.div_ceil(8), 1);
        }
        enc.copy_buffer_to_buffer(&bufs.packed, 0, &bufs.readback, 0, bufs.packed.size());
        r.queue.submit([enc.finish()]);
        let mapped = Arc::new(AtomicBool::new(false));
        let m2 = mapped.clone();
        bufs.readback.slice(..).map_async(wgpu::MapMode::Read, move |res| m2.store(res.is_ok(), Ordering::Release));
        job.preview = Some(PreviewState::Reading { mapped, spp: job.submitted_spp, t0: Instant::now() });
        true
    }

    /// Advance an asynchronous preview update; never blocks. Returns the
    /// image once it has been read back and denoised.
    pub fn poll_preview(&mut self, r: &Renderer, job: &mut PtJob) -> Option<PreviewImage> {
        r.device.poll(wgpu::Maintain::Poll);
        match job.preview.take()? {
            PreviewState::Reading { mapped, spp, t0 } => {
                if !mapped.load(Ordering::Acquire) {
                    job.preview = Some(PreviewState::Reading { mapped, spp, t0 });
                    return None;
                }
                let bufs = job.bufs.as_ref().expect("job buffers");
                let data = bufs.readback.slice(..).get_mapped_range().to_vec();
                bufs.readback.unmap();
                let read_ms = t0.elapsed().as_secs_f32() * 1000.0;
                let (w, h) = (job.width as usize, job.height as usize);
                let row_px = job.width.div_ceil(32) as usize * 32;
                let denoise_on = job.opts.denoise;
                let final_quality = job.finished() && spp >= job.opts.spp;
                let noisy_aux = job.lens[0] > 0.0;
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let t = Instant::now();
                    let plane = row_px * h * 8;
                    let out = if denoise_on {
                        let p = denoise::Packed {
                            width: w,
                            height: h,
                            row_px,
                            color: &data[..plane],
                            albedo: &data[plane..plane * 2],
                            normal: &data[plane * 2..plane * 3],
                        };
                        let (o, m) = denoise::denoise_packed(&p, final_quality, noisy_aux);
                        (o, Some(m))
                    } else {
                        (data[..plane].to_vec(), None)
                    };
                    let _ = tx.send((out.0, out.1, t.elapsed().as_secs_f32() * 1000.0));
                });
                job.preview = Some(PreviewState::Denoising { spp, rx, t0, read_ms });
                None
            }
            PreviewState::Denoising { spp, rx, t0, read_ms } => match rx.try_recv() {
                Ok((hdr, method, denoise_ms)) => Some(PreviewImage { hdr, spp, method, read_ms, denoise_ms, latency_ms: t0.elapsed().as_secs_f32() * 1000.0 }),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    job.preview = Some(PreviewState::Denoising { spp, rx, t0, read_ms });
                    None
                }
                Err(_) => None, // worker panicked: drop this update
            },
        }
    }

    /// Frame parameters for post-processing a path-traced image.
    pub fn post_params(job: &PtJob, fp: &FrameParams) -> FrameParams {
        let mut fp = fp.clone();
        fp.ao = 0.0; // the path tracer computed real occlusion
        fp.ssr = 0.0;
        if job.lens[0] > 0.0 {
            fp.lens[0] = 0.0; // depth of field already came from the lens
        }
        fp
    }

    /// Denoise and run the renderer's post chain on the result into `targets`.
    pub fn compose(&mut self, r: &mut Renderer, scene: &Scene, fp: &FrameParams, targets: &Targets, job: &mut PtJob) -> Option<denoise::Method> {
        let (plane, method) = self.hdr_plane(r, job);
        r.set_hdr_override_packed(job.width, job.height, &plane);
        let mut fp = fp.clone();
        fp.ao = 0.0; // the path tracer computed real occlusion
        fp.ssr = 0.0;
        if job.lens[0] > 0.0 {
            fp.lens[0] = 0.0; // depth of field already came from the lens
        }
        r.render(scene, &fp, targets);
        method
    }

    /// Finish: denoise, post-process and read back the final image.
    pub fn finish(
        &mut self,
        r: &mut Renderer,
        scene: &Scene,
        fp: &FrameParams,
        targets: &Targets,
        job: &mut PtJob,
    ) -> (image::RgbaImage, Option<denoise::Method>) {
        let method = self.compose(r, scene, fp, targets, job);
        (r.read_output(targets), method)
    }
}

/// Full-quality ray-traced still (blocking).
#[allow(clippy::too_many_arguments)]
pub fn render_still(
    r: &mut Renderer,
    pt: &mut PathTracer,
    scene: &Scene,
    fp: &FrameParams,
    w: u32,
    h: u32,
    opts: PtOpts,
    mut progress: impl FnMut(f32) -> bool,
) -> Option<(image::RgbaImage, Option<denoise::Method>)> {
    if let Err(e) = check_resolution(&r.device.limits(), w, h, true) {
        log::error!("{e}");
        return None;
    }
    let targets = r.create_targets(w, h);
    r.settle = true;
    let mut job = match pt.try_begin(r, scene, fp, &targets, opts) {
        Ok(j) => j,
        Err(e) => {
            log::error!("{e}");
            return None;
        }
    };
    job.target_ms = 250.0; // nothing else is drawing: longer passes, less overhead
    while !job.finished() {
        pt.step(r, &mut job, Duration::from_millis(250));
        if !progress(job.progress()) {
            pt.recycle(job);
            return None;
        }
    }
    let out = pt.finish(r, scene, fp, &targets, &mut job);
    pt.recycle(job);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_conversion() {
        assert_eq!(f16_bits(0.0), 0);
        assert_eq!(f16_bits(1.0), 0x3c00);
        assert_eq!(f16_bits(-2.0), 0xc000);
        assert_eq!(f16_bits(0.5), 0x3800);
        assert_eq!(f16_bits(1e9), 0x7bef); // clamped to 65000
    }

    #[test]
    fn oct_roundtrip() {
        for n in [Vec3::X, Vec3::NEG_Y, Vec3::new(0.3, -0.5, -0.8).normalize()] {
            let e = oct_encode(n);
            let mut v = Vec3::new(e[0], e[1], 1.0 - e[0].abs() - e[1].abs());
            if v.z < 0.0 {
                let s = |x: f32| if x >= 0.0 { 1.0 } else { -1.0 };
                v = Vec3::new((1.0 - v.y.abs()) * s(v.x), (1.0 - v.x.abs()) * s(v.y), v.z);
            }
            assert!(v.normalize().dot(n) > 0.999);
        }
    }

    #[test]
    fn texture_classes_follow_native_size() {
        let t = |w, h| TextureData::new(w, h, vec![0; (w * h * 4) as usize]);
        assert_eq!(native_class(&t(100, 64)), 0);
        assert_eq!(native_class(&t(300, 200)), 2);
        assert_eq!(native_class(&t(1024, 1024)), 3);
        assert_eq!(native_class(&t(2048, 1024)), 4);
        assert_eq!(native_class(&t(4096, 4096)), 5);
    }

    #[test]
    fn budget_demotes_only_the_largest_textures() {
        // classes: 0=128 1=256 2=512 3=1K 4=2K 5=4K
        let mut native = vec![2usize; 120];
        native.extend(vec![3usize; 80]);
        native.extend(vec![5usize; 10]);
        let data = vec![false; native.len()];
        let p = plan_classes(&native, &data, 1 << 30, 256);
        assert!(p.class[..200].iter().zip(&native).all(|(c, n)| *c == Some(*n)), "small textures untouched");
        assert!(p.demoted > 0 && p.dropped == 0);
        assert!(p.bytes <= 1 << 30);
        let p = plan_classes(&native, &data, 8 << 30, 256);
        assert_eq!((p.demoted, p.dropped), (0, 0));
    }

    #[test]
    fn budget_is_enforced_below_512_and_by_dropping() {
        // 400 2K maps into 64 MB: everything has to go far below 512 px.
        let native = vec![4usize; 400];
        let data: Vec<bool> = (0..400).map(|i| i % 2 == 1).collect();
        let p = plan_classes(&native, &data, 64 << 20, 1024);
        assert!(p.bytes <= 64 << 20, "budget exceeded: {}", p.bytes);
        assert!(p.class.iter().flatten().all(|&c| c <= 1), "maps must shrink below 512 px");
        assert_eq!(p.dropped, 0);
        // A budget too small even for 128 px copies drops data maps first.
        let p = plan_classes(&native, &data, 8 << 20, 1024);
        assert!(p.bytes <= 8 << 20);
        assert!(p.dropped > 0);
        let dropped_color = p.class.iter().zip(&data).filter(|(c, d)| c.is_none() && !**d).count();
        let kept_data = p.class.iter().zip(&data).filter(|(c, d)| c.is_some() && **d).count();
        assert!(dropped_color == 0 || kept_data == 0, "colour maps dropped while data maps remain");
        // Zero budget: nothing allocated.
        let p = plan_classes(&native, &data, 0, 1024);
        assert_eq!(p.bytes, 0);
        assert_eq!(p.dropped, 400);
    }

    #[test]
    fn layer_limit_pushes_overflow_down_a_class() {
        let native = vec![4usize; 5];
        let p = plan_classes(&native, &[false; 5], u64::MAX, 3);
        assert_eq!(p.class.iter().filter(|&&c| c == Some(4)).count(), 3);
        assert_eq!(p.class.iter().filter(|&&c| c == Some(3)).count(), 2);
        assert_eq!(p.demoted, 2);
    }

    #[test]
    fn resolution_check_offers_the_largest_size_that_fits() {
        let mut l = wgpu::Limits { max_storage_buffer_binding_size: 128 << 20, max_buffer_size: 2 << 30, max_texture_dimension_2d: 8192, ..Default::default() };
        // 4K path traced needs a 265 MB layer buffer: refused, with a reason.
        let e = check_resolution(&l, 3840, 2160, true).unwrap_err();
        assert!(e.reason.contains("MB"), "{}", e.reason);
        let (w, h) = e.max_supported;
        assert!(check_resolution(&l, w, h, true).is_ok());
        assert!(check_resolution(&l, w + 64, h + 36, true).is_err(), "not the largest: {w}x{h}");
        assert!((w as f32 / h as f32 - 16.0 / 9.0).abs() < 0.01, "aspect kept: {w}x{h}");
        // Rasterized photos don't need those buffers.
        assert!(check_resolution(&l, 3840, 2160, false).is_ok());
        // Larger than the largest texture.
        let e = check_resolution(&l, 10000, 5000, false).unwrap_err();
        assert!(e.max_supported.0 <= 8192);
        // 1 GiB binding limit: 8K UHD path traced (1.06 GB layer buffer) just
        // fits; 8192 x 4608 (1.21 GB) doesn't.
        l.max_storage_buffer_binding_size = 1 << 30;
        assert!(check_resolution(&l, 7680, 4320, true).is_ok());
        assert!(check_resolution(&l, 8192, 4608, true).is_err());
    }

    #[test]
    fn presets_increase() {
        assert!(PtOpts::preset(0).spp < PtOpts::preset(1).spp);
        assert!(PtOpts::preset(2).spp < PtOpts::preset(3).spp);
    }
}
