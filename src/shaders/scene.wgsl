// LumenCraft main scene shader: physically based shading, sun + sky lighting,
// soft shadows, procedural clouds, glass, water and landscape materials.

struct Frame {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    light_vp: array<mat4x4<f32>, 4>, // one per shadow cascade
    cascade_far: vec4<f32>,  // view-depth where each cascade ends
    cascade_texel: vec4<f32>,// world size of one shadow texel per cascade
    cam_pos: vec4<f32>,     // xyz, time
    cam_fwd: vec4<f32>,     // xyz, number of lights
    sun_dir: vec4<f32>,     // xyz (towards sun), intensity
    sun_color: vec4<f32>,   // rgb, ambient strength
    sky: vec4<f32>,         // cloudiness, fog density, haze, shadow softness
    post: vec4<f32>,        // exposure, contrast, saturation, vignette
    misc: vec4<f32>,        // pcf radius, wind, reflections, unused
    extra: vec4<f32>,       // ssr strength, unused, unused, unused
};

struct Light {
    pos_range: vec4<f32>,   // xyz position, range (m)
    color: vec4<f32>,       // rgb * intensity, cos(outer cone) (-2 = point light)
    dir: vec4<f32>,         // xyz spot direction, cos(inner cone)
};

struct MaterialU {
    base: vec4<f32>,
    emissive: vec4<f32>,    // rgb * strength, normal-map strength
    params: vec4<f32>,      // metallic, roughness, kind, uv scale
    extra: vec4<f32>,       // variation, has colour map, has normal map, cutout
    uvt: vec4<f32>,         // uv scale.xy, offset.xy from the source file
    extra2: vec4<f32>,      // has roughness map, uv rotation, uv offset
    extra3: vec4<f32>,      // texture influence, reflectivity, anti-tiling, highlight
    extra4: vec4<f32>,      // real-world box mapping
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var shadow_map: texture_depth_2d_array;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(0) @binding(3) var<storage, read> lights: array<Light>;
@group(0) @binding(4) var sky_lut: texture_2d<f32>;
@group(0) @binding(5) var sky_samp: sampler;

// Pass B only (reflections on glossy surfaces, glass and water): the
// already-rendered opaque scene and its depth.
@group(2) @binding(0) var scene_color: texture_2d<f32>;
@group(2) @binding(1) var scene_depth: texture_2d<f32>;
@group(2) @binding(2) var scene_samp: sampler;

@group(1) @binding(0) var<uniform> mat: MaterialU;
@group(1) @binding(1) var base_tex: texture_2d<f32>;
@group(1) @binding(2) var base_sampler: sampler;
@group(1) @binding(3) var normal_tex: texture_2d<f32>;
@group(1) @binding(4) var rough_tex: texture_2d<f32>;

// Tangent frame from screen-space derivatives (no stored tangents needed).
fn cotangent_frame(n: vec3<f32>, p: vec3<f32>, uv: vec2<f32>) -> mat3x3<f32> {
    let dp1 = dpdx(p);
    let dp2 = dpdy(p);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let m = max(dot(t, t), dot(b, b));
    let inv = select(0.0, inverseSqrt(m), m > 1e-20);
    return mat3x3<f32>(t * inv, b * inv, n);
}

const PI: f32 = 3.14159265;

// ---------------------------------------------------------------- noise
fn hash2(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(123.34, 456.21));
    let r = q + dot(q, q + 45.32);
    return fract(r.x * r.y);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2<f32>(1.0, 0.0));
    let c = hash2(i + vec2<f32>(0.0, 1.0));
    let d = hash2(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
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

// ---------------------------------------------------------------- sky
fn day_factor() -> f32 {
    return smoothstep(-0.12, 0.18, frame.sun_dir.y);
}

// Sky radiance from the atmosphere LUT (see sky.wgsl).
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

// Cumulus layer: density from layered noise, lit by marching a few steps
// toward the sun (self-shadowing), with forward-scattering silver linings.
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
    let day = day_factor();
    let sun_c = frame.sun_color.rgb * frame.sun_dir.w * 0.13;
    let amb = sky_base(vec3<f32>(0.0, 1.0, 0.0)) * 0.9 + sky_base(normalize(vec3<f32>(dir.x, 0.05, dir.z))) * 0.5;
    var cloud_col = sun_c * light * (0.7 + 6.0 * hg) * day + amb * (0.55 + 0.45 * (1.0 - d));
    cloud_col = mix(cloud_col, vec3<f32>(dot(cloud_col, vec3<f32>(0.3, 0.5, 0.2))), cover * cover * 0.5);
    let alpha = (1.0 - exp(-d * 4.0)) * smoothstep(0.0, 0.12, dir.y);
    // Distant clouds fade into the horizon haze.
    let far = smoothstep(0.02, 0.25, dir.y);
    return mix(col, mix(col, cloud_col, far * 0.85 + 0.15), alpha);
}

// Cheaper 3-octave noise for surface detail (clouds keep the full 5).
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

fn sky_full(dir: vec3<f32>) -> vec3<f32> {
    return clouds(dir, sky_base(dir));
}

// ---------------------------------------------------------------- brdf
fn d_ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d + 1e-6);
}

fn g_smith(nv: f32, nl: f32, r: f32) -> f32 {
    let k = (r + 1.0) * (r + 1.0) / 8.0;
    return (nv / (nv * (1.0 - k) + k)) * (nl / (nl * (1.0 - k) + k));
}

fn f_schlick(c: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - c, 5.0);
}

fn f_schlick_rough(c: f32, f0: vec3<f32>, r: f32) -> vec3<f32> {
    return f0 + (max(vec3<f32>(1.0 - r), f0) - f0) * pow(1.0 - c, 5.0);
}

fn sample_cascade(c: u32, wp: vec3<f32>, n: vec3<f32>) -> f32 {
    let texel = frame.cascade_texel[c];
    let lp = frame.light_vp[c] * vec4<f32>(wp + n * texel * 1.5, 1.0);
    let ndc = lp.xyz / lp.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, -ndc.y * 0.5 + 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z > 1.0) {
        return 1.0;
    }
    let soft = frame.sky.w;
    let step = (1.0 + soft * 3.0) / f32(textureDimensions(shadow_map).x);
    let k = i32(frame.misc.x);
    let zref = ndc.z - 0.0004;
    if (k <= 1) {
        // 3x3 grid (each tap is a bilinear 2x2 compare in hardware)
        var s = 0.0;
        for (var x = -1; x <= 1; x = x + 1) {
            for (var y = -1; y <= 1; y = y + 1) {
                s = s + textureSampleCompareLevel(shadow_map, shadow_sampler, uv + vec2<f32>(f32(x), f32(y)) * step, i32(c), zref);
            }
        }
        return s / 9.0;
    }
    // 12-tap Poisson disk covering the same area as a 5x5 grid.
    var taps = array<vec2<f32>, 12>(
        vec2<f32>(-0.326, -0.406), vec2<f32>(-0.840, -0.074), vec2<f32>(-0.696, 0.457),
        vec2<f32>(-0.203, 0.621), vec2<f32>(0.962, -0.195), vec2<f32>(0.473, -0.480),
        vec2<f32>(0.519, 0.767), vec2<f32>(0.185, -0.893), vec2<f32>(0.507, 0.064),
        vec2<f32>(0.896, 0.412), vec2<f32>(-0.322, -0.933), vec2<f32>(-0.792, -0.598));
    var s = 0.0;
    for (var i = 0; i < 12; i = i + 1) {
        s = s + textureSampleCompareLevel(shadow_map, shadow_sampler, uv + taps[i] * step * 2.2, i32(c), zref);
    }
    return s / 12.0;
}

// Cascaded shadow maps: sharp near the camera, still present far away.
fn shadow_factor(wp: vec3<f32>, n: vec3<f32>) -> f32 {
    let depth = dot(wp - frame.cam_pos.xyz, frame.cam_fwd.xyz);
    var c = 0u;
    loop {
        if (c >= 3u || depth < frame.cascade_far[c]) { break; }
        c = c + 1u;
    }
    let last = frame.cascade_far[3];
    if (depth > last) {
        return 1.0;
    }
    var s = sample_cascade(c, wp, n);
    // Blend into the next cascade near the boundary to hide seams.
    let start = select(0.0, frame.cascade_far[max(c, 1u) - 1u], c > 0u);
    let t = (depth - start) / max(frame.cascade_far[c] - start, 1e-3);
    if (t > 0.85) {
        let next = select(1.0, sample_cascade(min(c + 1u, 3u), wp, n), c < 3u);
        s = mix(s, next, (t - 0.85) / 0.15);
    }
    // Fade out at the very end of the shadow distance.
    return mix(s, 1.0, smoothstep(last * 0.9, last, depth));
}

// Point and spot lights (inverse-square falloff with a smooth range window).
fn local_lights(wp: vec3<f32>, n: vec3<f32>, v: vec3<f32>, albedo: vec3<f32>, f0: vec3<f32>, rough: f32, metallic: f32) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    let count = u32(frame.cam_fwd.w);
    let a = rough * rough;
    for (var i = 0u; i < count; i = i + 1u) {
        let li = lights[i];
        let lv = li.pos_range.xyz - wp;
        let d2 = max(dot(lv, lv), 1e-4);
        let range = li.pos_range.w;
        if (d2 > range * range) { continue; }
        let d = sqrt(d2);
        let l = lv / d;
        let nl = dot(n, l);
        if (nl <= 0.0) { continue; }
        let x = d / range;
        let win = clamp(1.0 - x * x * x * x, 0.0, 1.0);
        var atten = win * win / (d2 + 0.25);
        if (li.color.w > -1.5) {
            atten = atten * smoothstep(li.color.w, li.dir.w, dot(-l, li.dir.xyz));
        }
        let h = normalize(l + v);
        let F = f_schlick(max(dot(v, h), 0.0), f0);
        let spec = d_ggx(max(dot(n, h), 0.0), a) * g_smith(max(dot(n, v), 1e-3), nl, rough) * F / (4.0 * max(dot(n, v), 1e-3) * nl + 1e-4);
        let kd = (vec3<f32>(1.0) - F) * (1.0 - metallic);
        sum = sum + (kd * albedo / PI + spec) * li.color.rgb * atten * nl;
    }
    return sum;
}

// ---------------------------------------------------------------- vertex
struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) nrm: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VsOut {
    // invariant: the depth pre-pass and the shading pass must produce the
    // exact same depth for the Equal depth test.
    @builtin(position) @invariant clip: vec4<f32>,
    @location(0) wpos: vec3<f32>,
    @location(1) nrm: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    var o: VsOut;
    var p = v.pos;
    // Foliage sways gently in the wind.
    if (u32(mat.params.z) == 4u) {
        let t = frame.cam_pos.w * (0.6 + frame.misc.y);
        let w = sin(t * 1.7 + p.x * 0.7 + p.z * 0.5) * 0.04 * max(p.y - 0.5, 0.0) * 0.15;
        p = p + vec3<f32>(w, 0.0, w * 0.6);
    }
    o.wpos = p;
    o.nrm = v.nrm;
    o.uv = v.uv;
    o.clip = frame.view_proj * vec4<f32>(p, 1.0);
    return o;
}

// ---------------------------------------------------------------- fragment
fn apply_fog(c: vec3<f32>, wp: vec3<f32>, a: f32) -> vec3<f32> {
    let d = distance(wp, frame.cam_pos.xyz);
    // Height fog: dense near the ground, thin when looking down from above.
    let avg_h = max((wp.y + frame.cam_pos.y) * 0.5, 0.0);
    let fh = 1.0 - exp(-min(d, 1200.0) * frame.sky.y * 0.01 * exp(-avg_h / 120.0));
    // Aerial perspective: far terrain and mountains fade into the sky.
    let fa = 1.0 - exp(-d * (0.00002 + frame.sky.z * 0.00007));
    let f = 1.0 - (1.0 - fh) * (1.0 - fa);
    let v = normalize(wp - frame.cam_pos.xyz);
    let fog_col = sky_base(normalize(vec3<f32>(v.x, max(v.y, 0.02), v.z)));
    return mix(c, fog_col * a, clamp(f, 0.0, 1.0));
}

fn water_normal(n: vec3<f32>, wp: vec3<f32>, t: f32) -> vec3<f32> {
    // Animated wave normals.
    let p = wp.xz;
    let dx = cos(p.x * 1.3 + t * 1.1) * 0.06 + cos(p.x * 3.7 + p.y * 1.9 + t * 2.3) * 0.03 + (vnoise(p * 4.0 + t * 0.6) - 0.5) * 0.08;
    let dz = cos(p.y * 1.7 + t * 0.9) * 0.06 + cos(p.y * 4.1 - p.x * 1.3 + t * 2.7) * 0.03 + (vnoise(p * 4.0 - t * 0.5 + 7.0) - 0.5) * 0.08;
    return normalize(n + vec3<f32>(dx, 0.0, dz));
}

// Box (tri-planar, dominant axis) projection: u runs left-to-right as you
// face the surface, v runs up on walls.
fn box_uv(p: vec3<f32>, n: vec3<f32>) -> vec2<f32> {
    let a = abs(n);
    if (a.y >= a.x && a.y >= a.z) {
        return vec2<f32>(p.x, -p.z * sign(n.y + 1e-6));
    } else if (a.x >= a.z) {
        return vec2<f32>(-p.z * sign(n.x), -p.y);
    }
    return vec2<f32>(p.x * sign(n.z), -p.y);
}

struct Shaded {
    color: vec3<f32>,
    alpha: f32,
    coverage: f32, // texture alpha, for cut-outs
};

fn shade(in: VsOut, ssr: vec4<f32>) -> Shaded {
    let kind = u32(mat.params.z);
    let v = normalize(frame.cam_pos.xyz - in.wpos);
    // Orient the shading normal with the true geometric normal, so models with
    // inconsistent triangle winding (common in Revit exports) shade correctly.
    var geo = normalize(cross(dpdx(in.wpos), dpdy(in.wpos)));
    if (dot(geo, v) < 0.0) {
        geo = -geo;
    }
    var n = normalize(in.nrm);
    if (dot(n, geo) < 0.0) {
        n = -n;
    }
    let t = frame.cam_pos.w;

    var uv_in = in.uv * mat.uvt.xy + mat.uvt.zw;
    if (mat.extra4.x > 0.5) {
        // Real-world box mapping from the face direction (metres).
        // Face direction on the side the model's normal points to (the
        // outside), so both sides of a wall and the path tracer agree.
        uv_in = box_uv(in.wpos, geo * select(-1.0, 1.0, dot(geo, in.nrm) >= 0.0));
    }
    let uv0 = uv_in * mat.params.w;
    let rc = cos(mat.extra2.y);
    let rs = sin(mat.extra2.y);
    let uv = vec2<f32>(uv0.x * rc - uv0.y * rs, uv0.x * rs + uv0.y * rc) + mat.extra2.zw;
    // Derivative-based work must happen before any non-uniform branching.
    let tbn = cotangent_frame(n, in.wpos, uv);
    var tex = textureSample(base_tex, base_sampler, uv);
    // Anti-tiling: blend in a rotated, larger-scale copy with a world-space mask.
    let uv_b = vec2<f32>(uv.x * 0.6 - uv.y * 0.8, uv.x * 0.8 + uv.y * 0.6) * 0.43 + vec2<f32>(0.37, 0.71);
    let tex_b = textureSample(base_tex, base_sampler, uv_b);
    if (mat.extra3.z > 0.5) {
        let m = smoothstep(0.35, 0.65, fbm3(in.wpos.xz * 0.11 + in.wpos.y * 0.07));
        tex = mix(tex, tex_b, m);
    }
    let nmap = textureSample(normal_tex, base_sampler, uv).xyz;
    let rmap = textureSample(rough_tex, base_sampler, uv).r;
    var albedo = mat.base.rgb;
    var opacity = mat.base.a;
    var coverage = 1.0;
    if (mat.extra.y > 0.5) {
        albedo = albedo * mix(vec3<f32>(1.0), tex.rgb, clamp(mat.extra3.x, 0.0, 1.0));
        // Keep alpha-tested foliage full at a distance: mip-mapping averages
        // the alpha down, so boost it with the mip level.
        let tdim = vec2<f32>(textureDimensions(base_tex));
        let lod = 0.5 * log2(max(max(dot(dpdx(uv * tdim), dpdx(uv * tdim)), dot(dpdy(uv * tdim), dpdy(uv * tdim))), 1.0));
        coverage = tex.a * (1.0 + lod * 0.3);
        if (mat.extra.w < 0.5) {
            opacity = opacity * tex.a;
        }
    }
    if (mat.extra.z > 0.5) {
        // OpenGL-style normal maps with v pointing down the image: flip green.
        var tn = nmap * 2.0 - 1.0;
        tn = vec3<f32>(tn.x * mat.emissive.w, -tn.y * mat.emissive.w, tn.z);
        let pn = normalize(tbn * tn);
        if (dot(pn, pn) > 0.5) {
            n = pn;
        }
    }
    var metallic = mat.params.x;
    var rough = clamp(mat.params.y, 0.03, 1.0);
    if (mat.extra2.x > 0.5) {
        // Map gives the roughness; the slider (0.5 = neutral) shifts it.
        rough = clamp(rmap + (mat.params.y - 0.5), 0.03, 1.0);
    }

    // Procedural variation / weathering so flat Revit colours don't look CG.
    // Fade procedural detail out with distance (it's invisible far away anyway).
    let cam_dist = distance(in.wpos, frame.cam_pos.xyz);
    let var_amt = mat.extra.x * (1.0 - smoothstep(120.0, 250.0, cam_dist));
    if (var_amt > 0.0) {
        let wp2 = in.wpos.xz + in.wpos.y * 0.37;
        let nz = fbm3(wp2 * 1.3) * 0.7 + vnoise(wp2 * 9.0) * 0.3;
        albedo = albedo * (1.0 + (nz - 0.5) * var_amt * 0.9);
        rough = clamp(rough + (nz - 0.5) * var_amt * 0.2, 0.03, 1.0);
    }

    if (kind == 5u) {
        // Landscape: multi-scale colour breakup + dry patches.
        let p = in.wpos.xz;
        let big = fbm3(p * 0.05);
        let mid = select(0.5, fbm3(p * 0.6), cam_dist < 400.0);
        let fine = select(0.5, vnoise(p * 14.0), cam_dist < 60.0);
        let dry = albedo * vec3<f32>(1.6, 1.35, 0.8);
        albedo = mix(albedo, dry, smoothstep(0.45, 0.8, big) * mat.extra.x);
        albedo = albedo * (0.7 + 0.45 * mid + 0.25 * (fine - 0.5) * var_amt);
    }

    if (kind == 6u) {
        // Distant mountains: forest low down, rock on steep or high ground,
        // snow on the peaks; noise-based bumps for a rugged silhouette.
        let p = in.wpos;
        let nz = fbm3(p.xz * 0.0035);
        let e = 6.0;
        let g0 = fbm3(p.xz * 0.02);
        let gx = fbm3((p.xz + vec2<f32>(e, 0.0)) * 0.02) - g0;
        let gz = fbm3((p.xz + vec2<f32>(0.0, e)) * 0.02) - g0;
        n = normalize(n + vec3<f32>(-gx, 0.0, -gz) * 9.0);
        let slope = 1.0 - clamp(n.y, 0.0, 1.0);
        let fine = vnoise(p.xz * 0.05);
        let rock = vec3<f32>(0.30, 0.29, 0.27) * (0.65 + 0.7 * g0);
        let forest = vec3<f32>(0.028, 0.05, 0.025) * (0.55 + 0.9 * fine);
        let meadow = vec3<f32>(0.11, 0.13, 0.05) * (0.7 + 0.6 * fine);
        var c = mix(forest, meadow, smoothstep(0.55, 0.85, nz) * 0.6);
        let tree_line = 300.0 + nz * 260.0;
        let rocky = max(smoothstep(tree_line - 60.0, tree_line + 80.0, p.y), smoothstep(0.42, 0.62, slope));
        c = mix(c, rock, rocky);
        let snow_line = 700.0 + nz * 220.0;
        let snow = smoothstep(snow_line, snow_line + 120.0, p.y) * (1.0 - smoothstep(0.5, 0.72, slope));
        c = mix(c, vec3<f32>(0.82, 0.84, 0.88), snow);
        albedo = c;
        rough = 0.9;
    }
    if (kind == 2u) {
        n = water_normal(n, in.wpos, t);
    }

    if (kind == 4u && mat.extra.y < 0.5) {
        // Grass blades: darker at the root, lighter and yellower at the tip.
        albedo = albedo * mix(vec3<f32>(0.35, 0.4, 0.35), vec3<f32>(1.25, 1.2, 0.95), in.uv.y);
    }
    let l = frame.sun_dir.xyz;
    let h = normalize(l + v);
    let nl_raw = dot(n, l);
    let nl = max(nl_raw, 0.0);
    let nv = max(dot(n, v), 1e-3);
    let nh = max(dot(n, h), 0.0);
    let vh = max(dot(v, h), 0.0);

    let f0 = mix(vec3<f32>(0.08 * mat.extra3.y), albedo, metallic);
    let sun_vis = smoothstep(-0.03, 0.04, frame.sun_dir.y);
    var shadow = 1.0;
    if (sun_vis > 0.0) {
        shadow = shadow_factor(in.wpos, n);
    }
    let sun = frame.sun_color.rgb * frame.sun_dir.w * sun_vis * shadow;

    let a = rough * rough;
    let F = f_schlick(vh, f0);
    let spec = d_ggx(nh, a) * g_smith(nv, nl, rough) * F / (4.0 * nv * max(nl, 1e-3) + 1e-4);
    let kd = (vec3<f32>(1.0) - F) * (1.0 - metallic);
    var direct = (kd * albedo / PI + spec) * sun * nl + local_lights(in.wpos, n, v, albedo, f0, rough, metallic);

    if (kind == 4u && nl_raw < 0.0) {
        // Leaves transmit light.
        direct = direct + albedo * vec3<f32>(1.0, 1.1, 0.6) * sun * (-nl_raw) * 0.25;
    }

    // Image based lighting from the analytic sky.
    let amb = frame.sun_color.a;
    let up = n.y * 0.5 + 0.5;
    // Light bounced off the sunlit ground fills in downward-facing surfaces.
    let ground_bounce = vec3<f32>(0.20, 0.18, 0.15) * day_factor()
        + vec3<f32>(0.16, 0.14, 0.11) * frame.sun_color.rgb * frame.sun_dir.w * max(frame.sun_dir.y, 0.0) * 0.18;
    let sky_irr = sky_base(normalize(n + vec3<f32>(0.0, 0.6, 0.0)));
    let sky_l = dot(sky_irr, vec3<f32>(0.2126, 0.7152, 0.0722));
    let irr = mix(ground_bounce, mix(vec3<f32>(sky_l), sky_irr, 0.65), up) * amb * 0.6;
    let r = reflect(-v, n);
    let fr = f_schlick_rough(nv, f0, rough);
    let refl_strength = frame.misc.z;
    var env = sky_base(normalize(vec3<f32>(r.x, max(r.y, -0.2), r.z)));
    if (rough < 0.3) {
        env = clouds(r, env);
    }
    // Screen-space reflection of the scene where the ray found something.
    env = mix(env, ssr.rgb, ssr.a);
    let horizon_occ = clamp(1.0 + dot(r, n), 0.0, 1.0);
    let spec_occ = mix(0.35, 1.0, shadow) * (1.0 - rough * 0.7) * mix(0.55, 1.0, metallic);
    let ambient = kd * albedo * irr + fr * env * refl_strength * spec_occ * horizon_occ * amb;

    var color = direct + ambient;
    color = color + mat.emissive.rgb;

    var out: Shaded;
    out.alpha = 1.0;
    out.coverage = coverage;

    if (kind == 1u) {
        // Glass: premultiplied — reflections stay fully visible.
        let fres = f_schlick(nv, vec3<f32>(0.04)).x;
        let refl = env * refl_strength * fres * amb + spec * sun * nl * 4.0;
        let body = albedo * irr * 0.4 * opacity;
        out.color = refl + body;
        out.alpha = clamp(opacity * (1.0 - fres) + fres, 0.0, 1.0);
        return out;
    }
    if (kind == 2u) {
        let fres = f_schlick(nv, vec3<f32>(0.02)).x;
        let refl = mix(clouds(r, sky_base(normalize(vec3<f32>(r.x, abs(r.y), r.z)))), ssr.rgb, ssr.a) * amb * refl_strength;
        let body = albedo * (irr * 1.5 + sun * 0.35);
        let sun_spec = spec * sun * nl * 3.0;
        out.alpha = clamp(mix(opacity, 1.0, fres * 0.7), 0.0, 1.0);
        out.color = body * opacity * (1.0 - fres) + refl * fres * 0.6 + sun_spec;
        return out;
    }
    out.color = color;
    out.alpha = 1.0;
    return out;
}

// Material picker highlight: yellow for the hovered surface, blue for the one
// being edited.
fn highlight(c: vec3<f32>) -> vec3<f32> {
    let h = mat.extra3.w;
    if (h < 0.5) {
        return c;
    }
    let tint = select(vec3<f32>(1.0, 0.82, 0.08), vec3<f32>(0.25, 0.6, 1.0), h > 1.5);
    let l = max(dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.18);
    return mix(c, tint * l * 1.6, select(0.42, 0.25, h > 1.5));
}

@fragment
fn fs_opaque(in: VsOut) -> @location(0) vec4<f32> {
    let s = shade(in, vec4<f32>(0.0));
    if (mat.extra.w > 0.5 && s.coverage < 0.5) {
        discard;
    }
    return vec4<f32>(highlight(apply_fog(s.color, in.wpos, 1.0)), 1.0);
}

// ---------------------------------------------------------------- reflections
// Linear view distance from a reverse-Z (infinite far) depth value.
fn lin_depth(d: f32) -> f32 {
    return 0.05 / max(d, 1e-7);
}

// March the reflected ray through the depth buffer of the opaque scene.
// Returns the reflected colour and a confidence (0 = fall back to the sky).
fn trace_ssr(wp: vec3<f32>, n: vec3<f32>, rough: f32) -> vec4<f32> {
    let strength = frame.extra.x;
    if (strength <= 0.0 || rough > 0.6) {
        return vec4<f32>(0.0);
    }
    let dims = vec2<f32>(textureDimensions(scene_depth));
    let v = normalize(wp - frame.cam_pos.xyz);
    let r = reflect(v, n);
    let dist = distance(wp, frame.cam_pos.xyz);
    let start = wp + n * (0.02 + dist * 0.002);
    let jitter = fract(sin(dot(wp.xz, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    var step = 0.08 + dist * 0.015;
    var t = step * (0.5 + jitter);
    var prev_t = 0.0;
    let steps = 40;
    for (var i = 0; i < steps; i = i + 1) {
        let p = start + r * t;
        let c = frame.view_proj * vec4<f32>(p, 1.0);
        if (c.w <= 0.0) {
            break;
        }
        let ndc = c.xyz / c.w;
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
            break;
        }
        let sd = textureLoad(scene_depth, vec2<i32>(uv * dims), 0).r;
        if (sd > 0.0) {
            let scene_z = lin_depth(sd);
            let ray_z = c.w;
            let thick = 0.25 + step * 1.5;
            if (ray_z > scene_z && ray_z - scene_z < thick) {
                // Refine between the last two steps.
                var lo = prev_t;
                var hi = t;
                var huv = uv;
                for (var k = 0; k < 5; k = k + 1) {
                    let mid = (lo + hi) * 0.5;
                    let pm = start + r * mid;
                    let cm = frame.view_proj * vec4<f32>(pm, 1.0);
                    let um = vec2<f32>(cm.x / cm.w * 0.5 + 0.5, 0.5 - cm.y / cm.w * 0.5);
                    let dm = lin_depth(textureLoad(scene_depth, vec2<i32>(clamp(um, vec2<f32>(0.0), vec2<f32>(0.9999)) * dims), 0).r);
                    if (cm.w > dm) {
                        hi = mid;
                        huv = um;
                    } else {
                        lo = mid;
                    }
                }
                let col = textureSampleLevel(scene_color, scene_samp, huv, 0.0).rgb;
                let edge = smoothstep(0.0, 0.08, min(min(huv.x, 1.0 - huv.x), min(huv.y, 1.0 - huv.y)));
                let fade = edge * (1.0 - f32(i) / f32(steps)) * (1.0 - smoothstep(0.25, 0.6, rough));
                return vec4<f32>(col, clamp(fade * strength, 0.0, 1.0));
            }
        }
        prev_t = t;
        step = step * 1.12;
        t = t + step;
    }
    return vec4<f32>(0.0);
}

fn oriented_normal(in: VsOut) -> vec3<f32> {
    let v = normalize(frame.cam_pos.xyz - in.wpos);
    var geo = normalize(cross(dpdx(in.wpos), dpdy(in.wpos)));
    if (dot(geo, v) < 0.0) {
        geo = -geo;
    }
    var n = normalize(in.nrm);
    if (dot(n, geo) < 0.0) {
        n = -n;
    }
    return n;
}

@fragment
fn fs_transparent(in: VsOut) -> @location(0) vec4<f32> {
    var n = oriented_normal(in);
    if (u32(mat.params.z) == 2u) {
        n = water_normal(n, in.wpos, frame.cam_pos.w);
    }
    let ssr = trace_ssr(in.wpos, n, mat.params.y);
    let s = shade(in, ssr);
    return vec4<f32>(highlight(apply_fog(s.color, in.wpos, s.alpha)), s.alpha);
}

// Glossy opaque surfaces (floors, polished stone, metal, car paint): blend
// the screen-space reflection over what the main pass already drew.
@fragment
fn fs_ssr_overlay(in: VsOut) -> @location(0) vec4<f32> {
    let n = oriented_normal(in);
    let rough = clamp(mat.params.y, 0.03, 1.0);
    let ssr = trace_ssr(in.wpos, n, rough);
    if (ssr.a <= 0.001) {
        discard;
    }
    let v = normalize(frame.cam_pos.xyz - in.wpos);
    let nv = max(dot(n, v), 1e-3);
    let f0 = mix(vec3<f32>(0.08 * mat.extra3.y), mat.base.rgb, mat.params.x);
    let fr = f_schlick_rough(nv, f0, rough);
    let w = clamp(dot(fr, vec3<f32>(0.333)) * frame.misc.z, 0.0, 1.0) * ssr.a * (1.0 - rough);
    let col = apply_fog(ssr.rgb * fr * frame.sun_color.a, in.wpos, 1.0);
    return vec4<f32>(col * ssr.a * (1.0 - rough), w);
}

// ---------------------------------------------------------------- sky pass
struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: SkyOut;
    o.ndc = uv * 2.0 - 1.0;
    o.clip = vec4<f32>(o.ndc, 0.0, 1.0); // reverse-Z: 0 = infinitely far
    return o;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    // Reverse-Z with an infinite far plane: z=1 is the near plane, z=0.5 is 2x near.
    let far = frame.inv_view_proj * vec4<f32>(in.ndc, 0.5, 1.0);
    let near = frame.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(far.xyz / far.w - near.xyz / near.w);
    var col = sky_full(dir);
    // Sun disk
    let mu = dot(dir, frame.sun_dir.xyz);
    let disk = smoothstep(0.9995, 0.99975, mu) * smoothstep(-0.02, 0.01, frame.sun_dir.y);
    col = col + frame.sun_color.rgb * disk * 40.0;
    // Stars at night
    let night = 1.0 - day_factor();
    if (night > 0.0 && dir.y > 0.0) {
        let sp = dir.xz / (dir.y + 0.3) * 300.0;
        let st = step(0.997, hash2(floor(sp)));
        col = col + vec3<f32>(st) * night * 0.6 * (1.0 - frame.sky.x);
    }
    return vec4<f32>(col * frame.sun_color.a, 1.0);
}

// Depth pre-pass: colour writes are masked off; only depth matters.
@fragment
fn fs_depth(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}
