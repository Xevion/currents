override scale: f32;
override to_linear: bool;

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let dim = vec2f(textureDimensions(src));
    let c = textureSampleLevel(src, smp, pos.xy * scale / dim, 0.0).rgb;
    if (to_linear) {
        return vec4f(pow(c, vec3f(2.2)), 1.0);
    }
    return vec4f(c, 1.0);
}
