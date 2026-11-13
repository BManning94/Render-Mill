// LumenCraft path tracer: unidirectional path tracing with next-event
// estimation for the sun and local lights, GGX + Lambert materials matching
// the rasterizer's material model, thin glass, refractive water, foliage
// translucency and alpha cut-outs. One sample per pixel per dispatch; the
// result accumulates in a buffer and is denoised afterwards.

struct Frame {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    light_vp: array<mat4x4<f32>, 4>,
    cascade_far: vec4<f32>,
    cascade_texel: vec4<f32>,
    cam_pos: vec4<f32>,     // xyz, time
    cam_fwd: vec4<f32>,     // xyz, number of lights
    sun_dir: vec4<f32>,     // xyz (towards sun), intensity
    sun_color: vec4<f32>,   // rgb, ambient strength
    sky: vec4<f32>,         // cloudiness, fog density, haze, shadow softness
    post: vec4<f32>,
    misc: vec4<f32>,        // pcf, wind, reflections, unused
    extra: vec4<f32>,
};

struct Light {
    pos_range: vec4<f32>,
    color: vec4<f32>,
    dir: vec4<f32>,
};

struct Params {
    size: vec4<u32>,     // width, height, frame index (seed), samples so far
    tile: vec4<u32>,     // x0, y0, tile width, tile height
    opts: vec4<f32>,     // max bounces, firefly clamp, pixel spread (rad), total window (portal) area
    lens: vec4<f32>,     // aperture radius (m), focus distance (m), samples this dispatch, centre rays (tests)
    portal: vec4<u32>,   // first portal entry in `nodes`, portal count, unused x2
};

struct Node {
    lo: vec3<f32>,
    a: u32,
    hi: vec3<f32>,
    b: u32,
};

struct Mat {
    base: vec4<f32>,
    p0: vec4<f32>,   // metallic, roughness, kind, uv scale
    p1: vec4<f32>,   // colour layer, normal layer, roughness layer (-1 = none), normal strength
    uvt: vec4<f32>,  // uv scale xy, offset zw
    p3: vec4<f32>,   // uv rotation, uv offset u, v, texture influence
    p4: vec4<f32>,   // emissive rgb * strength, reflectivity
    p5: vec4<f32>,   // variation, cutout, has colour map, real-world box mapping
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var sky_lut: texture_2d<f32>;
@group(0) @binding(2) var sky_samp: sampler;
@group(0) @binding(3) var<storage, read> lights: array<Light>;
@group(0) @binding(4) var<uniform> pt: Params;

@group(1) @binding(0) var<storage, read> positions: array<vec4<f32>>;
@group(1) @binding(1) var<storage, read> attrs: array<vec4<f32>>;
@group(1) @binding(2) var<storage, read> tris: array<vec4<u32>>;
@group(1) @binding(3) var<storage, read> nodes: array<Node>;
@group(1) @binding(4) var<storage, read> mats: array<Mat>;
// Textures keep their own resolution: one array per size class
// (128, 256, 512, 1024, 2048, 4096) for colour (sRGB) and data (linear:
// normal, roughness). A material refers to a map by code = class * 1024 + layer.
@group(1) @binding(5) var tex_c0: texture_2d_array<f32>;
@group(1) @binding(6) var tex_c1: texture_2d_array<f32>;
@group(1) @binding(7) var tex_c2: texture_2d_array<f32>;
@group(1) @binding(8) var tex_c3: texture_2d_array<f32>;
@group(1) @binding(9) var tex_c4: texture_2d_array<f32>;
@group(1) @binding(10) var tex_c5: texture_2d_array<f32>;
@group(1) @binding(11) var tex_d0: texture_2d_array<f32>;
@group(1) @binding(12) var tex_d1: texture_2d_array<f32>;
@group(1) @binding(13) var tex_d2: texture_2d_array<f32>;
@group(1) @binding(14) var tex_d3: texture_2d_array<f32>;
@group(1) @binding(15) var tex_d4: texture_2d_array<f32>;
@group(1) @binding(16) var tex_d5: texture_2d_array<f32>;
@group(1) @binding(17) var tex_samp: sampler;

// Mip level for a map: footprint in UV units (log2) plus the class size
// (class c is 128 << c = 2^(7 + c) pixels).
fn map_level(code: f32, lod_uv: f32) -> f32 {
    let size_class = u32(code) / 1024u;
    return max(lod_uv + 7.0 + f32(size_class), 0.0);
}

fn color_map(code: f32, uv: vec2<f32>, level: f32) -> vec4<f32> {
    let c = u32(code);
    let layer = i32(c % 1024u);
    switch (c / 1024u) {
        case 0u: { return textureSampleLevel(tex_c0, tex_samp, uv, layer, level); }
        case 1u: { return textureSampleLevel(tex_c1, tex_samp, uv, layer, level); }
        case 2u: { return textureSampleLevel(tex_c2, tex_samp, uv, layer, level); }
        case 3u: { return textureSampleLevel(tex_c3, tex_samp, uv, layer, level); }
        case 4u: { return textureSampleLevel(tex_c4, tex_samp, uv, layer, level); }
        default: { return textureSampleLevel(tex_c5, tex_samp, uv, layer, level); }
    }
}

fn data_map(code: f32, uv: vec2<f32>, level: f32) -> vec4<f32> {
    let c = u32(code);
    let layer = i32(c % 1024u);
    switch (c / 1024u) {
        case 0u: { return textureSampleLevel(tex_d0, tex_samp, uv, layer, level); }
        case 1u: { return textureSampleLevel(tex_d1, tex_samp, uv, layer, level); }
        case 2u: { return textureSampleLevel(tex_d2, tex_samp, uv, layer, level); }
        case 3u: { return textureSampleLevel(tex_d3, tex_samp, uv, layer, level); }
        case 4u: { return textureSampleLevel(tex_d4, tex_samp, uv, layer, level); }
        default: { return textureSampleLevel(tex_d5, tex_samp, uv, layer, level); }
    }
}

@group(2) @binding(0) var<storage, read_write> accum: array<vec4<f32>>;
// [0, n): albedo.rgb + hit distance; [n, 2n): normal.xyz
@group(2) @binding(1) var<storage, read_write> aux: array<vec4<f32>>;

const PI: f32 = 3.14159265;
const INF: f32 = 1e30;

// ---------------------------------------------------------------- random
var<private> rng_state: u32;

fn pcg() -> u32 {
    rng_state = rng_state * 747796405u + 2891336453u;
    let w = ((rng_state >> ((rng_state >> 28u) + 4u)) ^ rng_state) * 277803737u;
    return (w >> 22u) ^ w;
}

fn rnd() -> f32 {
    return f32(pcg() >> 8u) / 16777216.0;
}

// ---------------------------------------------------------------- noise (as in scene.wgsl)
fn hash2(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(123.34, 456.21));
    let r = q + dot(q, q + 45.32);
    return fract(r.x * r.y);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash2(i), hash2(i + vec2<f32>(1.0, 0.0)), u.x), mix(hash2(i + vec2<f32>(0.0, 1.0)), hash2(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

fn fbm(p0: vec2<f32>) -> f32 {
    var p = p0;
    var a = 0.5;
    var s = 0.0;
    for (var i = 0; i < 5; i = i + 1) {
        s = s + a * vnoise(p);
        p = p * 2.03 + vec2<f32>(17.1, 9.7);
        a = a * 0.5;
    }
    return s;
}

fn fbm3(p0: vec2<f32>) -> f32 {
    var p = p0;
    var a = 0.5;
    var s = 0.0;
    for (var i = 0; i < 3; i = i + 1) {
        s = s + a * vnoise(p);
        p = p * 2.03 + vec2<f32>(17.1, 9.7);
        a = a * 0.5;
    }
    return s / 0.875;
}

// ---------------------------------------------------------------- sky
fn day_factor() -> f32 {
    return smoothstep(-0.12, 0.18, frame.sun_dir.y);
}

fn sky_base(dir: vec3<f32>) -> vec3<f32> {
    let az = atan2(dir.x, -dir.z) / (2.0 * PI) + 0.5;
    let el = asin(clamp(dir.y, -1.0, 1.0)) / (PI * 0.5);
    let v = 0.5 + 0.5 * sign(el) * sqrt(abs(el));
    return textureSampleLevel(sky_lut, sky_samp, vec2<f32>(az, 1.0 - v), 0.0).rgb;
}

fn cloud_density(p: vec2<f32>, cover: f32) -> f32 {
    let base = fbm(p * 0.55);
    let detail = fbm(p * 2.3 + vec2<f32>(base * 1.7, 3.1));
    let n = base * 0.75 + detail * 0.35;
    return clamp((n - (1.0 - cover) * 0.72 - 0.06) * 3.0, 0.0, 1.0);
}

fn clouds(dir: vec3<f32>, col: vec3<f32>) -> vec3<f32> {
    let cover = frame.sky.x;
    if (dir.y <= 0.0 || cover <= 0.001) {
        return col;
    }
    let t = frame.cam_pos.w * frame.misc.y;
    let p = dir.xz / (dir.y + 0.06) * 1.2 + vec2<f32>(t * 0.015, t * 0.006);
    let d = cloud_density(p, cover);
    if (d <= 0.0) {
        return col;
    }
    let sun = frame.sun_dir.xyz;
    let sd = normalize(sun.xz + vec2<f32>(1e-4, 0.0));
    var od = 0.0;
    for (var i = 1; i <= 3; i = i + 1) {
        od = od + cloud_density(p + sd * 0.12 * f32(i), cover);
    }
    let light = exp(-od * 1.1);
    let mu = dot(dir, sun);
    let g = 0.6;
    let hg = (1.0 - g * g) / pow(max(1.0 + g * g - 2.0 * g * mu, 1e-3), 1.5) / (4.0 * PI);
    let sun_c = frame.sun_color.rgb * frame.sun_dir.w * 0.13;
    let amb = sky_base(vec3<f32>(0.0, 1.0, 0.0)) * 0.9 + sky_base(normalize(vec3<f32>(dir.x, 0.05, dir.z))) * 0.5;
    var cloud_col = sun_c * light * (0.7 + 6.0 * hg) * day_factor() + amb * (0.55 + 0.45 * (1.0 - d));
    cloud_col = mix(cloud_col, vec3<f32>(dot(cloud_col, vec3<f32>(0.3, 0.5, 0.2))), cover * cover * 0.5);
    let alpha = (1.0 - exp(-d * 4.0)) * smoothstep(0.0, 0.12, dir.y);
    let far = smoothstep(0.02, 0.25, dir.y);
    return mix(col, mix(col, cloud_col, far * 0.85 + 0.15), alpha);
}

// What the camera sees when a ray leaves the scene.
fn background(dir: vec3<f32>) -> vec3<f32> {
    var col = clouds(dir, sky_base(dir));
    let mu = dot(dir, frame.sun_dir.xyz);
    let disk = smoothstep(0.9995, 0.99975, mu) * smoothstep(-0.02, 0.01, frame.sun_dir.y);
    col = col + frame.sun_color.rgb * disk * 40.0;
    return col * frame.sun_color.a;
}

// Sky light arriving along a bounce direction: the same sky the camera sees
// (energy-consistent), without the stylised sun disk - the sun itself is
// added with multiple importance sampling (see sun_hit / direct).
fn sky_light(dir: vec3<f32>) -> vec3<f32> {
    let d = normalize(vec3<f32>(dir.x, max(dir.y, -0.15), dir.z));
    return clouds(d, sky_base(d)) * frame.sun_color.a;
}

// The sun as a light source: a disc of angular radius sun_radius() with
// radiance chosen so its irradiance equals sun_color * intensity.
fn sun_radius() -> f32 {
    return 0.0047 + frame.sky.w * 0.01;
}

fn sun_solid_angle() -> f32 {
    return 2.0 * PI * (1.0 - cos(sun_radius()));
}

fn sun_irradiance() -> vec3<f32> {
    let vis = smoothstep(-0.03, 0.04, frame.sun_dir.y);
    return frame.sun_color.rgb * frame.sun_dir.w * vis;
}

fn power_heuristic(a: f32, b: f32) -> f32 {
    let a2 = a * a;
    return a2 / max(a2 + b * b, 1e-20);
}

// Offset a ray origin off a surface along its geometric normal, scaled with
// the coordinate magnitude so models far from the origin (survey coordinates,
// distant terrain) don't self-intersect from float rounding.
fn offset_ray(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let m = max(abs(p.x), max(abs(p.y), abs(p.z)));
    return p + n * (2e-4 + 1e-5 * m);
}

fn apply_fog(c: vec3<f32>, wp: vec3<f32>) -> vec3<f32> {
    let d = distance(wp, frame.cam_pos.xyz);
    let avg_h = max((wp.y + frame.cam_pos.y) * 0.5, 0.0);
    let fh = 1.0 - exp(-min(d, 1200.0) * frame.sky.y * 0.01 * exp(-avg_h / 120.0));
    let fa = 1.0 - exp(-d * (0.00002 + frame.sky.z * 0.00007));
    let f = 1.0 - (1.0 - fh) * (1.0 - fa);
    let v = normalize(wp - frame.cam_pos.xyz);
    let fog_col = sky_base(normalize(vec3<f32>(v.x, max(v.y, 0.02), v.z))) * frame.sun_color.a;
    return mix(c, fog_col, clamp(f, 0.0, 1.0));
}

// ---------------------------------------------------------------- traversal
struct Hit {
    t: f32,
    tri: u32,
    u: f32,
    v: f32,
    // Shadow rays: light let through by glass/water passed on the way.
    trans: vec3<f32>,
    // Shadow rays: solid-angle pdf with which window sampling (portals)
    // would have chosen this direction (sum over the glass crossed).
    portal_pdf: f32,
};

// Traversal stack size. The BVH builder bounds the tree depth by
// bvh::MAX_DEPTH (63), and this traversal holds at most one deferred
// sibling per level plus the node being visited, so 64 entries can never
// overflow (tested in bvh.rs and tests/pathtrace_bvh.rs).
const STACK_SIZE: i32 = 64;

fn ray_box(o: vec3<f32>, inv: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>, tmax: f32) -> f32 {
    let t0 = (lo - o) * inv;
    let t1 = (hi - o) * inv;
    let tn = max(max(min(t0.x, t1.x), min(t0.y, t1.y)), min(t0.z, t1.z));
    let tf = min(min(max(t0.x, t1.x), max(t0.y, t1.y)), max(t0.z, t1.z));
    if (tf >= max(tn, 0.0) && tn < tmax) {
        return max(tn, 0.0);
    }
    return INF;
}

fn box_uv(p: vec3<f32>, n: vec3<f32>) -> vec2<f32> {
    let a = abs(n);
    if (a.y >= a.x && a.y >= a.z) {
        return vec2<f32>(p.x, -p.z * sign(n.y + 1e-6));
    } else if (a.x >= a.z) {
        return vec2<f32>(-p.z * sign(n.x), -p.y);
    }
    return vec2<f32>(p.x * sign(n.z), -p.y);
}

// The three corner UVs of a triangle (model UVs, or real-world box mapping).
fn corner_uvs(tri: vec4<u32>, m: Mat) -> array<vec2<f32>, 3> {
    if (m.p5.w > 0.5) {
        let p0 = positions[tri.x].xyz;
        let p1 = positions[tri.y].xyz;
        let p2 = positions[tri.z].xyz;
        let n = normalize(cross(p1 - p0, p2 - p0));
        // As seen from the front: the rasterizer orients by the view side,
        // so use the vertex normal's side to match.
        let s = select(-1.0, 1.0, dot(n, vert_normal(tri.x)) >= 0.0);
        return array<vec2<f32>, 3>(box_uv(p0, n * s), box_uv(p1, n * s), box_uv(p2, n * s));
    }
    return array<vec2<f32>, 3>(attrs[tri.x].yz, attrs[tri.y].yz, attrs[tri.z].yz);
}

fn tri_uv(tri: vec4<u32>, u: f32, v: f32) -> vec2<f32> {
    let c = corner_uvs(tri, mats[tri.w]);
    return c[0] * (1.0 - u - v) + c[1] * u + c[2] * v;
}

fn mat_uv(m: Mat, uv_in: vec2<f32>) -> vec2<f32> {
    let uv0 = (uv_in * m.uvt.xy + m.uvt.zw) * m.p0.w;
    let rc = cos(m.p3.x);
    let rs = sin(m.p3.x);
    return vec2<f32>(uv0.x * rc - uv0.y * rs, uv0.x * rs + uv0.y * rc) + m.p3.yz;
}

// Alpha cut-outs (leaves, fences) let rays through where the texture is clear.
fn cut_away(tri: vec4<u32>, u: f32, v: f32) -> bool {
    // Most candidate hits are on opaque materials: test the cut-out flag
    // before loading the rest of the material.
    if (mats[tri.w].p5.y < 0.5) {
        return false;
    }
    let m = mats[tri.w];
    if (m.p1.x < 0.0) {
        return false;
    }
    let uv = mat_uv(m, tri_uv(tri, u, v));
    // Alpha test at a coarse-ish level (~128 px across a tile), as fine
    // levels make thin leaves flicker between samples.
    let a = color_map(m.p1.x, uv, map_level(m.p1.x, -7.0)).a;
    return a < 0.5;
}

// shadow = true: any opaque hit before tmax ends the search; glass, water
// and cut-out holes let the light through.
fn trace(o: vec3<f32>, d: vec3<f32>, tmax: f32, shadow: bool) -> Hit {
    var hit: Hit;
    hit.t = tmax;
    hit.tri = 0xffffffffu;
    hit.trans = vec3<f32>(1.0);
    hit.portal_pdf = 0.0;
    // Keep the sign when replacing near-zero components (1/±0 -> ±large).
    let inv = 1.0 / select(d, sign(d + vec3<f32>(1e-30)) * 1e-12, abs(d) < vec3<f32>(1e-12));
    var stack: array<u32, STACK_SIZE>;
    var sp = 0;
    stack[0] = 0u;
    sp = 1;
    loop {
        if (sp == 0) {
            break;
        }
        sp = sp - 1;
        let n = nodes[stack[sp]];
        if (ray_box(o, inv, n.lo, n.hi, hit.t) == INF) {
            continue;
        }
        if (n.b > 0u) {
            for (var k = n.a; k < n.a + n.b; k = k + 1u) {
                let tri = tris[k];
                let p0 = positions[tri.x].xyz;
                let e1 = positions[tri.y].xyz - p0;
                let e2 = positions[tri.z].xyz - p0;
                let pv = cross(d, e2);
                let det = dot(e1, pv);
                // Only exactly-degenerate triangles are skipped: a fixed
                // threshold also rejected small details (mm-scale trim).
                if (det == 0.0) {
                    continue;
                }
                let idet = 1.0 / det;
                let s = o - p0;
                let u = dot(s, pv) * idet;
                // Slightly inclusive edge tests close the hairline cracks
                // between neighbouring triangles (light leaks at seams).
                if (u < -1e-6 || u > 1.000001) {
                    continue;
                }
                let q = cross(s, e1);
                let v = dot(d, q) * idet;
                if (v < -1e-6 || u + v > 1.000001) {
                    continue;
                }
                let t = dot(e2, q) * idet;
                if (t <= 1e-6 || t >= hit.t) {
                    continue;
                }
                // Read only the fields needed: this runs for every candidate
                // hit, and copying the whole material struct is wasted work.
                let kind = u32(mats[tri.w].p0.z);
                if (shadow && (kind == 1u || kind == 2u)) {
                    let base = mats[tri.w].base;
                    // Glass/water transmit light, tinted, minus Fresnel reflection.
                    let gn = normalize(cross(e1, e2));
                    let c = abs(dot(gn, d));
                    let f0 = select(0.04, 0.02, kind == 2u);
                    let fr = f0 + (1.0 - f0) * pow(1.0 - c, 5.0);
                    hit.trans = hit.trans * (1.0 - fr) * mix(vec3<f32>(1.0), base.rgb, clamp(base.a, 0.0, 1.0) * 0.6);
                    if (kind == 1u && pt.opts.w > 0.0) {
                        hit.portal_pdf = hit.portal_pdf + t * t / (max(c, 1e-4) * pt.opts.w);
                    }
                    continue;
                }
                if (cut_away(tri, u, v)) {
                    continue;
                }
                hit.t = t;
                hit.tri = k;
                hit.u = u;
                hit.v = v;
                if (shadow) {
                    return hit;
                }
            }
        } else {
            // Visit the nearer child first.
            let l = nodes[n.a];
            let r = nodes[n.a + 1u];
            let tl = ray_box(o, inv, l.lo, l.hi, hit.t);
            let tr = ray_box(o, inv, r.lo, r.hi, hit.t);
            // No capacity check needed (see STACK_SIZE): previously a full
            // stack silently dropped both children, i.e. lost geometry.
            if (tl < tr) {
                if (tr != INF) { stack[sp] = n.a + 1u; sp = sp + 1; }
                if (tl != INF) { stack[sp] = n.a; sp = sp + 1; }
            } else {
                if (tl != INF) { stack[sp] = n.a; sp = sp + 1; }
                if (tr != INF) { stack[sp] = n.a + 1u; sp = sp + 1; }
            }
        }
    }
    return hit;
}

// ---------------------------------------------------------------- shading
struct Surface {
    p: vec3<f32>,
    n: vec3<f32>,      // shading normal (facing the ray origin)
    ng: vec3<f32>,     // geometric normal (facing the ray origin)
    albedo: vec3<f32>,
    rough: f32,
    metal: f32,
    f0: vec3<f32>,
    kind: u32,
    opacity: f32,
    emission: vec3<f32>,
    front: bool,       // ray hit the side the vertex normal points to
};

fn oct_decode(e: vec2<f32>) -> vec3<f32> {
    var v = vec3<f32>(e.x, e.y, 1.0 - abs(e.x) - abs(e.y));
    if (v.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), v.xy >= vec2<f32>(0.0));
        v = vec3<f32>((1.0 - abs(v.yx)) * s, v.z);
    }
    return normalize(v);
}

fn vert_normal(i: u32) -> vec3<f32> {
    return oct_decode(unpack2x16snorm(bitcast<u32>(attrs[i].x)));
}

fn surface(o: vec3<f32>, d: vec3<f32>, h: Hit, path_len: f32) -> Surface {
    var s: Surface;
    let tri = tris[h.tri];
    let m = mats[tri.w];
    let w = 1.0 - h.u - h.v;
    let p0 = positions[tri.x].xyz;
    let p1 = positions[tri.y].xyz;
    let p2 = positions[tri.z].xyz;
    s.p = o + d * h.t;
    var ng = normalize(cross(p1 - p0, p2 - p0));
    var n = normalize(vert_normal(tri.x) * w + vert_normal(tri.y) * h.u + vert_normal(tri.z) * h.v);
    s.front = dot(n, d) < 0.0;
    if (dot(ng, d) > 0.0) {
        ng = -ng;
    }
    if (dot(n, ng) < 0.0) {
        n = -n;
    }
    let cuv = corner_uvs(tri, m);
    let uva = cuv[0];
    let uvb = cuv[1];
    let uvc = cuv[2];
    let uv = mat_uv(m, uva * w + uvb * h.u + uvc * h.v);
    // Texture level of detail from the ray footprint (ray-cone style).
    let tl = mat_uv(m, uvb) - mat_uv(m, uva);
    let tr = mat_uv(m, uvc) - mat_uv(m, uva);
    let uv_area = abs(tl.x * tr.y - tl.y * tr.x);
    let world_area = length(cross(p1 - p0, p2 - p0));
    let uv_per_m = sqrt(uv_area / max(world_area, 1e-12));
    let footprint = path_len * pt.opts.z / max(abs(dot(d, ng)), 0.15);
    // log2 of the pixel footprint in UV units; each map adds its own size.
    let lod_uv = log2(max(footprint * uv_per_m, 1e-9));

    var albedo = m.base.rgb;
    var opacity = m.base.a;
    if (m.p1.x >= 0.0) {
        let tex = color_map(m.p1.x, uv, map_level(m.p1.x, lod_uv));
        albedo = albedo * mix(vec3<f32>(1.0), tex.rgb, clamp(m.p3.w, 0.0, 1.0));
        if (m.p5.y < 0.5) {
            opacity = opacity * tex.a;
        }
    }
    if (m.p1.y >= 0.0) {
        // Tangent frame from the triangle's UV layout.
        let det = tl.x * tr.y - tl.y * tr.x;
        if (abs(det) > 1e-12) {
            let e1 = p1 - p0;
            let e2 = p2 - p0;
            let tg = (e1 * tr.y - e2 * tl.y) / det;
            let bt = (e2 * tl.x - e1 * tr.x) / det;
            var t3 = normalize(tg - n * dot(n, tg));
            let b3 = normalize(bt - n * dot(n, bt));
            var tn = data_map(m.p1.y, uv, map_level(m.p1.y, lod_uv)).xyz * 2.0 - 1.0;
            tn = vec3<f32>(tn.x * m.p1.w, -tn.y * m.p1.w, tn.z);
            let pn = normalize(t3 * tn.x + b3 * tn.y + n * tn.z);
            if (dot(pn, ng) > 0.02) {
                n = pn;
            }
        }
    }
    var rough = clamp(m.p0.y, 0.03, 1.0);
    if (m.p1.z >= 0.0) {
        let r = data_map(m.p1.z, uv, map_level(m.p1.z, lod_uv)).r;
        rough = clamp(r + (m.p0.y - 0.5), 0.03, 1.0);
    }
    let kind = u32(m.p0.z);
    // Procedural detail, as in the rasterizer.
    let cam_dist = distance(s.p, frame.cam_pos.xyz);
    let var_amt = m.p5.x * (1.0 - smoothstep(120.0, 250.0, cam_dist));
    if (var_amt > 0.0) {
        let wp2 = s.p.xz + s.p.y * 0.37;
        let nz = fbm3(wp2 * 1.3) * 0.7 + vnoise(wp2 * 9.0) * 0.3;
        albedo = albedo * (1.0 + (nz - 0.5) * var_amt * 0.9);
        rough = clamp(rough + (nz - 0.5) * var_amt * 0.2, 0.03, 1.0);
    }
    if (kind == 5u) {
        let pp = s.p.xz;
        let big = fbm3(pp * 0.05);
        let mid = fbm3(pp * 0.6);
        let fine = vnoise(pp * 14.0);
        let dry = albedo * vec3<f32>(1.6, 1.35, 0.8);
        albedo = mix(albedo, dry, smoothstep(0.45, 0.8, big) * m.p5.x);
        albedo = albedo * (0.7 + 0.45 * mid + 0.25 * (fine - 0.5) * var_amt);
    }
    if (kind == 6u) {
        let pp = s.p;
        let nz = fbm3(pp.xz * 0.0035);
        let g0 = fbm3(pp.xz * 0.02);
        let slope = 1.0 - clamp(n.y, 0.0, 1.0);
        let fine = vnoise(pp.xz * 0.05);
        let rock = vec3<f32>(0.30, 0.29, 0.27) * (0.65 + 0.7 * g0);
        let forest = vec3<f32>(0.028, 0.05, 0.025) * (0.55 + 0.9 * fine);
        let meadow = vec3<f32>(0.11, 0.13, 0.05) * (0.7 + 0.6 * fine);
        var c = mix(forest, meadow, smoothstep(0.55, 0.85, nz) * 0.6);
        let tree_line = 300.0 + nz * 260.0;
        c = mix(c, rock, max(smoothstep(tree_line - 60.0, tree_line + 80.0, pp.y), smoothstep(0.42, 0.62, slope)));
        let snow_line = 700.0 + nz * 220.0;
        c = mix(c, vec3<f32>(0.82, 0.84, 0.88), smoothstep(snow_line, snow_line + 120.0, pp.y) * (1.0 - smoothstep(0.5, 0.72, slope)));
        albedo = c;
        rough = 0.9;
    }
    if (kind == 4u && m.p1.x < 0.0) {
        // grass blades: darker at the root
        let uvg = uva * w + uvb * h.u + uvc * h.v;
        albedo = albedo * mix(vec3<f32>(0.6, 0.65, 0.6), vec3<f32>(1.15, 1.12, 0.95), uvg.y);
    }
    if (kind == 2u) {
        let t = frame.cam_pos.w;
        let pp = s.p.xz;
        let dx = cos(pp.x * 1.3 + t * 1.1) * 0.06 + cos(pp.x * 3.7 + pp.y * 1.9 + t * 2.3) * 0.03 + (vnoise(pp * 4.0 + t * 0.6) - 0.5) * 0.08;
        let dz = cos(pp.y * 1.7 + t * 0.9) * 0.06 + cos(pp.y * 4.1 - pp.x * 1.3 + t * 2.7) * 0.03 + (vnoise(pp * 4.0 - t * 0.5 + 7.0) - 0.5) * 0.08;
        n = normalize(n + vec3<f32>(dx, 0.0, dz));
    }
    s.n = n;
    s.ng = ng;
    s.albedo = clamp(albedo, vec3<f32>(0.0), vec3<f32>(1.0));
    s.rough = rough;
    s.metal = m.p0.x;
    s.f0 = mix(vec3<f32>(0.08 * m.p4.w), s.albedo, s.metal);
    s.kind = kind;
    s.opacity = opacity;
    s.emission = m.p4.rgb;
    return s;
}

// ---------------------------------------------------------------- BRDF
fn f_schlick(c: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - clamp(c, 0.0, 1.0), 5.0);
}

fn d_ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d + 1e-7);
}

fn g1(nx: f32, a: f32) -> f32 {
    let a2 = a * a;
    return 2.0 * nx / (nx + sqrt(a2 + (1.0 - a2) * nx * nx));
}

fn basis(n: vec3<f32>) -> mat3x3<f32> {
    let s = select(-1.0, 1.0, n.z >= 0.0);
    let a = -1.0 / (s + n.z);
    let b = n.x * n.y * a;
    let t = vec3<f32>(1.0 + s * n.x * n.x * a, s * b, -s * n.x);
    let bt = vec3<f32>(b, s + n.y * n.y * a, -n.y);
    return mat3x3<f32>(t, bt, n);
}

fn cosine_dir(n: vec3<f32>) -> vec3<f32> {
    let r1 = rnd();
    let r2 = rnd();
    let r = sqrt(r1);
    let phi = 2.0 * PI * r2;
    return normalize(basis(n) * vec3<f32>(r * cos(phi), r * sin(phi), sqrt(max(1.0 - r1, 0.0))));
}

// GGX visible-normal sampling (Heitz 2018), in the frame of n.
fn sample_vndf(n: vec3<f32>, v: vec3<f32>, a: f32) -> vec3<f32> {
    let tb = basis(n);
    let vl = vec3<f32>(dot(v, tb[0]), dot(v, tb[1]), dot(v, tb[2]));
    let vh = normalize(vec3<f32>(a * vl.x, a * vl.y, vl.z));
    let lensq = vh.x * vh.x + vh.y * vh.y;
    let t1 = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-vh.y, vh.x, 0.0) / sqrt(lensq), lensq > 0.0);
    let t2 = cross(vh, t1);
    let r = sqrt(rnd());
    let phi = 2.0 * PI * rnd();
    let p1 = r * cos(phi);
    var p2 = r * sin(phi);
    let s = 0.5 * (1.0 + vh.z);
    p2 = (1.0 - s) * sqrt(max(1.0 - p1 * p1, 0.0)) + s * p2;
    let nh = p1 * t1 + p2 * t2 + sqrt(max(0.0, 1.0 - p1 * p1 - p2 * p2)) * vh;
    let hl = normalize(vec3<f32>(a * nh.x, a * nh.y, max(0.0, nh.z)));
    return normalize(tb * hl);
}

// Single-scattering GGX loses energy at high roughness (rough metals and
// stone look too dark). Directional albedo from Karis' analytic fit gives
// the multiple-scattering compensation 1 + f0 (1/E - 1) (Fdez-Aguera 2019).
fn ggx_energy_comp(f0: vec3<f32>, nv: f32, rough: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = rough * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    let e = clamp(ab.x + ab.y, 0.05, 1.0);
    return vec3<f32>(1.0) + f0 * (1.0 / e - 1.0);
}

// Probability of choosing the specular lobe when sampling a bounce.
fn spec_prob(s: Surface, nv: f32) -> f32 {
    let fv = f_schlick(nv, s.f0);
    return clamp(select(lum(fv) * 2.0 + 0.1, 1.0, s.metal > 0.5), 0.05, 1.0) * (1.0 - smoothstep(0.85, 1.0, s.rough) * 0.5);
}

// Solid-angle pdf of the bounce sampling (VNDF specular + cosine diffuse).
fn bsdf_pdf(s: Surface, v: vec3<f32>, l: vec3<f32>) -> f32 {
    let nl = dot(s.n, l);
    if (nl <= 0.0) {
        return 0.0;
    }
    let nv = max(dot(s.n, v), 1e-4);
    let a = max(s.rough * s.rough, 0.002);
    let h = normalize(v + l);
    let ps = spec_prob(s, nv);
    let pdf_spec = d_ggx(max(dot(s.n, h), 0.0), a) * g1(nv, a) / (4.0 * nv);
    var pdf = ps * pdf_spec + (1.0 - ps) * nl / PI;
    if (s.kind == 4u) {
        pdf = pdf * 0.7;
    }
    return pdf;
}

// Diffuse + specular reflectance toward l (times n·l).
fn brdf_cos(s: Surface, v: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    let nl = dot(s.n, l);
    if (nl <= 0.0 || dot(s.ng, l) <= 0.0) {
        return vec3<f32>(0.0);
    }
    let nv = max(dot(s.n, v), 1e-4);
    let h = normalize(v + l);
    let a = max(s.rough * s.rough, 0.002);
    let F = f_schlick(dot(v, h), s.f0);
    let spec = F * d_ggx(max(dot(s.n, h), 0.0), a) * g1(nv, a) * g1(nl, a) / (4.0 * nv * nl + 1e-6) * ggx_energy_comp(s.f0, nv, s.rough);
    let kd = (vec3<f32>(1.0) - F) * (1.0 - s.metal);
    return (kd * s.albedo / PI + spec) * nl;
}

fn lum(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Direct light from the sun (soft disc) and one random local light.
// `last`: no bounce ray will be traced from this vertex, so light sampled
// here takes full weight (MIS against a strategy that won't run would lose
// its share).
fn direct(s: Surface, v: vec3<f32>, last: bool) -> vec3<f32> {
    var out = vec3<f32>(0.0);
    let o = offset_ray(s.p, s.ng);
    let e_sun = sun_irradiance();
    if (lum(e_sun) > 0.0) {
        // Uniform sample of the sun's cone.
        let cos_max = cos(sun_radius());
        let ct = 1.0 - rnd() * (1.0 - cos_max);
        let st = sqrt(max(1.0 - ct * ct, 0.0));
        let phi = 2.0 * PI * rnd();
        let l = normalize(basis(frame.sun_dir.xyz) * vec3<f32>(st * cos(phi), st * sin(phi), ct));
        var f = brdf_cos(s, v, l);
        // MIS against bounce rays that find the sun (sun_hit): glossy and
        // mirror-like surfaces get clean highlights from whichever is better.
        if (!last) {
            f = f * power_heuristic(1.0 / sun_solid_angle(), bsdf_pdf(s, v, l));
        }
        if (s.kind == 4u) {
            // leaves and grass let some sunlight through
            let back = max(-dot(s.n, l), 0.0);
            f = f + s.albedo * vec3<f32>(1.0, 1.1, 0.6) * back * 0.25;
        }
        if (lum(f) > 0.0) {
            // Shadow rays from the back side for translucent foliage.
            let so = select(o, offset_ray(s.p, -s.ng), dot(l, s.ng) < 0.0);
            let h = trace(so, l, INF, true);
            if (h.tri == 0xffffffffu) {
                out = out + f * e_sun * h.trans;
            }
        }
    }
    // Sky through windows: sample a point on the glass (area-weighted) and
    // look past it. Indoors this finds the sky far more often than random
    // bounces; MIS with bounce rays (see the main loop) keeps it unbiased.
    if (pt.portal.y > 0u && pt.opts.w > 0.0) {
        let pick = rnd() * pt.opts.w;
        var lo = 0u;
        var hi = pt.portal.y - 1u;
        loop {
            if (lo >= hi) {
                break;
            }
            let mid = (lo + hi) / 2u;
            if (nodes[pt.portal.x + mid].lo.x < pick) {
                lo = mid + 1u;
            } else {
                hi = mid;
            }
        }
        let tri = tris[nodes[pt.portal.x + lo].a];
        let su = sqrt(rnd());
        let b1 = rnd() * su;
        let q = positions[tri.x].xyz * (1.0 - su) + positions[tri.y].xyz * b1 + positions[tri.z].xyz * (su - b1);
        let l = normalize(q - s.p);
        if (dot(l, s.ng) > 0.0) {
            let f = brdf_cos(s, v, l);
            if (lum(f) > 0.0) {
                let h = trace(o, l, INF, true);
                if (h.tri == 0xffffffffu && h.portal_pdf > 0.0) {
                    let w = select(power_heuristic(h.portal_pdf, bsdf_pdf(s, v, l)), 1.0, last);
                    out = out + f * sky_light(l) * h.trans * (w / h.portal_pdf);
                }
            }
        }
    }
    let count = u32(frame.cam_fwd.w);
    if (count > 0u) {
        let li = lights[min(u32(rnd() * f32(count)), count - 1u)];
        let lv = li.pos_range.xyz - s.p;
        let d2 = max(dot(lv, lv), 1e-4);
        let range = li.pos_range.w;
        if (d2 < range * range) {
            let d = sqrt(d2);
            let l = lv / d;
            let x = d / range;
            let win = clamp(1.0 - x * x * x * x, 0.0, 1.0);
            var atten = win * win / (d2 + 0.25);
            if (li.color.w > -1.5) {
                atten = atten * smoothstep(li.color.w, li.dir.w, dot(-l, li.dir.xyz));
            }
            let f = brdf_cos(s, v, l);
            if (atten > 0.0 && lum(f) > 0.0) {
                let h = trace(o, l, d - 0.05, true);
                if (h.tri == 0xffffffffu) {
                    out = out + f * li.color.rgb * atten * PI * f32(count) * h.trans;
                }
            }
        }
    }
    return out;
}

// A bounce ray (sampled with pdf `pdf`) that escapes toward the sun: its
// share of the sun's light under the power heuristic.
fn sun_hit(d: vec3<f32>, pdf: f32) -> vec3<f32> {
    if (pdf <= 0.0 || dot(d, frame.sun_dir.xyz) < cos(sun_radius())) {
        return vec3<f32>(0.0);
    }
    let pl = 1.0 / sun_solid_angle();
    return sun_irradiance() * pl * power_heuristic(pdf, pl);
}

struct Sample {
    radiance: vec3<f32>,
    albedo_t: vec4<f32>,
    normal: vec3<f32>,
};

fn sample_pixel(px: vec2<u32>, w: u32, h: u32) -> Sample {

    // Camera ray with sub-pixel jitter (antialiasing).
    var jitter = vec2<f32>(rnd(), rnd());
    if (pt.lens.w > 0.5) {
        jitter = vec2<f32>(0.5); // deterministic pixel-centre rays (tests)
    }
    let uv = (vec2<f32>(px) + jitter) / vec2<f32>(f32(w), f32(h));
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let far = frame.inv_view_proj * vec4<f32>(ndc, 0.5, 1.0);
    let near = frame.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let o0 = near.xyz / near.w;
    var o = o0;
    var d = normalize(far.xyz / far.w - o0);
    if (pt.lens.x > 0.0) {
        // Thin-lens camera: real depth of field with round bokeh.
        let fwd = normalize(frame.cam_fwd.xyz);
        let focal = o0 + d * (pt.lens.y / max(dot(d, fwd), 0.05));
        let right = normalize(cross(fwd, vec3<f32>(0.0, 1.0, 0.0)));
        let up = cross(right, fwd);
        let r = sqrt(rnd()) * pt.lens.x;
        let a = rnd() * 2.0 * PI;
        o = o0 + (right * cos(a) + up * sin(a)) * r;
        d = normalize(focal - o);
    }

    var radiance = vec3<f32>(0.0);
    var through = vec3<f32>(1.0);
    var first_albedo = vec3<f32>(0.0);
    var first_normal = vec3<f32>(0.0);
    var first_t = 1e4;
    var have_aov = false;
    var path_len = 0.0;
    var primary_p = vec3<f32>(0.0);
    var hit_any = false;
    let max_b = u32(pt.opts.x);
    // True while every bounce so far was a delta event (camera, glass,
    // water, Fresnel mirror): such rays see the visible sun disk. After a
    // surface that did next-event estimation, the sun is counted with MIS.
    var specular_path = true;
    var last_pdf = 0.0;
    // Window-sampling MIS bookkeeping for the current bounce ray.
    var portal_track = false;
    var portal_pdf = 0.0;
    var seg_t = 0.0;

    // `bounce` counts scattering at surfaces only: passing through glass or
    // water (and Fresnel reflections off them) must not use up the path
    // budget, or light reaching a room through a double-sided pane at the
    // last bounces is lost (interiors came out too dark). `events` caps the
    // total so a ray trapped between panes still terminates.
    var bounce = 0u;
    var events = 0u;
    loop {
        if (bounce > max_b || events > max_b + 32u) {
            break;
        }
        events = events + 1u;
        let hh = trace(o, d, INF, false);
        if (hh.tri == 0xffffffffu) {
            var sky: vec3<f32>;
            if (specular_path) {
                sky = background(d);
            } else {
                // Share the sky seen through windows with window sampling.
                var w_sky = 1.0;
                if (portal_track && portal_pdf > 0.0 && last_pdf > 0.0) {
                    w_sky = power_heuristic(last_pdf, portal_pdf);
                }
                sky = sky_light(d) * w_sky + sun_hit(d, last_pdf);
            }
            var add = through * sky;
            if (bounce > 1u) {
                let l = lum(add);
                if (l > pt.opts.y) {
                    add = add * (pt.opts.y / l);
                }
            }
            radiance = radiance + add;
            if (!have_aov) {
                first_albedo = sky / max(lum(sky), 1e-3) * 0.5;
                first_normal = -d;
                have_aov = true;
            }
            break;
        }
        path_len = path_len + hh.t;
        seg_t = seg_t + hh.t;
        let s = surface(o, d, hh, path_len);
        if (!hit_any) {
            primary_p = s.p;
            hit_any = true;
        }
        let v = -d;
        // Glass and water: choose reflection or transmission by Fresnel.
        if (s.kind == 1u || s.kind == 2u) {
            let cosi = abs(dot(v, s.n));
            let F = f_schlick(cosi, vec3<f32>(select(0.04, 0.02, s.kind == 2u))).x;
            if (rnd() < F) {
                d = reflect(d, s.n);
                o = offset_ray(s.p, s.ng);
                portal_track = false; // no longer a straight line from the last vertex
                continue;
            }
            if (s.kind == 1u) {
                // Thin architectural glass: passes straight through, lightly tinted.
                through = through * mix(vec3<f32>(1.0), s.albedo, clamp(s.opacity, 0.0, 1.0) * 0.6);
                o = offset_ray(s.p, -s.ng);
                if (portal_track) {
                    portal_pdf = portal_pdf + seg_t * seg_t / (max(abs(dot(s.ng, d)), 1e-4) * pt.opts.w);
                }
                continue;
            }
            portal_track = false; // water refracts: not a straight line
            // Water: refraction (n = 1.33) and absorption by its colour.
            let eta = select(1.33, 1.0 / 1.33, s.front);
            let rd = refract(d, s.n, eta);
            if (dot(rd, rd) < 1e-6) {
                d = reflect(d, s.n);
                o = offset_ray(s.p, s.ng);
            } else {
                d = normalize(rd);
                o = offset_ray(s.p, -s.ng);
                if (s.front) {
                    // Absorption: the water's colour, normalised so it tints rather than darkens.
                    let tint = s.albedo / max(max(s.albedo.r, max(s.albedo.g, s.albedo.b)), 1e-3);
                    through = through * mix(vec3<f32>(1.0), tint * 0.85, clamp(s.opacity, 0.0, 1.0) * 0.9 + 0.1);
                }
            }
            continue;
        }
        if (!have_aov) {
            first_albedo = s.albedo;
            first_normal = s.n;
            first_t = path_len;
            have_aov = true;
        }
        // Emission
        if (lum(s.emission) > 0.0) {
            radiance = radiance + through * s.emission;
        }
        // Direct light (next-event estimation)
        var dl = through * direct(s, v, bounce == max_b);
        if (bounce > 0u) {
            let l = lum(dl);
            if (l > pt.opts.y) {
                dl = dl * (pt.opts.y / l);
            }
        }
        radiance = radiance + dl;
        if (bounce == max_b) {
            break;
        }
        // Next direction
        let nv = max(dot(s.n, v), 1e-4);
        let Fv = f_schlick(nv, s.f0);
        let p_spec = spec_prob(s, nv);
        let a = max(s.rough * s.rough, 0.002);
        specular_path = false;
        if (s.kind == 4u && rnd() < 0.3) {
            // foliage translucency (its sunlight comes from direct(); no MIS)
            d = cosine_dir(-s.n);
            o = offset_ray(s.p, -s.ng);
            through = through * s.albedo * vec3<f32>(1.0, 1.1, 0.6) * 0.9;
            last_pdf = 0.0;
        } else if (rnd() < p_spec) {
            let hv = sample_vndf(s.n, v, a);
            let l = reflect(-v, hv);
            if (dot(l, s.ng) <= 0.0 || dot(l, s.n) <= 0.0) {
                break;
            }
            let F = f_schlick(dot(v, hv), s.f0);
            through = through * F * g1(max(dot(s.n, l), 1e-4), a) * ggx_energy_comp(s.f0, nv, s.rough) / p_spec;
            last_pdf = bsdf_pdf(s, v, l);
            d = l;
            o = offset_ray(s.p, s.ng);
        } else {
            let l = cosine_dir(s.n);
            if (dot(l, s.ng) <= 0.0) {
                break;
            }
            let kd = (vec3<f32>(1.0) - Fv) * (1.0 - s.metal);
            through = through * kd * s.albedo / (1.0 - p_spec);
            last_pdf = bsdf_pdf(s, v, l);
            d = l;
            o = offset_ray(s.p, s.ng);
        }
        // (Foliage: 30% of light is transmitted, 70% reflected, so neither
        // branch is divided by its selection probability.)
        portal_track = pt.portal.y > 0u && last_pdf > 0.0;
        portal_pdf = 0.0;
        seg_t = 0.0;
        // Russian roulette
        if (bounce >= 3u) {
            let q = clamp(max(through.x, max(through.y, through.z)), 0.05, 0.95);
            if (rnd() > q) {
                break;
            }
            through = through / q;
        }
        bounce = bounce + 1u;
    }
    if (hit_any) {
        radiance = apply_fog(radiance, primary_p);
    }
    if (any(radiance != radiance)) {
        radiance = vec3<f32>(0.0); // drop NaNs
    }
    var out: Sample;
    out.radiance = radiance;
    out.albedo_t = vec4<f32>(first_albedo, first_t);
    out.normal = first_normal;
    return out;
}

// ---------------------------------------------------------------- main
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= pt.tile.z || gid.y >= pt.tile.w) {
        return;
    }
    let px = vec2<u32>(gid.x + pt.tile.x, gid.y + pt.tile.y);
    let w = pt.size.x;
    let h = pt.size.y;
    if (px.x >= w || px.y >= h) {
        return;
    }
    let idx = px.y * w + px.x;
    rng_state = (idx * 1973u + pt.size.z * 9277u + 26699u) | 1u;
    rng_state = pcg() ^ (pt.size.z * 0x9E3779B9u);
    let count = max(u32(pt.lens.z), 1u);
    var sum = vec3<f32>(0.0);
    var sum_alb = vec4<f32>(0.0);
    var sum_nrm = vec3<f32>(0.0);
    for (var si = 0u; si < count; si = si + 1u) {
        let r = sample_pixel(px, w, h);
        sum = sum + r.radiance;
        sum_alb = sum_alb + r.albedo_t;
        sum_nrm = sum_nrm + r.normal;
    }
    let n = w * h;
    accum[idx] = accum[idx] + vec4<f32>(sum, f32(count));
    aux[idx] = aux[idx] + sum_alb;
    aux[n + idx] = aux[n + idx] + vec4<f32>(sum_nrm, 0.0);
}

