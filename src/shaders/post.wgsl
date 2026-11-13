// Tone mapping + colour grading (Lumion "effects" stack, simplified).
struct Post {
    params: vec4<f32>,   // exposure, contrast, saturation, vignette
    extra: vec4<f32>,    // bloom strength, sharpen, ao strength, ao radius (m)
    quality: vec4<f32>,  // ssao sample count, fxaa, time, auto exposure on
    lens: vec4<f32>,     // dof amount, focus distance (0 = auto), chromatic aberration, film grain
    proj: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    tone: vec4<f32>,     // filmic (AgX) on
};
@group(0) @binding(0) var hdr: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> post: Post;
@group(0) @binding(3) var bloom_tex: texture_2d<f32>;
@group(0) @binding(4) var depth_tex: texture_2d<f32>;
@group(0) @binding(5) var exposure_tex: texture_2d<f32>;

fn view_pos(coord: vec2<i32>, dims: vec2<f32>) -> vec3<f32> {
    let c = clamp(coord, vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
    let d = textureLoad(depth_tex, c, 0).r;
    let uv = (vec2<f32>(c) + 0.5) / dims;
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let v = post.inv_proj * ndc;
    return v.xyz / v.w;
}

// Screen-space ambient occlusion (normal-oriented, distance-weighted).
fn ssao(coord: vec2<i32>, dims: vec2<f32>) -> f32 {
    let strength = post.extra.z;
    let d0 = textureLoad(depth_tex, coord, 0).r;
    let N = i32(post.quality.x);
    if (strength <= 0.0 || d0 <= 0.0 || N <= 0) { // reverse-Z: 0 = sky
        return 1.0;
    }
    let p = view_pos(coord, dims);
    let px = view_pos(coord + vec2<i32>(1, 0), dims);
    let nx = view_pos(coord - vec2<i32>(1, 0), dims);
    let py = view_pos(coord + vec2<i32>(0, 1), dims);
    let ny = view_pos(coord - vec2<i32>(0, 1), dims);
    // Pick the smaller difference on each axis to avoid edge artifacts.
    var dx = px - p;
    if (abs(nx.z - p.z) < abs(dx.z)) { dx = p - nx; }
    var dy = py - p;
    if (abs(ny.z - p.z) < abs(dy.z)) { dy = p - ny; }
    let n = normalize(cross(dy, dx));
    let radius = post.extra.w;
    let r_px = clamp(radius * post.proj[1][1] * 0.5 * dims.y / max(-p.z, 0.05), 3.0, 120.0);
    let noise = fract(sin(dot(vec2<f32>(coord), vec2<f32>(12.9898, 78.233))) * 43758.5453) * 6.2831;
    var occ = 0.0;
    for (var i = 0; i < N; i = i + 1) {
        let fi = f32(i);
        let ang = fi * 2.39996 + noise;
        let rr = sqrt((fi + 0.5) / f32(N)) * r_px;
        let off = vec2<i32>(vec2<f32>(cos(ang), sin(ang)) * rr);
        let q = view_pos(coord + off, dims);
        let v = q - p;
        let dist = length(v);
        let fall = 1.0 - smoothstep(radius * 0.5, radius * 2.0, dist);
        occ = occ + max(dot(n, v / max(dist, 1e-4)) - 0.15, 0.0) * fall;
    }
    return clamp(1.0 - strength * occ / f32(N) * 1.6, 0.0, 1.0);
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

fn aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51; let b = 0.03; let c = 2.43; let d = 0.59; let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

// AgX (Troy Sobotka's filmic view transform; fitted polynomial by
// Benjamin Wrensch). Desaturates highlights toward white like film instead
// of skewing hues, so skies and lit walls look photographic.
fn agx_contrast(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

fn agx(c_in: vec3<f32>) -> vec3<f32> {
    let inset = mat3x3<f32>(
        vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336),
        vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104));
    let outset = mat3x3<f32>(
        vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116));
    let min_ev = -12.47393;
    let max_ev = 4.026069;
    var c = inset * max(c_in, vec3<f32>(1e-10));
    c = clamp((log2(c) - min_ev) / (max_ev - min_ev), vec3<f32>(0.0), vec3<f32>(1.0));
    c = agx_contrast(c);
    // "Punchy" look (as in Blender): more contrast and saturation.
    c = pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.35));
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = luma + 1.4 * (c - luma);
    c = outset * c;
    // AgX output is display-encoded (~gamma 2.2); return linear for the grading below.
    return pow(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
}

fn lin_depth(d: f32) -> f32 {
    return select(1e5, 0.05 / max(d, 1e-7), d > 0.0);
}

// Lens: chromatic aberration splits colour channels toward the edges.
fn sample_lens(uv: vec2<f32>) -> vec3<f32> {
    let ca = post.lens.z;
    if (ca <= 0.0) {
        return textureSampleLevel(hdr, samp, uv, 0.0).rgb;
    }
    let d = (uv - 0.5) * ca * 0.012;
    let r = textureSampleLevel(hdr, samp, uv + d, 0.0).r;
    let g = textureSampleLevel(hdr, samp, uv, 0.0).g;
    let b = textureSampleLevel(hdr, samp, uv - d, 0.0).b;
    return vec3<f32>(r, g, b);
}

// Depth of field: golden-angle gather, each tap weighted by its own blur
// size so sharp foreground edges don't bleed.
fn coc_px(dist: f32, focus: f32, dims: vec2<f32>) -> f32 {
    let amount = post.lens.x;
    return clamp(abs(1.0 / focus - 1.0 / dist) * focus * amount * dims.y * 0.02, 0.0, dims.y * 0.02 * (amount + 0.5));
}

fn depth_of_field(uv: vec2<f32>, base: vec3<f32>, dims: vec2<f32>) -> vec3<f32> {
    if (post.lens.x <= 0.0) {
        return base;
    }
    var focus = post.lens.y;
    if (focus <= 0.0) {
        // Auto focus on whatever is in the middle of the frame.
        // Nearest surface in a small window around the centre of the frame.
        focus = 1e5;
        for (var j = -2; j <= 2; j = j + 1) {
            for (var i = -2; i <= 2; i = i + 1) {
                let q = vec2<i32>(dims * (vec2<f32>(0.5) + vec2<f32>(f32(i), f32(j)) * 0.015));
                focus = min(focus, lin_depth(textureLoad(depth_tex, q, 0).r));
            }
        }
    }
    focus = max(focus, 0.3);
    let d0 = lin_depth(textureLoad(depth_tex, vec2<i32>(uv * dims), 0).r);
    let c0 = coc_px(d0, focus, dims);
    if (c0 < 0.6) {
        return base;
    }
    var sum = base;
    var wsum = 1.0;
    let n = 24;
    for (var i = 0; i < n; i = i + 1) {
        let fi = f32(i) + 0.5;
        let r = sqrt(fi / f32(n)) * c0;
        let a = fi * 2.39996;
        let o = vec2<f32>(cos(a), sin(a)) * r / dims;
        let su = uv + o;
        let ds = lin_depth(textureLoad(depth_tex, vec2<i32>(clamp(su, vec2<f32>(0.0), vec2<f32>(0.9999)) * dims), 0).r);
        let cs = coc_px(ds, focus, dims);
        // Background taps only count if they're blurred at least this much.
        let w = select(clamp(cs / max(r, 1e-3), 0.0, 1.0), 1.0, ds <= d0);
        sum = sum + sample_lens(su) * w;
        wsum = wsum + w;
    }
    return sum / wsum;
}

@fragment
fn fs_post(in: Out) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(hdr));
    let px = 1.0 / dims;
    var c = sample_lens(in.uv);
    if (post.quality.y > 0.5) {
        // FXAA-style edge smoothing when MSAA is off (lower quality levels).
        let lw = vec3<f32>(0.299, 0.587, 0.114);
        let cn = textureSampleLevel(hdr, samp, in.uv + vec2<f32>(0.0, -px.y), 0.0).rgb;
        let cs = textureSampleLevel(hdr, samp, in.uv + vec2<f32>(0.0, px.y), 0.0).rgb;
        let ce = textureSampleLevel(hdr, samp, in.uv + vec2<f32>(px.x, 0.0), 0.0).rgb;
        let cw = textureSampleLevel(hdr, samp, in.uv + vec2<f32>(-px.x, 0.0), 0.0).rgb;
        let lc = dot(c / (1.0 + c), lw);
        let ln = dot(cn / (1.0 + cn), lw);
        let ls = dot(cs / (1.0 + cs), lw);
        let le = dot(ce / (1.0 + ce), lw);
        let lwv = dot(cw / (1.0 + cw), lw);
        let lmax = max(lc, max(max(ln, ls), max(le, lwv)));
        let lmin = min(lc, min(min(ln, ls), min(le, lwv)));
        let range = lmax - lmin;
        if (range > max(0.04, lmax * 0.12)) {
            // Blur along the edge: offset perpendicular to the luminance gradient.
            let grad = vec2<f32>((le - lwv), (ls - ln));
            let dir = normalize(vec2<f32>(-grad.y, grad.x) + vec2<f32>(1e-5, 0.0)) * px;
            let a = textureSampleLevel(hdr, samp, in.uv + dir * 0.5, 0.0).rgb;
            let b = textureSampleLevel(hdr, samp, in.uv - dir * 0.5, 0.0).rgb;
            let blend = clamp(range * 2.0, 0.0, 0.75);
            c = mix(c, (a + b + cn + cs + ce + cw) / 6.0 * 0.5 + (a + b) * 0.25, blend);
        }
    }
    let ao = ssao(vec2<i32>(in.clip.xy), dims);
    c = c * ao;
    c = depth_of_field(in.uv, c, dims);

    // Bloom (blurred bright pass computed at quarter resolution).
    c = c + textureSampleLevel(bloom_tex, samp, in.uv, 0.0).rgb * post.extra.x * 0.35;

    // Sharpen
    let sh = post.extra.y;
    if (sh > 0.0) {
        let nb = textureSampleLevel(hdr, samp, in.uv + vec2<f32>(0.0, px.y), 0.0).rgb
              + textureSampleLevel(hdr, samp, in.uv - vec2<f32>(0.0, px.y), 0.0).rgb
              + textureSampleLevel(hdr, samp, in.uv + vec2<f32>(px.x, 0.0), 0.0).rgb
              + textureSampleLevel(hdr, samp, in.uv - vec2<f32>(px.x, 0.0), 0.0).rgb;
        c = max(c + (c * 4.0 - nb) * sh * 0.25, vec3<f32>(0.0));
    }

    var exposure = post.params.x;
    if (post.quality.w > 0.5) {
        // Auto exposure: scale so the average scene brightness lands on mid grey.
        let avg = textureLoad(exposure_tex, vec2<i32>(0, 0), 0).r;
        exposure = exposure * clamp(0.32 / max(avg, 1e-4), 0.35, 4.0);
    }
    if (post.tone.x > 0.5) {
        // Scaled so mid grey lands where the ACES curve puts it.
        c = agx(c * exposure * 2.2);
    } else {
        c = aces(c * exposure);
    }
    // Contrast around mid grey (in perceptual-ish space)
    let g = pow(c, vec3<f32>(1.0 / 2.2));
    var g2 = (g - 0.5) * post.params.y + 0.5;
    let luma = dot(g2, vec3<f32>(0.2126, 0.7152, 0.0722));
    g2 = mix(vec3<f32>(luma), g2, post.params.z);
    // Vignette
    let d = distance(in.uv, vec2<f32>(0.5));
    g2 = g2 * (1.0 - post.params.w * smoothstep(0.3, 0.85, d));
    // Dither to avoid banding in skies
    let dn = fract(sin(dot(in.clip.xy, vec2<f32>(12.9898, 78.233))) * 43758.5453) - 0.5;
    g2 = g2 + dn / 255.0;
    // Film grain (stronger in the mid-tones, like real film).
    let grain = post.lens.w;
    if (grain > 0.0) {
        let gn = fract(sin(dot(in.clip.xy + vec2<f32>(post.quality.z * 37.0, post.quality.z * 11.0), vec2<f32>(12.9898, 78.233))) * 43758.5453) - 0.5;
        let lum = dot(g2, vec3<f32>(0.2126, 0.7152, 0.0722));
        g2 = g2 + gn * grain * 0.12 * (1.0 - abs(lum - 0.5) * 1.4);
    }
    c = pow(clamp(g2, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
    return vec4<f32>(c, 1.0);
}

// Average log luminance of the frame (one pixel) for auto exposure.
@fragment
fn fs_exposure(in: Out) -> @location(0) vec4<f32> {
    var sum = 0.0;
    let n = 16;
    for (var y = 0; y < n; y = y + 1) {
        for (var x = 0; x < n; x = x + 1) {
            let uv = (vec2<f32>(f32(x), f32(y)) + 0.5) / f32(n);
            // Weight the centre more, like a camera's metering.
            let w = 1.0;
            let l = dot(textureSampleLevel(hdr, samp, uv, 0.0).rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
            sum = sum + log(max(l, 1e-4)) * w;
        }
    }
    return vec4<f32>(exp(sum / f32(n * n)), 0.0, 0.0, 1.0);
}

