//! wgpu renderer: atmosphere LUT -> cascaded shadow passes -> opaque + sky
//! (MSAA HDR, reverse-Z) -> depth copy -> reflections + transparent (SSR)
//! -> bloom -> auto exposure -> SSAO, depth of field, lens + tonemap.

use crate::scene::{MaterialKind, MaterialParams, Scene, TextureData};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::collections::HashMap;
use std::sync::Arc;
use wgpu::util::DeviceExt;

pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const OUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const SAMPLES: u32 = 4;
pub const CASCADES: usize = 4;
pub const SHADOW_SIZE: u32 = 2048;
pub const MAX_LIGHTS: usize = 256;
const SKY_LUT: (u32, u32) = (192, 96);
const LIN_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 3],
    nrm: [f32; 3],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameU {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    light_vp: [[[f32; 4]; 4]; CASCADES],
    cascade_far: [f32; 4],
    cascade_texel: [f32; 4],
    cam_pos: [f32; 4],
    cam_fwd: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sky: [f32; 4],
    post: [f32; 4],
    misc: [f32; 4],
    extra: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default, PartialEq)]
pub struct LightU {
    pub pos_range: [f32; 4],
    pub color: [f32; 4],
    pub dir: [f32; 4],
}

impl LightU {
    pub fn point(pos: Vec3, color: Vec3, intensity: f32, range: f32) -> Self {
        let c = color * intensity;
        Self { pos_range: [pos.x, pos.y, pos.z, range], color: [c.x, c.y, c.z, -2.0], dir: [0.0, -1.0, 0.0, 1.0] }
    }
    pub fn spot(pos: Vec3, dir: Vec3, color: Vec3, intensity: f32, range: f32, outer_deg: f32) -> Self {
        let c = color * intensity;
        let d = dir.normalize_or_zero();
        let outer = (outer_deg * 0.5).to_radians().cos();
        let inner = (outer_deg * 0.5 * 0.7).to_radians().cos();
        Self { pos_range: [pos.x, pos.y, pos.z, range], color: [c.x, c.y, c.z, outer], dir: [d.x, d.y, d.z, inner] }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialU {
    base: [f32; 4],
    emissive: [f32; 4], // rgb * strength, normal-map strength
    params: [f32; 4],   // metallic, roughness, kind, uv scale
    extra: [f32; 4],    // variation, has colour map, has normal map, cutout
    uvt: [f32; 4],      // uv scale.xy, offset.xy from the source file
    extra2: [f32; 4],   // has roughness map, uv rotation (rad), uv offset
    extra3: [f32; 4],   // texture influence, reflectivity, anti-tiling, highlight (1 hover, 2 selected)
    extra4: [f32; 4],   // real-world box mapping, unused x3
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostU {
    params: [f32; 4],
    extra: [f32; 4],
    quality: [f32; 4], // ssao samples, fxaa, time, auto exposure
    lens: [f32; 4],    // dof amount, focus distance, chromatic aberration, grain
    proj: [[f32; 4]; 4],
    inv_proj: [[f32; 4]; 4],
    tone: [f32; 4], // filmic (AgX) on, unused x3
}

/// Everything the renderer needs to know about the current view & environment.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameParams {
    pub view: Mat4,
    pub proj: Mat4,
    pub cam_pos: Vec3,
    pub time: f32,
    pub sun_dir: Vec3,
    pub sun_color: Vec3,
    pub sun_intensity: f32,
    pub ambient: f32,
    pub cloudiness: f32,
    pub fog: f32,
    pub haze: f32,
    pub shadow_softness: f32,
    pub wind: f32,
    pub reflections: f32,
    pub exposure: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub vignette: f32,
    pub bloom: f32,
    pub sharpen: f32,
    pub ao: f32,
    pub ao_radius: f32,
    /// Screen-space reflection strength (0 = off).
    pub ssr: f32,
    /// Depth of field amount, focus distance (0 = auto), chromatic aberration, film grain.
    pub lens: [f32; 4],
    pub auto_exposure: bool,
    /// Filmic (AgX) tone mapping instead of ACES.
    pub filmic: bool,
    pub lights: Vec<LightU>,
}

struct GpuMesh {
    rev: u64,
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    count: u32,
    material: String,
    center: Vec3,
    lo: Vec3,
    hi: Vec3,
    /// The terrain can't shadow anything above it; skipping it saves filling
    /// every shadow map with ground pixels.
    casts_shadow: bool,
}

/// Frustum as 6 planes (xyz normal, w distance), positive = inside.
struct Frustum([glam::Vec4; 6]);

impl Frustum {
    fn from_matrix(m: Mat4) -> Self {
        let (r0, r1, r2, r3) = (m.row(0), m.row(1), m.row(2), m.row(3));
        let mut p = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2];
        for pl in &mut p {
            let l = pl.truncate().length();
            if l > 0.0 {
                *pl /= l;
            }
        }
        Frustum(p)
    }
    fn visible(&self, lo: Vec3, hi: Vec3) -> bool {
        self.0.iter().all(|pl| {
            let n = pl.truncate();
            let pv = Vec3::new(if n.x >= 0.0 { hi.x } else { lo.x }, if n.y >= 0.0 { hi.y } else { lo.y }, if n.z >= 0.0 { hi.z } else { lo.z });
            n.dot(pv) + pl.w >= 0.0
        })
    }
}

struct GpuMaterial {
    buf: wgpu::Buffer,
    bg: wgpu::BindGroup,
    tex_key: (u64, u64, u64),
    kind: MaterialKind,
    cutout: bool,
    invisible: bool,
    glossy: bool,
    uniform: MaterialU,
}

/// Size-dependent render targets (one set for the viewport, one per photo render).
pub struct Targets {
    pub size: (u32, u32),
    pub samples: u32,
    hdr_msaa: Option<wgpu::TextureView>,
    depth: wgpu::TextureView,
    pub output: wgpu::Texture,
    pub output_view: wgpu::TextureView,
    post_bg: wgpu::BindGroup,
    resolve_view: wgpu::TextureView,
    bloom_a: wgpu::TextureView,
    bloom_b: wgpu::TextureView,
    bright_bg: wgpu::BindGroup,
    blur_h_bg: wgpu::BindGroup,
    blur_v_bg: wgpu::BindGroup,
    hdr_resolve_tex: wgpu::Texture,
    scene_color: wgpu::Texture,
    scene_color_view: wgpu::TextureView,
    lin_depth_view: wgpu::TextureView,
    exposure_view: wgpu::TextureView,
    ssr_bg: wgpu::BindGroup,
    depth_copy_bg: wgpu::BindGroup,
    exposure_bg: wgpu::BindGroup,
}

pub struct Renderer {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub(crate) frame_buf: wgpu::Buffer,
    post_buf: wgpu::Buffer,
    frame_bg: wgpu::BindGroup,
    shadow_layers: Vec<wgpu::TextureView>,
    shadow_size: u32,
    frame_bgl: wgpu::BindGroupLayout,
    cmp_sampler: wgpu::Sampler,
    shadow_cache: [Option<CascadeCache>; CASCADES],
    frame_no: u64,
    geom_version: u64,
    look_version: u64,
    highlight: (Option<String>, Option<String>),
    last_overrides: std::collections::BTreeMap<String, MaterialParams>,
    shadow_ubufs: Vec<wgpu::Buffer>,
    shadow_bgs: Vec<wgpu::BindGroup>,
    pub(crate) light_buf: wgpu::Buffer,
    pub stats: RenderStats,
    pub quality: Quality,
    /// Camera at rest: bring every shadow cascade fully up to date.
    pub settle: bool,
    mat_bgl: wgpu::BindGroupLayout,
    post_bgl: wgpu::BindGroupLayout,
    ssr_bgl: wgpu::BindGroupLayout,
    depth_copy_bgls: [wgpu::BindGroupLayout; 2],
    depth_copy_pipes: [wgpu::RenderPipeline; 2],
    exposure_bgl: wgpu::BindGroupLayout,
    exposure_pipe: wgpu::RenderPipeline,
    overlay_pipes: [wgpu::RenderPipeline; 2],
    sky_lut_pipe: wgpu::RenderPipeline,
    sky_lut_bg: wgpu::BindGroup,
    sky_lut_view: wgpu::TextureView,
    pub(crate) sky_lut_tex: wgpu::Texture,
    pub(crate) sky_samp: wgpu::Sampler,
    /// A finished HDR image (e.g. from the path tracer) that replaces the
    /// rasterized one for the next frame, before bloom/exposure/post.
    hdr_override: Option<(u32, u32, wgpu::Buffer)>,
    /// Texture bytes written since the last submission (see upload_texture).
    staged_bytes: u64,
    sky_pipe_ss: wgpu::RenderPipeline,
    opaque_pipe_ss: wgpu::RenderPipeline,
    transparent_pipe_ss: wgpu::RenderPipeline,
    /// [no MSAA, MSAA]
    depth_pipes: [wgpu::RenderPipeline; 2],
    opaque_eq_pipes: [wgpu::RenderPipeline; 2],
    bloom_bgl: wgpu::BindGroupLayout,
    bright_pipe: wgpu::RenderPipeline,
    blur_pipe: wgpu::RenderPipeline,
    sky_pipe: wgpu::RenderPipeline,
    opaque_pipe: wgpu::RenderPipeline,
    transparent_pipe: wgpu::RenderPipeline,
    shadow_pipe: wgpu::RenderPipeline,
    shadow_cut_pipe: wgpu::RenderPipeline,
    post_pipe: wgpu::RenderPipeline,
    linear_sampler: wgpu::Sampler,
    repeat_sampler: wgpu::Sampler,
    white_view: wgpu::TextureView,
    flat_normal_view: wgpu::TextureView,
    textures: HashMap<u64, wgpu::TextureView>,
    /// Viewport texture budget (bytes) and the largest side textures are
    /// currently stored at to stay inside it.
    tex_budget: u64,
    tex_max_side: u32,
    tex_stats: TextureStats,
    meshes: HashMap<String, GpuMesh>,
    materials: HashMap<String, GpuMaterial>,
    last_material_rev: u64,
}

/// Default viewport texture budget; `LUMENCRAFT_TEXTURE_MB` or the
/// Settings slider override it.
pub const DEFAULT_VIEWPORT_TEXTURE_MB: u64 = 2048;
const MAX_SIDE_UNLIMITED: u32 = 16384;

/// Viewport texture memory, for the UI.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextureStats {
    /// Bytes the maps would need at full resolution (with mips).
    pub native_bytes: u64,
    /// Bytes actually stored.
    pub bytes: u64,
    pub budget: u64,
    /// Maps stored smaller than their own resolution.
    pub reduced: u32,
    pub maps: u32,
    /// Largest side maps are limited to (None = full resolution).
    pub max_side: Option<u32>,
}

impl TextureStats {
    pub fn reduction_note(&self) -> Option<String> {
        let side = self.max_side?;
        (self.reduced > 0).then(|| {
            format!(
                "Viewport: {} of {} texture map(s) limited to {side} px to fit the {} MB texture budget (full resolution would need {} MB)",
                self.reduced,
                self.maps,
                self.budget >> 20,
                self.native_bytes.div_ceil(1 << 20)
            )
        })
    }
}

/// Size of a `w`x`h` map once limited to `max_side` (halving, as the mip chain does).
pub fn capped_size(mut w: u32, mut h: u32, max_side: u32) -> (u32, u32) {
    while w.max(h) > max_side && w.max(h) > 1 {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    (w, h)
}

/// Largest power-of-two side limit (>= 128 px) that keeps these maps
/// inside `budget`; maps already smaller are never touched. If even 128 px
/// copies don't fit, 128 is returned (the smallest the renderer goes).
pub fn fit_max_side(sizes: &[(u32, u32)], budget: u64) -> u32 {
    let total = |cap: u32| -> u64 {
        sizes
            .iter()
            .map(|&(w, h)| {
                let (w, h) = capped_size(w, h, cap);
                crate::mips::chain_bytes(w, h)
            })
            .sum()
    };
    let mut cap = MAX_SIDE_UNLIMITED;
    while cap > 128 && total(cap) > budget {
        cap /= 2;
    }
    cap
}

/// What a cached shadow cascade was rendered with.
#[derive(Clone, Copy, Debug)]
struct CascadeCache {
    vp: Mat4,
    texel: f32,
    rot: Mat4,
    center_ls: glam::Vec2,
    radius: f32,
    sun: Vec3,
    geom: u64,
}

struct CascadeFit {
    vp: Mat4,
    texel: f32,
    rot: Mat4,
    center: Vec3,
    radius: f32,
}

fn make_shadow_maps(device: &wgpu::Device, size: u32) -> (wgpu::TextureView, Vec<wgpu::TextureView>) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow"),
        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: CASCADES as u32 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let array = tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
    let layers = (0..CASCADES as u32)
        .map(|i| {
            tex.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: i,
                array_layer_count: Some(1),
                ..Default::default()
            })
        })
        .collect();
    (array, layers)
}

/// Viewport quality (Lumion's F1–F4 "stars"). Photo renders always use 4.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quality {
    pub level: u8,
    pub shadow_size: u32,
    /// PCF kernel radius in texels (1 = 3x3, 2 = 5x5).
    pub pcf_radius: u32,
    pub ssao_samples: u32,
    pub bloom_passes: u32,
    /// Re-render far shadow cascades every few frames instead of every frame.
    pub stagger_shadows: bool,
    /// 4 = 4x MSAA; 1 = no MSAA (FXAA smooths edges instead).
    pub msaa: u32,
    /// Depth pre-pass so the lighting shader runs once per visible pixel.
    pub prepass: bool,
}

impl Quality {
    pub fn preset(level: u8) -> Self {
        match level {
            1 => Self { level: 1, shadow_size: 1024, pcf_radius: 1, ssao_samples: 0, bloom_passes: 1, stagger_shadows: true, msaa: 1, prepass: true },
            2 => Self { level: 2, shadow_size: 2048, pcf_radius: 1, ssao_samples: 6, bloom_passes: 1, stagger_shadows: true, msaa: 1, prepass: true },
            3 => Self { level: 3, shadow_size: 2048, pcf_radius: 2, ssao_samples: 10, bloom_passes: 2, stagger_shadows: true, msaa: 4, prepass: true },
            _ => Self { level: 4, shadow_size: 4096, pcf_radius: 2, ssao_samples: 16, bloom_passes: 2, stagger_shadows: false, msaa: 4, prepass: true },
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    pub shadow_updates: usize,
    pub drawn: usize,
    pub culled: usize,
    pub lights: usize,
}

impl Renderer {
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        let scene_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/scene.wgsl").into()),
        });
        let shadow_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shadow.wgsl").into()),
        });
        let post_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()),
        });

        let frame_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame"),
            size: std::mem::size_of::<FrameU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let post_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post"),
            size: std::mem::size_of::<PostU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_entry = |binding, vis| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: vis,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let vf = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;

        let frame_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame_bgl"),
            entries: &[
                uniform_entry(0, vf),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shadow_frame_bgl =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("shadow_frame_bgl"), entries: &[uniform_entry(0, vf)] });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let samp_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let mat_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mat_bgl"),
            entries: &[uniform_entry(0, vf), tex_entry(1), samp_entry(2), tex_entry(3), tex_entry(4)],
        });
        let unfilt_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let post_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post_bgl"),
            entries: &[tex_entry(0), samp_entry(1), uniform_entry(2, wgpu::ShaderStages::FRAGMENT), tex_entry(3), unfilt_entry(4), unfilt_entry(5)],
        });
        let ssr_bgl = device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("ssr_bgl"), entries: &[tex_entry(0), unfilt_entry(1), samp_entry(2)] });
        let exposure_bgl =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("exposure_bgl"), entries: &[tex_entry(0), samp_entry(1)] });
        let depth_copy_bgls = [false, true].map(|ms| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("depth_copy_bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: ms,
                    },
                    count: None,
                }],
            })
        });
        let bloom_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom_bgl"),
            entries: &[tex_entry(0), samp_entry(1), uniform_entry(2, wgpu::ShaderStages::FRAGMENT)],
        });

        // Cascaded shadow map (one array layer per cascade)
        let (shadow_array_view, shadow_layers) = make_shadow_maps(&device, SHADOW_SIZE);
        let light_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lights"),
            size: (std::mem::size_of::<LightU>() * MAX_LIGHTS) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let cmp_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow_cmp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear_clamp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let repeat_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("repeat"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });

        let sky_lut_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky_lut"),
            size: wgpu::Extent3d { width: SKY_LUT.0, height: SKY_LUT.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let sky_lut_view = sky_lut_tex.create_view(&Default::default());
        let sky_samp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky_lut_samp"),
            address_mode_u: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let frame_bg = Self::make_frame_bg(&device, &frame_bgl, &frame_buf, &shadow_array_view, &cmp_sampler, &light_buf, &sky_lut_view, &sky_samp);
        let shadow_ubufs: Vec<wgpu::Buffer> = (0..CASCADES)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("shadow_u"),
                    size: 64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let shadow_bgs: Vec<wgpu::BindGroup> = shadow_ubufs
            .iter()
            .map(|b| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("shadow_bg"),
                    layout: &shadow_frame_bgl,
                    entries: &[wgpu::BindGroupEntry { binding: 0, resource: b.as_entire_binding() }],
                })
            })
            .collect();

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
        };
        let shadow_vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3],
        };

        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene_layout"),
            bind_group_layouts: &[&frame_bgl, &mat_bgl],
            push_constant_ranges: &[],
        });
        let sky_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky_layout"),
            bind_group_layouts: &[&frame_bgl],
            push_constant_ranges: &[],
        });
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow_layout"),
            bind_group_layouts: &[&shadow_frame_bgl],
            push_constant_ranges: &[],
        });
        let post_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post_layout"),
            bind_group_layouts: &[&post_bgl],
            push_constant_ranges: &[],
        });
        let scene_b_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene_b_layout"),
            bind_group_layouts: &[&frame_bgl, &mat_bgl, &ssr_bgl],
            push_constant_ranges: &[],
        });

        let msaa_state = |count: u32| wgpu::MultisampleState { count, mask: !0, alpha_to_coverage_enabled: false };
        let prim = wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None, // Revit exports have inconsistent winding.
            ..Default::default()
        };

        let make_sky = |count: u32| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sky"),
                layout: Some(&sky_layout),
                vertex: wgpu::VertexState { module: &scene_sm, entry_point: "vs_sky", buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &scene_sm,
                    entry_point: "fs_sky",
                    targets: &[Some(HDR_FORMAT.into())],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                // Drawn after opaque geometry at depth 0 (reverse-Z far), so the
                // expensive sky/cloud shader only runs on pixels that show sky.
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: false,
                    depth_compare: wgpu::CompareFunction::GreaterEqual,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: msaa_state(count),
                multiview: None,
                cache: None,
            })
        };
        let sky_pipe = make_sky(SAMPLES);
        let sky_pipe_ss = make_sky(1);

        let make_scene_pipe_l = |layout: &wgpu::PipelineLayout,
                                 label,
                                 fs,
                                 blend: Option<wgpu::BlendState>,
                                 write,
                                 count: u32,
                                 compare: wgpu::CompareFunction,
                                 mask: wgpu::ColorWrites| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &scene_sm,
                    entry_point: "vs_main",
                    buffers: std::slice::from_ref(&vertex_layout),
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &scene_sm,
                    entry_point: fs,
                    targets: &[Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend, write_mask: mask })],
                    compilation_options: Default::default(),
                }),
                primitive: prim,
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: write,
                    depth_compare: compare, // reverse-Z: GreaterEqual = closer
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: msaa_state(count),
                multiview: None,
                cache: None,
            })
        };
        let make_scene_pipe_ex = |label, fs, blend: Option<wgpu::BlendState>, write, count: u32, compare: wgpu::CompareFunction, mask: wgpu::ColorWrites| {
            make_scene_pipe_l(&scene_layout, label, fs, blend, write, count, compare, mask)
        };
        let make_scene_pipe =
            |label, fs, blend, write, count| make_scene_pipe_ex(label, fs, blend, write, count, wgpu::CompareFunction::GreaterEqual, wgpu::ColorWrites::ALL);
        let depth_pipes = [1, SAMPLES]
            .map(|n| make_scene_pipe_ex("depth_prepass", "fs_depth", None, true, n, wgpu::CompareFunction::GreaterEqual, wgpu::ColorWrites::empty()));
        let opaque_eq_pipes =
            [1, SAMPLES].map(|n| make_scene_pipe_ex("opaque_eq", "fs_opaque", None, false, n, wgpu::CompareFunction::Equal, wgpu::ColorWrites::ALL));
        let opaque_pipe = make_scene_pipe("opaque", "fs_opaque", None, true, SAMPLES);
        let opaque_pipe_ss = make_scene_pipe("opaque_ss", "fs_opaque", None, true, 1);
        let premul = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let transparent_pipe = make_scene_pipe_l(
            &scene_b_layout,
            "transparent",
            "fs_transparent",
            premul,
            false,
            SAMPLES,
            wgpu::CompareFunction::GreaterEqual,
            wgpu::ColorWrites::ALL,
        );
        let transparent_pipe_ss = make_scene_pipe_l(
            &scene_b_layout,
            "transparent_ss",
            "fs_transparent",
            premul,
            false,
            1,
            wgpu::CompareFunction::GreaterEqual,
            wgpu::ColorWrites::ALL,
        );
        let overlay_pipes = [1, SAMPLES].map(|n| {
            make_scene_pipe_l(&scene_b_layout, "ssr_overlay", "fs_ssr_overlay", premul, false, n, wgpu::CompareFunction::Equal, wgpu::ColorWrites::ALL)
        });

        // Alpha-tested casters (leaves, fences): sample the colour map's alpha.
        let shadow_cut_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow_cut_layout"),
            bind_group_layouts: &[&shadow_frame_bgl, &mat_bgl],
            push_constant_ranges: &[],
        });
        let shadow_cut_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow_cut"),
            layout: Some(&shadow_cut_layout),
            vertex: wgpu::VertexState {
                module: &shadow_sm,
                entry_point: "vs_cut",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 24, shader_location: 1 },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState { module: &shadow_sm, entry_point: "fs_cut", targets: &[], compilation_options: Default::default() }),
            primitive: prim,
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let shadow_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &shadow_sm,
                entry_point: "vs_shadow",
                buffers: &[shadow_vertex_layout],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: prim,
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let make_post = |layout: &wgpu::PipelineLayout, module: &wgpu::ShaderModule| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("post"),
                layout: Some(layout),
                vertex: wgpu::VertexState { module, entry_point: "vs_full", buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: "fs_post",
                    targets: &[Some(OUT_FORMAT.into())],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let post_pipe = make_post(&post_layout, &post_sm);
        let make_full = |label: &str, layout: &wgpu::PipelineLayout, module: &wgpu::ShaderModule, fs: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module, entry_point: "vs_full", buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState { module, entry_point: fs, targets: &[Some(format.into())], compilation_options: Default::default() }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let exposure_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("exposure_layout"),
            bind_group_layouts: &[&exposure_bgl],
            push_constant_ranges: &[],
        });
        let exposure_pipe = make_full("exposure", &exposure_layout, &post_sm, "fs_exposure", HDR_FORMAT);
        let dc_src = include_str!("shaders/depth_copy.wgsl");
        let depth_copy_pipes = [false, true].map(|ms| {
            let src = if ms {
                dc_src.to_string()
            } else {
                dc_src
                    .replace("texture_depth_multisampled_2d", "texture_depth_2d")
                    .replace("i32(textureNumSamples(ms_depth))", "1")
                    .replace("textureLoad(ms_depth, p, i)", "textureLoad(ms_depth, p, 0)")
            };
            let m = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("depth_copy"), source: wgpu::ShaderSource::Wgsl(src.into()) });
            let l = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("depth_copy_layout"),
                bind_group_layouts: &[&depth_copy_bgls[ms as usize]],
                push_constant_ranges: &[],
            });
            make_full("depth_copy", &l, &m, "fs_depth_copy", LIN_DEPTH_FORMAT)
        });
        let sky_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/sky.wgsl").into()),
        });
        let sky_lut_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky_lut_layout"),
            bind_group_layouts: &[&shadow_frame_bgl],
            push_constant_ranges: &[],
        });
        let sky_lut_pipe = make_full("sky_lut", &sky_lut_layout, &sky_sm, "fs_skylut", HDR_FORMAT);
        let sky_lut_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky_lut_bg"),
            layout: &shadow_frame_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: frame_buf.as_entire_binding() }],
        });

        let bloom_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/bloom.wgsl").into()),
        });
        let bloom_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bloom_layout"),
            bind_group_layouts: &[&bloom_bgl],
            push_constant_ranges: &[],
        });
        let make_bloom = |fs: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&bloom_layout),
                vertex: wgpu::VertexState { module: &bloom_sm, entry_point: "vs_full", buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &bloom_sm,
                    entry_point: fs,
                    targets: &[Some(HDR_FORMAT.into())],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let bright_pipe = make_bloom("fs_bright");
        let blur_pipe = make_bloom("fs_blur");

        let white = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("white"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[255, 255, 255, 255],
        );
        let white_view = white.create_view(&Default::default());
        let flat = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("flat_normal"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[128, 128, 255, 255],
        );
        let flat_normal_view = flat.create_view(&Default::default());

        Self {
            device,
            queue,
            frame_buf,
            post_buf,
            frame_bg,
            shadow_layers,
            shadow_size: SHADOW_SIZE,
            frame_bgl,
            cmp_sampler,
            shadow_cache: [None; CASCADES],
            frame_no: 0,
            geom_version: 0,
            look_version: 0,
            highlight: (None, None),
            last_overrides: Default::default(),
            shadow_ubufs,
            shadow_bgs,
            light_buf,
            stats: RenderStats::default(),
            quality: Quality::preset(4),
            settle: false,
            mat_bgl,
            post_bgl,
            ssr_bgl,
            depth_copy_bgls,
            depth_copy_pipes,
            exposure_bgl,
            exposure_pipe,
            overlay_pipes,
            sky_lut_pipe,
            sky_lut_bg,
            sky_lut_view,
            sky_lut_tex,
            hdr_override: None,
            staged_bytes: 0,
            sky_samp,
            sky_pipe_ss,
            opaque_pipe_ss,
            transparent_pipe_ss,
            depth_pipes,
            opaque_eq_pipes,
            bloom_bgl,
            bright_pipe,
            blur_pipe,
            sky_pipe,
            opaque_pipe,
            transparent_pipe,
            shadow_pipe,
            shadow_cut_pipe,
            post_pipe,
            linear_sampler,
            repeat_sampler,
            white_view,
            flat_normal_view,
            textures: HashMap::new(),
            tex_budget: std::env::var("LUMENCRAFT_TEXTURE_MB").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(DEFAULT_VIEWPORT_TEXTURE_MB) << 20,
            tex_max_side: MAX_SIDE_UNLIMITED,
            tex_stats: TextureStats::default(),
            meshes: HashMap::new(),
            materials: HashMap::new(),
            last_material_rev: u64::MAX,
        }
    }

    pub fn create_targets(&self, width: u32, height: u32) -> Targets {
        let (w, h) = (width.max(1), height.max(1));
        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let tex = |label, format, samples, usage| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let ra = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let samples = if self.quality.msaa > 1 { SAMPLES } else { 1 };
        let hdr_msaa = (samples > 1).then(|| tex("hdr_msaa", HDR_FORMAT, samples, ra).create_view(&Default::default()));
        let tb = wgpu::TextureUsages::TEXTURE_BINDING;
        let hdr_resolve = tex("hdr_resolve", HDR_FORMAT, 1, ra | tb | wgpu::TextureUsages::COPY_DST);
        let scene_color = tex("scene_color", HDR_FORMAT, 1, ra | tb | wgpu::TextureUsages::COPY_SRC);
        let scene_color_view = scene_color.create_view(&Default::default());
        let lin_depth_view = tex("lin_depth", LIN_DEPTH_FORMAT, 1, ra | tb).create_view(&Default::default());
        let exposure_view = self
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("exposure"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_FORMAT,
                usage: ra | tb,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let depth = tex("depth", DEPTH_FORMAT, samples, ra | wgpu::TextureUsages::TEXTURE_BINDING).create_view(&Default::default());
        let output = tex("output", OUT_FORMAT, 1, ra | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
        let output_view = output.create_view(&Default::default());
        let resolve_view = hdr_resolve.create_view(&Default::default());
        let (bw, bh) = ((w / 4).max(1), (h / 4).max(1));
        let bloom_tex = |label| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: bw, height: bh, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: HDR_FORMAT,
                    usage: ra | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let bloom_a = bloom_tex("bloom_a");
        let bloom_b = bloom_tex("bloom_b");
        let ubuf = |v: [f32; 4]| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("bloom_u"),
                contents: bytemuck::cast_slice(&v),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        };
        let bright_u = ubuf([0.0, 0.0, 1.2, 0.0]);
        let h_u = ubuf([1.0 / bw as f32, 0.0, 0.0, 0.0]);
        let v_u = ubuf([0.0, 1.0 / bh as f32, 0.0, 0.0]);
        let bloom_bg = |src: &wgpu::TextureView, u: &wgpu::Buffer| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom_bg"),
                layout: &self.bloom_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.linear_sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: u.as_entire_binding() },
                ],
            })
        };
        let bright_bg = bloom_bg(&resolve_view, &bright_u);
        let blur_h_bg = bloom_bg(&bloom_a, &h_u);
        let blur_v_bg = bloom_bg(&bloom_b, &v_u);
        let post_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post_bg"),
            layout: &self.post_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&resolve_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.linear_sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: self.post_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&bloom_a) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&lin_depth_view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&exposure_view) },
            ],
        });
        let ssr_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ssr_bg"),
            layout: &self.ssr_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&scene_color_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&lin_depth_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.linear_sampler) },
            ],
        });
        let depth_copy_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("depth_copy_bg"),
            layout: &self.depth_copy_bgls[(samples > 1) as usize],
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&depth) }],
        });
        let exposure_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("exposure_bg"),
            layout: &self.exposure_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&resolve_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.linear_sampler) },
            ],
        });
        Targets {
            size: (w, h),
            samples,
            hdr_msaa,
            hdr_resolve_tex: hdr_resolve,
            scene_color,
            scene_color_view,
            lin_depth_view,
            exposure_view,
            ssr_bg,
            depth_copy_bg,
            exposure_bg,
            depth,
            output,
            output_view,
            post_bg,
            resolve_view,
            bloom_a,
            bloom_b,
            bright_bg,
            blur_h_bg,
            blur_v_bg,
        }
    }

    /// Key under which a texture is stored for a colour space (normal maps are linear).
    fn tex_key(t: &TextureData, linear: bool) -> u64 {
        if linear {
            t.key ^ 0x9E37_79B9_7F4A_7C15
        } else {
            t.key
        }
    }

    fn upload_texture(&mut self, t: &TextureData, kind: crate::mips::MapKind) -> u64 {
        let linear = kind != crate::mips::MapKind::Srgb;
        let cap = self.tex_max_side;
        let key = Self::tex_key(t, linear) ^ (cap as u64).rotate_left(40);
        if self.textures.contains_key(&key) {
            return key;
        }
        // CPU mip chain so distant textures don't shimmer; colour maps are
        // filtered in linear light and normal maps renormalised (mips.rs).
        // Over budget: start the chain at the first level within the limit.
        let mut levels = crate::mips::mip_chain(t.width, t.height, (*t.rgba).clone(), kind);
        let skip = levels.iter().take_while(|(w, h, _)| (*w).max(*h) > cap).count().min(levels.len() - 1);
        levels.drain(..skip);
        let (tw, th) = (levels[0].0, levels[0].1);
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("material_tex"),
            size: wgpu::Extent3d { width: tw, height: th, depth_or_array_layers: 1 },
            mip_level_count: levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: if linear { wgpu::TextureFormat::Rgba8Unorm } else { wgpu::TextureFormat::Rgba8UnormSrgb },
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (i, (w, h, data)) in levels.iter().enumerate() {
            self.queue.write_texture(
                wgpu::ImageCopyTexture { texture: &tex, mip_level: i as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                data,
                wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(*h) },
                wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
            );
            self.staged_bytes += data.len() as u64;
        }
        // write_texture keeps a staging copy until the next submission: flush
        // periodically so loading a large model doesn't hold every texture
        // twice in system memory.
        if self.staged_bytes > 256 << 20 {
            let idx = self.queue.submit(std::iter::empty());
            self.device.poll(wgpu::Maintain::WaitForSubmissionIndex(idx));
            self.staged_bytes = 0;
        }
        self.textures.insert(key, tex.create_view(&Default::default()));
        key
    }

    fn material_uniform(p: &MaterialParams, has_tex: bool, has_normal: bool, has_rough: bool) -> MaterialU {
        let e = p.emissive_strength;
        let b = |v: bool| if v { 1.0 } else { 0.0 };
        MaterialU {
            base: p.base_color,
            emissive: [p.emissive[0] * e, p.emissive[1] * e, p.emissive[2] * e, p.normal_strength],
            params: [p.metallic, p.roughness, p.kind.id() as f32, p.uv_scale.max(0.001)],
            extra: [p.variation, b(has_tex), b(has_normal), b(p.cutout && has_tex && !p.kind.is_transparent())],
            uvt: p.uv_transform,
            extra2: [b(has_rough), p.uv_rotation.to_radians(), p.uv_offset[0], p.uv_offset[1]],
            extra3: [p.tex_influence, p.reflectivity, b(p.anti_tiling), 0.0],
            extra4: [b(p.box_mapping), 0.0, 0.0, 0.0],
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_frame_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        frame_buf: &wgpu::Buffer,
        shadow: &wgpu::TextureView,
        cmp: &wgpu::Sampler,
        lights: &wgpu::Buffer,
        lut: &wgpu::TextureView,
        lut_samp: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: frame_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(shadow) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(cmp) },
                wgpu::BindGroupEntry { binding: 3, resource: lights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(lut) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(lut_samp) },
            ],
        })
    }

    pub fn set_quality(&mut self, q: Quality) {
        if q.shadow_size != self.shadow_size {
            let size = q.shadow_size.min(self.device.limits().max_texture_dimension_2d);
            let (array, layers) = make_shadow_maps(&self.device, size);
            self.frame_bg = Self::make_frame_bg(
                &self.device,
                &self.frame_bgl,
                &self.frame_buf,
                &array,
                &self.cmp_sampler,
                &self.light_buf,
                &self.sky_lut_view,
                &self.sky_samp,
            );
            self.shadow_layers = layers;
            self.shadow_size = size;
            self.shadow_cache = [None; CASCADES];
        }
        if q.stagger_shadows != self.quality.stagger_shadows {
            self.shadow_cache = [None; CASCADES];
        }
        self.quality = q;
    }

    /// Changes whenever anything that affects the image (geometry/materials) changes.
    pub fn content_version(&self) -> u64 {
        self.geom_version + self.look_version
    }

    fn highlight_value(&self, name: &str) -> f32 {
        if self.highlight.1.as_deref() == Some(name) {
            2.0
        } else if self.highlight.0.as_deref() == Some(name) {
            1.0
        } else {
            0.0
        }
    }

    /// Tint the surfaces of a hovered (yellow) and a selected (blue) material,
    /// like Lumion's material picker. Only touches the affected uniforms.
    pub fn set_highlight(&mut self, hover: Option<&str>, selected: Option<&str>) {
        let new = (hover.map(str::to_string), selected.map(str::to_string));
        if new == self.highlight {
            return;
        }
        let old = std::mem::replace(&mut self.highlight, new);
        let mut names: Vec<String> = [old.0, old.1, self.highlight.0.clone(), self.highlight.1.clone()].into_iter().flatten().collect();
        names.sort();
        names.dedup();
        for n in names {
            let hv = self.highlight_value(&n);
            if let Some(g) = self.materials.get_mut(&n) {
                g.uniform.extra3[3] = hv;
                self.queue.write_buffer(&g.buf, 0, bytemuck::bytes_of(&g.uniform));
            }
        }
        self.look_version += 1;
    }

    /// Forget cached shadow maps (e.g. after a photo render with other settings).
    pub fn invalidate_shadows(&mut self) {
        self.shadow_cache = [None; CASCADES];
    }

    /// Forget all cached GPU meshes/materials (when switching to an unrelated scene).
    pub fn reset_cache(&mut self) {
        self.meshes.clear();
        self.materials.clear();
        self.last_material_rev = u64::MAX;
    }

    /// Bring GPU state in line with the scene (only touches what changed).
    /// Colour, normal and roughness maps a material uses. A map the user
    /// assigned in the editor wins over the model's own.
    fn material_maps(scene: &Scene, name: &str) -> (Option<TextureData>, Option<TextureData>, Option<TextureData>) {
        let m = &scene.imported_materials[name];
        let p = scene.effective(name);
        let tex = match &p.color_map {
            Some(path) => scene.user_textures.get(&crate::scene::user_texture_key(path, crate::texlib::MapKind::Color)).cloned(),
            None if p.use_texture => m.texture.clone(),
            None => None,
        };
        let ntex = match &p.normal_map {
            Some(path) => scene.user_textures.get(&crate::scene::user_texture_key(path, crate::texlib::MapKind::Normal)).cloned(),
            None => m.normal_texture.clone(),
        };
        let rtex = match &p.roughness_map {
            Some(path) => scene.user_textures.get(&crate::scene::user_texture_key(path, crate::texlib::MapKind::Roughness)).cloned(),
            None => m.roughness_texture.clone(),
        };
        (tex, ntex, rtex)
    }

    /// Viewport texture memory and any quality reduction in force.
    pub fn texture_stats(&self) -> TextureStats {
        self.tex_stats
    }

    pub fn texture_budget(&self) -> u64 {
        self.tex_budget
    }

    /// Change the viewport texture budget; maps are re-fitted on the next sync.
    pub fn set_texture_budget(&mut self, bytes: u64) {
        if bytes != self.tex_budget {
            self.tex_budget = bytes;
            self.last_material_rev = u64::MAX;
        }
    }

    pub fn sync_scene(&mut self, scene: &Scene) {
        // Meshes
        let before = self.meshes.len();
        self.meshes.retain(|id, _| scene.meshes.contains_key(id));
        if self.meshes.len() != before {
            self.geom_version += 1;
        }
        for (id, m) in &scene.meshes {
            if self.meshes.get(id).is_some_and(|g| g.rev == m.rev) {
                continue;
            }
            self.geom_version += 1;
            let d = &m.data;
            let verts: Vec<Vertex> =
                (0..d.positions.len()).map(|i| Vertex { pos: d.positions[i].to_array(), nrm: d.normals[i].to_array(), uv: d.uvs[i].to_array() }).collect();
            let vb = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vb"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let ib = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ib"),
                contents: bytemuck::cast_slice(&d.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
            self.meshes.insert(
                id.clone(),
                GpuMesh {
                    rev: m.rev,
                    vb,
                    ib,
                    count: d.indices.len() as u32,
                    material: d.material.clone(),
                    center: (m.bounds.0 + m.bounds.1) * 0.5,
                    lo: m.bounds.0,
                    hi: m.bounds.1,
                    casts_shadow: !id.starts_with("env:"),
                },
            );
        }

        // Materials — only when something changed (cheap equality check per frame).
        if scene.material_rev == self.last_material_rev && scene.overrides == self.last_overrides {
            return;
        }
        self.last_material_rev = scene.material_rev;
        self.last_overrides = scene.overrides.clone();
        self.geom_version += 1; // cut-out/transparency may have changed what casts shadows
        let names: Vec<String> = scene.imported_materials.keys().cloned().collect();
        // Enforce the texture budget: find the side limit every map in use
        // fits under, before uploading anything.
        let mut seen = std::collections::HashSet::new();
        let mut sizes = Vec::new();
        for name in &names {
            let (c, n, r) = Self::material_maps(scene, name);
            for (t, lin) in [(c, false), (n, true), (r, true)] {
                if let Some(t) = t {
                    if seen.insert(Self::tex_key(&t, lin)) {
                        sizes.push((t.width, t.height));
                    }
                }
            }
        }
        let cap = fit_max_side(&sizes, self.tex_budget);
        let native_bytes: u64 = sizes.iter().map(|&(w, h)| crate::mips::chain_bytes(w, h)).sum();
        let bytes: u64 = sizes
            .iter()
            .map(|&(w, h)| {
                let (w, h) = capped_size(w, h, cap);
                crate::mips::chain_bytes(w, h)
            })
            .sum();
        let reduced = sizes.iter().filter(|(w, h)| (*w).max(*h) > cap).count() as u32;
        let stats = TextureStats { native_bytes, bytes, budget: self.tex_budget, reduced, maps: sizes.len() as u32, max_side: (reduced > 0).then_some(cap) };
        if stats.reduced > 0 && (stats.max_side != self.tex_stats.max_side || stats.reduced != self.tex_stats.reduced) {
            log::warn!("{}", stats.reduction_note().unwrap_or_default());
        }
        if cap != self.tex_max_side {
            // Re-fitted: drop the old copies so both sizes are never held at once.
            self.textures.clear();
        }
        self.tex_max_side = cap;
        self.tex_stats = stats;
        for name in names {
            let p = scene.effective(&name);
            let (tex, ntex, rtex) = Self::material_maps(scene, &name);
            use crate::mips::MapKind as K;
            let rk = rtex.as_ref().map_or(0, |t| self.upload_texture(t, K::Linear));
            let ck = tex.as_ref().map_or(0, |t| self.upload_texture(t, K::Srgb));
            let nk = ntex.as_ref().map_or(0, |t| self.upload_texture(t, K::Normal));
            let key = (ck, nk, rk);
            let mut u = Self::material_uniform(&p, tex.is_some(), ntex.is_some(), rtex.is_some());
            u.extra3[3] = self.highlight_value(&name);
            let cutout = u.extra[3] > 0.5;
            let glossy = !p.kind.is_transparent() && matches!(p.kind, MaterialKind::Standard) && (p.roughness < 0.42 || p.metallic > 0.5) && !p.invisible;
            let needs_new = self.materials.get(&name).is_none_or(|g| g.tex_key != key);
            if needs_new {
                let buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("material"),
                    contents: bytemuck::bytes_of(&u),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });
                let view = if ck == 0 { &self.white_view } else { &self.textures[&ck] };
                let nview = if nk == 0 { &self.flat_normal_view } else { &self.textures[&nk] };
                let rview = if rk == 0 { &self.white_view } else { &self.textures[&rk] };
                let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("material_bg"),
                    layout: &self.mat_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.repeat_sampler) },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(nview) },
                        wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(rview) },
                    ],
                });
                self.materials.insert(name.clone(), GpuMaterial { buf, bg, tex_key: key, kind: p.kind, cutout, invisible: p.invisible, glossy, uniform: u });
            } else {
                let g = self.materials.get_mut(&name).unwrap();
                self.queue.write_buffer(&g.buf, 0, bytemuck::bytes_of(&u));
                g.kind = p.kind;
                g.cutout = cutout;
                g.invisible = p.invisible;
                g.glossy = glossy;
                g.uniform = u;
            }
        }
        // Free GPU memory of materials/textures that no longer exist.
        self.materials.retain(|k, _| scene.imported_materials.contains_key(k));
        let live: std::collections::HashSet<u64> = self.materials.values().flat_map(|m| [m.tex_key.0, m.tex_key.1, m.tex_key.2]).collect();
        self.textures.retain(|k, _| live.contains(k));
    }

    /// Fit one orthographic light matrix per cascade around slices of the
    /// camera frustum. Returns (matrices, far split depths, texel sizes).
    fn cascades(scene: &Scene, fp: &FrameParams, shadow_size: u32, pad: f32) -> (Vec<CascadeFit>, [f32; 4]) {
        let (lo, hi) = scene
            .meshes
            .iter()
            .filter(|(k, _)| !k.starts_with("env:"))
            .map(|(_, m)| m.bounds)
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
            .unwrap_or((Vec3::splat(-20.0), Vec3::splat(20.0)));
        let scene_c = (lo + hi) * 0.5;
        let scene_r = ((hi - lo).length() * 0.5).max(5.0);
        // Shadow distance: cover the model from wherever the camera is, capped.
        let reach = fp.cam_pos.distance(scene_c) + scene_r;
        let max_dist = reach.clamp(30.0, 1500.0);
        let near = 0.1f32;
        let lambda = 0.8f32;
        let mut splits = [0f32; 4];
        for (i, split) in splits.iter_mut().enumerate().take(CASCADES) {
            let f = (i + 1) as f32 / CASCADES as f32;
            let log = near * (max_dist / near).powf(f);
            let lin = near + (max_dist - near) * f;
            *split = lambda * log + (1.0 - lambda) * lin;
        }
        let dir = if fp.sun_dir.y < 0.02 { Vec3::new(fp.sun_dir.x, 0.02, fp.sun_dir.z).normalize() } else { fp.sun_dir };
        let up = if dir.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
        // Camera basis from the view matrix.
        let inv_view = fp.view.inverse();
        let fwd = -inv_view.z_axis.truncate();
        let right = inv_view.x_axis.truncate();
        let cam_up = inv_view.y_axis.truncate();
        let tan_y = 1.0 / fp.proj.y_axis.y;
        let tan_x = 1.0 / fp.proj.x_axis.x;
        let rot = Mat4::look_at_rh(Vec3::ZERO, -dir, up);
        let mut fits = Vec::with_capacity(CASCADES);
        let mut prev = near;
        for &split in splits.iter().take(CASCADES) {
            let (a, b) = (prev, split);
            prev = b;
            let mut corners = Vec::with_capacity(8);
            for d in [a, b] {
                for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                    corners.push(fp.cam_pos + fwd * d + right * (sx * tan_x * d) + cam_up * (sy * tan_y * d));
                }
            }
            let center = corners.iter().copied().sum::<Vec3>() / 8.0;
            // `pad` leaves slack so a cached cascade keeps covering the view
            // while the camera moves a little (lets far cascades update less often).
            let radius = (corners.iter().map(|c| c.distance(center)).fold(0.0, f32::max) * pad).ceil().max(1.0);
            // Snap to whole texels so shadows don't shimmer when the camera moves.
            let texel = radius * 2.0 / shadow_size as f32;
            let mut ls = rot.transform_point3(center);
            ls.x = (ls.x / texel).floor() * texel;
            ls.y = (ls.y / texel).floor() * texel;
            let center = rot.inverse().transform_point3(ls);
            // Pull the eye back far enough to include every caster in the scene.
            let back = radius + scene_r * 2.0 + scene_c.distance(center);
            let view = Mat4::look_at_rh(center + dir * back, center, up);
            let proj = Mat4::orthographic_rh(-radius, radius, -radius, radius, 0.0, back + radius);
            fits.push(CascadeFit { vp: proj * view, texel, rot, center, radius });
        }
        (fits, splits)
    }

    /// Render the scene into `targets.output` (sRGB, ready to display / save).
    pub fn render(&mut self, scene: &Scene, fp: &FrameParams, targets: &Targets) {
        let vp = fp.proj * fp.view;
        let q = self.quality;
        let pad = if q.stagger_shadows { 1.2 } else { 1.0 };
        let (fits, splits) = Self::cascades(scene, fp, self.shadow_size, pad);
        self.frame_no += 1;
        // Decide which cascades need re-rendering; reuse the rest.
        let periods = [1u64, 2, 4, 6];
        let mut update = [false; CASCADES];
        for c in 0..CASCADES {
            let f = &fits[c];
            update[c] = match &self.shadow_cache[c] {
                None => true,
                Some(k) => {
                    let moved_sun = k.sun.distance(fp.sun_dir) > 1e-4;
                    let geom = k.geom != self.geom_version;
                    let cl = k.rot.transform_point3(f.center);
                    let covers = (glam::Vec2::new(cl.x, cl.y) - k.center_ls).length() + f.radius <= k.radius + 1e-3;
                    if moved_sun || geom || !covers {
                        true
                    } else if k.vp == f.vp {
                        false
                    } else if !q.stagger_shadows || c == 0 || self.settle {
                        true
                    } else {
                        (self.frame_no + c as u64).is_multiple_of(periods[c])
                    }
                }
            };
            if update[c] {
                let cl = f.rot.transform_point3(f.center);
                self.shadow_cache[c] = Some(CascadeCache {
                    vp: f.vp,
                    texel: f.texel,
                    rot: f.rot,
                    center_ls: glam::Vec2::new(cl.x, cl.y),
                    radius: f.radius,
                    sun: fp.sun_dir,
                    geom: self.geom_version,
                });
            }
        }
        let light_vps: [Mat4; CASCADES] = std::array::from_fn(|c| self.shadow_cache[c].unwrap().vp);
        let texels: [f32; 4] = std::array::from_fn(|c| self.shadow_cache[c].unwrap().texel);
        let nlights = fp.lights.len().min(MAX_LIGHTS);
        if nlights > 0 {
            self.queue.write_buffer(&self.light_buf, 0, bytemuck::cast_slice(&fp.lights[..nlights]));
        }
        let fwd = -fp.view.inverse().z_axis.truncate();
        let fu = FrameU {
            view_proj: vp.to_cols_array_2d(),
            inv_view_proj: vp.inverse().to_cols_array_2d(),
            light_vp: light_vps.map(|m| m.to_cols_array_2d()),
            cascade_far: splits,
            cascade_texel: texels,
            cam_pos: [fp.cam_pos.x, fp.cam_pos.y, fp.cam_pos.z, fp.time],
            cam_fwd: [fwd.x, fwd.y, fwd.z, nlights as f32],
            sun_dir: [fp.sun_dir.x, fp.sun_dir.y, fp.sun_dir.z, fp.sun_intensity],
            sun_color: [fp.sun_color.x, fp.sun_color.y, fp.sun_color.z, fp.ambient],
            sky: [fp.cloudiness, fp.fog, fp.haze, fp.shadow_softness],
            post: [fp.exposure, fp.contrast, fp.saturation, fp.vignette],
            misc: [q.pcf_radius as f32, fp.wind, fp.reflections, 0.0],
            extra: [if q.level >= 2 { fp.ssr } else { 0.0 }, q.level as f32, 0.0, 0.0],
        };
        self.queue.write_buffer(&self.frame_buf, 0, bytemuck::bytes_of(&fu));
        let pu = Self::post_uniform(q, fp, targets);
        self.queue.write_buffer(&self.post_buf, 0, bytemuck::bytes_of(&pu));

        let frustum = Frustum::from_matrix(vp);
        let mut opaque: Vec<&GpuMesh> = Vec::new();
        let mut transparent: Vec<(f32, &GpuMesh)> = Vec::new();
        let mut casters: Vec<&GpuMesh> = Vec::new();
        let mut stats = RenderStats { lights: nlights, ..Default::default() };
        for (id, m) in &self.meshes {
            let Some(mat) = self.materials.get(&m.material) else { continue };
            if mat.invisible || (q.level <= 1 && id.starts_with("env:grass")) {
                continue;
            }
            if !mat.kind.is_transparent() && m.casts_shadow {
                casters.push(m);
            }
            if !frustum.visible(m.lo, m.hi) {
                stats.culled += 1;
                continue;
            }
            stats.drawn += 1;
            if mat.kind.is_transparent() {
                transparent.push((m.center.distance_squared(fp.cam_pos), m));
            } else {
                opaque.push(m);
            }
        }
        transparent.sort_by(|a, b| b.0.total_cmp(&a.0));
        // Front-to-back so the depth test rejects hidden pixels before shading.
        opaque.sort_by(|a, b| a.center.distance_squared(fp.cam_pos).total_cmp(&b.center.distance_squared(fp.cam_pos)));
        stats.shadow_updates = update.iter().filter(|u| **u).count();

        let mut enc = self.device.create_command_encoder(&Default::default());
        for c in 0..CASCADES {
            if !update[c] {
                continue; // keep last frame's shadow map for this cascade
            }
            self.queue.write_buffer(&self.shadow_ubufs[c], 0, bytemuck::cast_slice(&light_vps[c].to_cols_array()));
            let cf = Frustum::from_matrix(light_vps[c]);
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow_pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_layers[c],
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if fp.sun_dir.y > -0.05 {
                pass.set_pipeline(&self.shadow_pipe);
                pass.set_bind_group(0, &self.shadow_bgs[c], &[]);
                let mut cut = Vec::new();
                // Casters smaller than ~2 shadow texels don't change the far cascades.
                let min_size = if c >= 2 { texels[c] * 2.0 } else { 0.0 };
                for m in casters.iter().filter(|m| cf.visible(m.lo, m.hi) && (m.hi - m.lo).max_element() >= min_size) {
                    let mat = &self.materials[&m.material];
                    if mat.cutout {
                        cut.push((*m, &mat.bg));
                        continue;
                    }
                    pass.set_vertex_buffer(0, m.vb.slice(..));
                    pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..m.count, 0, 0..1);
                }
                if !cut.is_empty() {
                    pass.set_pipeline(&self.shadow_cut_pipe);
                    pass.set_bind_group(0, &self.shadow_bgs[c], &[]);
                    for (m, bg) in cut {
                        pass.set_bind_group(1, bg, &[]);
                        pass.set_vertex_buffer(0, m.vb.slice(..));
                        pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..m.count, 0, 0..1);
                    }
                }
            }
        }
        // Atmosphere LUT for this sun position / haze.
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sky_lut"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.sky_lut_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.sky_lut_pipe);
            pass.set_bind_group(0, &self.sky_lut_bg, &[]);
            pass.draw(0..3, 0..1);
        }
        let ms = targets.samples > 1;
        let mi = ms as usize;
        // Pass A: opaque geometry + sky into the colour that reflections sample.
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: targets.hdr_msaa.as_ref().unwrap_or(&targets.scene_color_view),
                    resolve_target: targets.hdr_msaa.as_ref().map(|_| &targets.scene_color_view),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &self.frame_bg, &[]);
            // The pre-pass doubles vertex work; worth it unless the view is geometry-heavy.
            let visible_tris: u64 = opaque.iter().map(|m| m.count as u64 / 3).sum();
            let use_prepass = q.prepass && visible_tris < 600_000;
            if use_prepass {
                // 1) depth only (cut-outs need their alpha test, so they skip this)
                pass.set_pipeline(&self.depth_pipes[mi]);
                for m in opaque.iter().filter(|m| !self.materials[&m.material].cutout) {
                    pass.set_bind_group(1, &self.materials[&m.material].bg, &[]);
                    pass.set_vertex_buffer(0, m.vb.slice(..));
                    pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..m.count, 0, 0..1);
                }
                // 2) shade only the surviving (front-most) pixels
                pass.set_pipeline(&self.opaque_eq_pipes[mi]);
                for m in opaque.iter().filter(|m| !self.materials[&m.material].cutout) {
                    pass.set_bind_group(1, &self.materials[&m.material].bg, &[]);
                    pass.set_vertex_buffer(0, m.vb.slice(..));
                    pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..m.count, 0, 0..1);
                }
            }
            pass.set_pipeline(if ms { &self.opaque_pipe } else { &self.opaque_pipe_ss });
            for m in opaque.iter().filter(|m| !use_prepass || self.materials[&m.material].cutout) {
                pass.set_bind_group(1, &self.materials[&m.material].bg, &[]);
                pass.set_vertex_buffer(0, m.vb.slice(..));
                pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..m.count, 0, 0..1);
            }
            pass.set_pipeline(if ms { &self.sky_pipe } else { &self.sky_pipe_ss });
            pass.draw(0..3, 0..1);
        }
        // Depth as a plain float texture for reflections, AO and depth of field.
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("depth_copy"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.lin_depth_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.depth_copy_pipes[mi]);
            pass.set_bind_group(0, &targets.depth_copy_bg, &[]);
            pass.draw(0..3, 0..1);
        }
        if !ms {
            // Without MSAA, pass B draws over a copy of pass A's image.
            let (w, h) = targets.size;
            enc.copy_texture_to_texture(
                targets.scene_color.as_image_copy(),
                targets.hdr_resolve_tex.as_image_copy(),
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        }
        // Pass B: reflections on glossy surfaces, then glass and water.
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("reflect_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: targets.hdr_msaa.as_ref().unwrap_or(&targets.resolve_view),
                    resolve_target: targets.hdr_msaa.as_ref().map(|_| &targets.resolve_view),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Discard },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &self.frame_bg, &[]);
            pass.set_bind_group(2, &targets.ssr_bg, &[]);
            if q.level >= 2 && fp.ssr > 0.0 {
                pass.set_pipeline(&self.overlay_pipes[mi]);
                for m in opaque.iter().filter(|m| {
                    let g = &self.materials[&m.material];
                    g.glossy && !g.cutout
                }) {
                    pass.set_bind_group(1, &self.materials[&m.material].bg, &[]);
                    pass.set_vertex_buffer(0, m.vb.slice(..));
                    pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..m.count, 0, 0..1);
                }
            }
            pass.set_pipeline(if ms { &self.transparent_pipe } else { &self.transparent_pipe_ss });
            for (_, m) in &transparent {
                pass.set_bind_group(1, &self.materials[&m.material].bg, &[]);
                pass.set_vertex_buffer(0, m.vb.slice(..));
                pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..m.count, 0, 0..1);
            }
        }
        let ov = self.hdr_override.take();
        self.encode_post(&mut enc, fp, targets, ov);
        self.queue.submit(Some(enc.finish()));
        self.stats = stats;
    }

    fn post_uniform(q: Quality, fp: &FrameParams, targets: &Targets) -> PostU {
        PostU {
            params: [fp.exposure, fp.contrast, fp.saturation, fp.vignette],
            extra: [fp.bloom, fp.sharpen, fp.ao, fp.ao_radius],
            quality: [q.ssao_samples as f32, if targets.samples > 1 { 0.0 } else { 1.0 }, fp.time, if fp.auto_exposure { 1.0 } else { 0.0 }],
            lens: fp.lens,
            proj: fp.proj.to_cols_array_2d(),
            inv_proj: fp.proj.inverse().to_cols_array_2d(),
            tone: [if fp.filmic { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
        }
    }

    /// Post-process only: the HDR image set with `set_hdr_override*` goes
    /// through bloom, exposure, depth of field and grading into `targets`,
    /// reusing the depth those targets got from their last full `render`
    /// (same view). Used for path-traced images: skips shadow maps and the
    /// whole raster scene, which the override would replace anyway.
    pub fn render_post_only(&mut self, fp: &FrameParams, targets: &Targets) {
        let pu = Self::post_uniform(self.quality, fp, targets);
        self.queue.write_buffer(&self.post_buf, 0, bytemuck::bytes_of(&pu));
        let mut enc = self.device.create_command_encoder(&Default::default());
        let ov = self.hdr_override.take();
        self.encode_post(&mut enc, fp, targets, ov);
        self.queue.submit(Some(enc.finish()));
    }

    fn encode_post(&self, enc: &mut wgpu::CommandEncoder, fp: &FrameParams, targets: &Targets, ov: Option<(u32, u32, wgpu::Buffer)>) {
        let q = self.quality;
        if let Some((w, h, buf)) = ov {
            if (w, h) == targets.size {
                let row = (w * 8).div_ceil(256) * 256;
                enc.copy_buffer_to_texture(
                    wgpu::ImageCopyBuffer { buffer: &buf, layout: wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
                    targets.hdr_resolve_tex.as_image_copy(),
                    wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
            }
        }
        // Bloom: bright pass -> blur H -> blur V (two iterations for a wide glow).
        let all_passes: [(&wgpu::RenderPipeline, &wgpu::BindGroup, &wgpu::TextureView); 5] = [
            (&self.bright_pipe, &targets.bright_bg, &targets.bloom_a),
            (&self.blur_pipe, &targets.blur_h_bg, &targets.bloom_b),
            (&self.blur_pipe, &targets.blur_v_bg, &targets.bloom_a),
            (&self.blur_pipe, &targets.blur_h_bg, &targets.bloom_b),
            (&self.blur_pipe, &targets.blur_v_bg, &targets.bloom_a),
        ];
        let n_bloom = if fp.bloom <= 0.0 {
            0
        } else if q.bloom_passes <= 1 {
            3
        } else {
            5
        };
        for (pipe, bg, dst) in all_passes.into_iter().take(n_bloom) {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bloom_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: dst,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..3, 0..1);
        }
        if fp.auto_exposure {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("exposure"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.exposure_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.exposure_pipe);
            pass.set_bind_group(0, &targets.exposure_bg, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.output_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.post_pipe);
            pass.set_bind_group(0, &targets.post_bg, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// Read the output texture back to CPU as RGBA8 (for photo export).
    /// Use this linear HDR image instead of the rasterized one on the next
    /// `render` with targets of the same size.
    pub fn set_hdr_override(&mut self, w: u32, h: u32, rgb: &[[f32; 3]]) {
        use wgpu::util::DeviceExt;
        let row = ((w * 8).div_ceil(256) * 256) as usize;
        let mut bytes = vec![0u8; row * h as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let c = rgb[y * w as usize + x];
                let o = y * row + x * 8;
                for (k, v) in [c[0], c[1], c[2], 1.0].into_iter().enumerate() {
                    bytes[o + k * 2..o + k * 2 + 2].copy_from_slice(&crate::pathtrace::f16_bits(v.max(0.0)).to_le_bytes());
                }
            }
        }
        let buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("hdr_override"),
            contents: &bytes,
            usage: wgpu::BufferUsages::COPY_SRC,
        });
        self.hdr_override = Some((w, h, buf));
    }

    /// As `set_hdr_override`, from RGBA16F pixels already laid out with
    /// 256-byte-aligned rows (the path tracer's packed read-back).
    pub fn set_hdr_override_packed(&mut self, w: u32, h: u32, rgba16f: &[u8]) {
        use wgpu::util::DeviceExt;
        let row = ((w * 8).div_ceil(256) * 256) as usize;
        assert!(rgba16f.len() >= row * h as usize, "packed HDR image too small");
        let buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("hdr_override"),
            contents: &rgba16f[..row * h as usize],
            usage: wgpu::BufferUsages::COPY_SRC,
        });
        self.hdr_override = Some((w, h, buf));
    }

    pub fn read_output(&self, targets: &Targets) -> image::RgbaImage {
        let (w, h) = targets.size;
        let bpr = (w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (bpr * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            targets.output.as_image_copy(),
            wgpu::ImageCopyBuffer { buffer: &buf, layout: wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit(Some(enc.finish()));
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        let _ = rx.recv();
        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for row in 0..h {
            let s = (row * bpr) as usize;
            out.extend_from_slice(&data[s..s + (w * 4) as usize]);
        }
        drop(data);
        buf.unmap();
        image::RgbaImage::from_raw(w, h, out).expect("readback size")
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn viewport_budget_limits_the_largest_maps() {
        let sizes = vec![(4096, 4096); 40]; // ~2.7 GB with mips
        let cap = fit_max_side(&sizes, 512 << 20);
        let used: u64 = sizes
            .iter()
            .map(|&(w, h)| {
                let (w, h) = capped_size(w, h, cap);
                crate::mips::chain_bytes(w, h)
            })
            .sum();
        assert!(used <= 512 << 20, "{used}");
        assert_eq!(cap, 1024);
        // Plenty of room: untouched.
        assert_eq!(fit_max_side(&sizes, 8 << 30), MAX_SIDE_UNLIMITED);
        // Tiny budget: goes below 512 px, never below 128.
        assert_eq!(fit_max_side(&sizes, 1 << 20), 128);
        assert_eq!(capped_size(4096, 1024, 256), (256, 64));
        assert_eq!(capped_size(100, 50, 256), (100, 50));
    }
}
