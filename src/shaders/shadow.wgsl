// Depth-only pass for one shadow cascade.
struct ShadowU {
    light_vp: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> u: ShadowU;

@vertex
fn vs_shadow(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return u.light_vp * vec4<f32>(pos, 1.0);
}

// Alpha-tested casters: leaves and fences cast shadows of their shape, not their quads.
struct MaterialU {
    base: vec4<f32>,
    emissive: vec4<f32>,
    params: vec4<f32>,
    extra: vec4<f32>,
    uvt: vec4<f32>,
    extra2: vec4<f32>,
    extra3: vec4<f32>,
    extra4: vec4<f32>,      // real-world box mapping
};
@group(1) @binding(0) var<uniform> mat: MaterialU;
@group(1) @binding(1) var base_tex: texture_2d<f32>;
@group(1) @binding(2) var base_sampler: sampler;

struct CutOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_cut(@location(0) pos: vec3<f32>, @location(1) uv: vec2<f32>) -> CutOut {
    var o: CutOut;
    o.clip = u.light_vp * vec4<f32>(pos, 1.0);
    let uv0 = (uv * mat.uvt.xy + mat.uvt.zw) * mat.params.w;
    let rc = cos(mat.extra2.y);
    let rs = sin(mat.extra2.y);
    o.uv = vec2<f32>(uv0.x * rc - uv0.y * rs, uv0.x * rs + uv0.y * rc) + mat.extra2.zw;
    return o;
}

@fragment
fn fs_cut(in: CutOut) {
    let a = textureSample(base_tex, base_sampler, in.uv).a * mat.base.a;
    if (a < 0.5) {
        discard;
    }
}
