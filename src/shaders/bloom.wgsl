// Bloom: soft-threshold bright pass + separable gaussian blur.
struct BloomU {
    dir: vec4<f32>,   // xy = texel step, z = threshold
};
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> u: BloomU;

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
fn fs_bright(in: Out) -> @location(0) vec4<f32> {
    let px = 1.0 / vec2<f32>(textureDimensions(src));
    var c = vec3<f32>(0.0);
    for (var x = -1; x <= 2; x = x + 1) {
        for (var y = -1; y <= 2; y = y + 1) {
            c = c + textureSampleLevel(src, samp, in.uv + vec2<f32>(f32(x) - 0.5, f32(y) - 0.5) * px, 0.0).rgb;
        }
    }
    c = c / 16.0;
    let l = max(c.r, max(c.g, c.b));
    let t = u.dir.z;
    let soft = clamp(l - t * 0.5, 0.0, t) ;
    let w = max(soft * soft / (2.0 * t + 1e-4), l - t) / max(l, 1e-4);
    return vec4<f32>(min(c * w, vec3<f32>(64.0)), 1.0);
}

@fragment
fn fs_blur(in: Out) -> @location(0) vec4<f32> {
    var w = array<f32, 5>(0.227027, 0.1945946, 0.1216216, 0.054054, 0.016216);
    var c = textureSampleLevel(src, samp, in.uv, 0.0).rgb * w[0];
    for (var i = 1; i < 5; i = i + 1) {
        let o = u.dir.xy * f32(i) * 1.6;
        c = c + textureSampleLevel(src, samp, in.uv + o, 0.0).rgb * w[i];
        c = c + textureSampleLevel(src, samp, in.uv - o, 0.0).rgb * w[i];
    }
    return vec4<f32>(c, 1.0);
}
