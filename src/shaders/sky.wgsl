// Physically based sky: single-scattering Rayleigh + Mie atmosphere,
// ray-marched into a small direction LUT once per frame. Everything that
// needs sky colour (background, reflections, ambient light, fog) samples it.

struct Frame {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    light_vp: array<mat4x4<f32>, 4>,
    cascade_far: vec4<f32>,
    cascade_texel: vec4<f32>,
    cam_pos: vec4<f32>,
    cam_fwd: vec4<f32>,
    sun_dir: vec4<f32>,     // xyz (towards sun), intensity
    sun_color: vec4<f32>,   // rgb, ambient strength
    sky: vec4<f32>,         // cloudiness, fog density, haze, shadow softness
    post: vec4<f32>,
    misc: vec4<f32>,
    extra: vec4<f32>,
};
@group(0) @binding(0) var<uniform> frame: Frame;

const PI: f32 = 3.14159265;
const R_PLANET: f32 = 6360000.0;
const R_ATMOS: f32 = 6420000.0;
const H_R: f32 = 8000.0;
const H_M: f32 = 1200.0;

fn ray_sphere(o: vec3<f32>, d: vec3<f32>, r: f32) -> vec2<f32> {
    let b = dot(o, d);
    let c = dot(o, o) - r * r;
    let disc = b * b - c;
    if (disc < 0.0) {
        return vec2<f32>(-1.0, -1.0);
    }
    let s = sqrt(disc);
    return vec2<f32>(-b - s, -b + s);
}

fn atmosphere(dir: vec3<f32>, sun: vec3<f32>) -> vec3<f32> {
    let haze = frame.sky.z;
    let beta_r = vec3<f32>(5.8e-6, 13.5e-6, 33.1e-6);
    let beta_m = vec3<f32>(21e-6) * (1.0 + haze * 5.0);
    let o = vec3<f32>(0.0, R_PLANET + 10.0, 0.0);
    var t_max = ray_sphere(o, dir, R_ATMOS).y;
    let ground = ray_sphere(o, dir, R_PLANET);
    if (ground.x > 0.0) {
        t_max = ground.x;
    }
    let n = 16;
    let seg = t_max / f32(n);
    var od_r = 0.0;
    var od_m = 0.0;
    var sum_r = vec3<f32>(0.0);
    var sum_m = vec3<f32>(0.0);
    let mu = dot(dir, sun);
    let g = 0.76;
    let phase_r = 3.0 / (16.0 * PI) * (1.0 + mu * mu);
    let phase_m = 3.0 / (8.0 * PI) * ((1.0 - g * g) * (1.0 + mu * mu)) / ((2.0 + g * g) * pow(max(1.0 + g * g - 2.0 * g * mu, 1e-4), 1.5));
    for (var i = 0; i < n; i = i + 1) {
        let p = o + dir * (seg * (f32(i) + 0.5));
        let h = length(p) - R_PLANET;
        let hr = exp(-h / H_R) * seg;
        let hm = exp(-h / H_M) * seg;
        od_r = od_r + hr;
        od_m = od_m + hm;
        // light path to the sun
        let ls = ray_sphere(p, sun, R_ATMOS).y;
        if (ray_sphere(p, sun, R_PLANET).x > 0.0) {
            continue; // in the planet's shadow
        }
        let lseg = ls / 4.0;
        var lr = 0.0;
        var lm = 0.0;
        for (var j = 0; j < 4; j = j + 1) {
            let q = p + sun * (lseg * (f32(j) + 0.5));
            let qh = length(q) - R_PLANET;
            lr = lr + exp(-qh / H_R) * lseg;
            lm = lm + exp(-qh / H_M) * lseg;
        }
        let tau = beta_r * (od_r + lr) + beta_m * 1.1 * (od_m + lm);
        let att = exp(-tau);
        sum_r = sum_r + att * hr;
        sum_m = sum_m + att * hm;
    }
    return (sum_r * beta_r * phase_r + sum_m * beta_m * phase_m) * 22.0;
}

// LUT layout: u = azimuth, v = elevation (square-root mapped so the horizon
// gets most of the resolution).
fn lut_dir(uv: vec2<f32>) -> vec3<f32> {
    let az = (uv.x - 0.5) * 2.0 * PI;
    let s = uv.y * 2.0 - 1.0;
    let el = sign(s) * s * s * (PI * 0.5);
    return vec3<f32>(sin(az) * cos(el), sin(el), -cos(az) * cos(el));
}

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> Out {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.clip = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(uv.x, 1.0 - uv.y);
    return o;
}

@fragment
fn fs_skylut(in: Out) -> @location(0) vec4<f32> {
    // The LUT is stored with v = 0 at the bottom (looking down).
    let dir = lut_dir(vec2<f32>(in.uv.x, 1.0 - in.uv.y));
    let sun = normalize(frame.sun_dir.xyz);
    var d = dir;
    // Below the horizon: the atmosphere just above it, darkening into the ground.
    let below = d.y < 0.0;
    if (below) {
        d = normalize(vec3<f32>(d.x, 0.0005, d.z));
    }
    var col = atmosphere(d, sun);
    // A faint night sky so moonless scenes aren't pitch black.
    col = col + vec3<f32>(0.004, 0.006, 0.012);
    // Overcast: clouds scatter the sky toward a flat grey.
    let cover = frame.sky.x;
    let lum = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));
    col = mix(col, vec3<f32>(lum * 1.05), cover * cover * 0.8);
    if (below) {
        let day = smoothstep(-0.12, 0.18, sun.y);
        let ground = vec3<f32>(0.11, 0.105, 0.095) * (0.15 + 0.85 * day) * max(sun.y, 0.05) * 2.5;
        col = mix(col, ground, smoothstep(0.0, 0.25, -dir.y));
    }
    return vec4<f32>(col, 1.0);
}
