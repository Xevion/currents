override to_linear: bool;

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;

struct Out {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
}

// One triangle over the target; uv runs 0..1 across it, top to bottom, whatever the sizes of
// the target and the picture it is filled from.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Out;
    out.pos = vec4f(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2f(p.x, 1.0 - p.y);
    return out;
}

@fragment
fn fs_main(in: Out) -> @location(0) vec4f {
    let c = textureSampleLevel(src, smp, in.uv, 0.0).rgb;
    if (to_linear) {
        return vec4f(pow(c, vec3f(2.2)), 1.0);
    }
    return vec4f(c, 1.0);
}
