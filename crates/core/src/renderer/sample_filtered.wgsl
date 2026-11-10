// Field at a shade-pixel position, filtered between its coarser texels by the sampler.
fn sample_field(p: vec2f) -> vec4f {
    return textureSampleLevel(field, smp, p / (u.field_scale * vec2f(textureDimensions(field))), 0.0);
}
