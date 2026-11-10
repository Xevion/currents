fn load_field(c: vec2i) -> vec4f {
    return textureLoad(field, clamp(c, vec2i(0), vec2i(textureDimensions(field)) - 1), 0);
}

// Field at a shade-pixel position, blended between its four nearest texels by hand for
// devices that cannot filter 32-bit float textures.
fn sample_field(p: vec2f) -> vec4f {
    let t = p / u.field_scale - 0.5;
    let i = vec2i(floor(t));
    let f = t - floor(t);
    let top = mix(load_field(i), load_field(i + vec2i(1, 0)), f.x);
    let bottom = mix(load_field(i + vec2i(0, 1)), load_field(i + vec2i(1, 1)), f.x);
    return mix(top, bottom, f.y);
}
