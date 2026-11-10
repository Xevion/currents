// Debug views of the flow field texture, drawn in place of the shade pass. `mode` picks one:
// 1 line coordinate, 2 flow label, 3 velocity, 4 stripe density.
override mode: u32 = 1u;

@group(0) @binding(1) var field: texture_2d<f32>;
// part of the layout shared with the shade pass; the views read raw texels instead
@group(0) @binding(2) var smp: sampler;

// Screen speed (reference px per unit time) drawn at half brightness and half arrow length.
const SPEED_REF = 40.0;
// Output pixels between velocity arrows.
const ARROW_CELL = 36.0;
const TAU = 6.2831853;

fn load(c: vec2i) -> vec4f {
    return textureLoad(field, clamp(c, vec2i(0), vec2i(textureDimensions(field)) - 1), 0);
}

fn hue(h: f32) -> vec3f {
    return clamp(abs(fract(h + vec3f(0.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0) - 1.0, vec3f(0.0), vec3f(1.0));
}

// black, blue, cyan, yellow, white as t runs 0 to 1
fn heat(t: f32) -> vec3f {
    let x = clamp(t, 0.0, 1.0) * 4.0;
    let i = min(i32(x), 3);
    var k = array<vec3f, 5>(vec3f(0.0), vec3f(0.1, 0.2, 0.9), vec3f(0.0, 0.9, 0.9), vec3f(1.0, 0.9, 0.1), vec3f(1.0));
    return mix(k[i], k[i + 1], x - f32(i));
}

fn segment_distance(p: vec2f, a: vec2f, b: vec2f) -> f32 {
    let ab = b - a;
    let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
    return length(p - a - ab * t);
}

// Lines of an arrow for the velocity under its grid cell, as coverage at q (output px).
fn arrow(q: vec2f) -> f32 {
    let centre = (floor(q / ARROW_CELL) + 0.5) * ARROW_CELL;
    let v = load(vec2i(centre)).zw;
    let speed = length(v);
    if (speed < 1e-4) { return 0.0; }
    let dir = v / speed;
    let half_len = 0.45 * ARROW_CELL * speed / (speed + SPEED_REF);
    let tip = centre + dir * half_len;
    let side = vec2f(-dir.y, dir.x);
    let head = 0.35 * half_len;
    var d = segment_distance(q, centre - dir * half_len, tip);
    d = min(d, segment_distance(q, tip, tip - dir * head + side * head * 0.6));
    d = min(d, segment_distance(q, tip, tip - dir * head - side * head * 0.6));
    return 1.0 - smoothstep(0.6, 1.4, d);
}

@fragment
fn fs_main(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    // field texels are output pixels; shade pixels are field_scale times finer
    let q = pos.xy / u.field_scale;
    let i = vec2i(q);
    let f = load(i);
    let c = f.x;
    // contours at whole line periods, about a pixel wide whatever the density
    let contour = 1.0 - smoothstep(0.5, 1.5, abs(fract(c + 0.5) - 0.5) / max(fwidth(c), 1e-4));

    var col: vec3f;
    if (mode == 1u) {
        // gaps and bands as a sawtooth, so the darker side shows which way c grows
        col = vec3f(mix(0.12, 0.8, fract(c)));
        if (c < 0.0) { col = vec3f(0.85, 0.15, 0.2) * mix(0.5, 1.0, fract(-c)); }
        col = mix(col, vec3f(1.0, 0.85, 0.2), contour);
    } else if (mode == 2u) {
        col = hue(fract(f.y / 16.0)) * mix(0.45, 1.0, fract(f.y));
    } else if (mode == 3u) {
        let v = f.zw;
        let speed = length(v);
        let angle = atan2(v.y, v.x) / TAU + 0.5;
        col = hue(angle) * (speed / (speed + SPEED_REF));
        col = mix(col, vec3f(1.0), arrow(q));
    } else {
        // line periods per output pixel; 0.5 is two pixels per period, where stripes alias
        let gx = load(i + vec2i(1, 0)).x - load(i - vec2i(1, 0)).x;
        let gy = load(i + vec2i(0, 1)).x - load(i - vec2i(0, 1)).x;
        let density = 0.5 * length(vec2f(gx, gy));
        col = heat(density / 0.5);
        if (density > 0.5) { col = vec3f(1.0, 0.1, 0.9); }
        if (c < 0.0) { col *= 0.35; }
    }
    return vec4f(col, 1.0);
}
