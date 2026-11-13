// Copy the scene depth to a plain float texture (nearest sample), so later
// passes can read it while the depth buffer is still attached.
@group(0) @binding(0) var ms_depth: texture_depth_multisampled_2d;

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_depth_copy(@builtin(position) clip: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(clip.xy);
    let n = i32(textureNumSamples(ms_depth));
    var d = 0.0;
    for (var i = 0; i < n; i = i + 1) {
        d = max(d, textureLoad(ms_depth, p, i));
    }
    return vec4<f32>(d, 0.0, 0.0, 1.0);
}
