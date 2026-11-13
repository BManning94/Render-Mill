// Average the path tracer's accumulation buffers on the GPU and pack them as
// fp16 for read-back: colour (RGB, A = 1), albedo (RGB, A = first-hit
// distance) and normal (XYZ). Rows are padded to 256 bytes so the colour
// plane can be uploaded straight back as the renderer's HDR image, and all
// three planes can be handed to Open Image Denoise as HALF3 images with an
// 8-byte pixel stride. 24 bytes per pixel instead of 48 for the raw float
// buffers: half the PCIe / Thunderbolt traffic per preview update.

struct ResolveParams {
    size: vec4<u32>, // width, height, row stride in pixels, unused
};

@group(0) @binding(0) var<storage, read> accum: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read> aux: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> packed: array<vec2<u32>>;
@group(0) @binding(3) var<uniform> rp: ResolveParams;

fn pack4(v: vec4<f32>) -> vec2<u32> {
    // f16 range is +-65504; keep values finite.
    let c = clamp(v, vec4<f32>(-65000.0), vec4<f32>(65000.0));
    return vec2<u32>(pack2x16float(c.xy), pack2x16float(c.zw));
}

@compute @workgroup_size(8, 8)
fn resolve(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = rp.size.x;
    let h = rp.size.y;
    if (gid.x >= w || gid.y >= h) {
        return;
    }
    let i = gid.y * w + gid.x;
    let n = w * h;
    let o = gid.y * rp.size.z + gid.x;
    let plane = rp.size.z * h;
    let a = accum[i];
    let k = 1.0 / max(a.w, 1.0);
    var c = a.rgb * k;
    if (any(c != c)) {
        c = vec3<f32>(0.0);
    }
    let b = aux[i];
    let alb = clamp(b.rgb * k, vec3<f32>(0.0), vec3<f32>(1.0));
    let nn = aux[n + i].xyz;
    var nrm = vec3<f32>(0.0, 1.0, 0.0);
    if (dot(nn, nn) > 1e-12) {
        nrm = normalize(nn);
    }
    packed[o] = pack4(vec4<f32>(max(c, vec3<f32>(0.0)), 1.0));
    packed[plane + o] = pack4(vec4<f32>(alb, b.w * k));
    packed[2u * plane + o] = pack4(vec4<f32>(nrm, 0.0));
}
