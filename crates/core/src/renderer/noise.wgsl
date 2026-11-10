// Bakes one level of the noise tile: gradient noise on a lattice that wraps every NOISE_PERIOD
// cells, with its value in x and its derivative in yz, `texels` samples across the tile.
override texels: f32;

fn corner(i: vec2f) -> vec2f {
    return hash22(i - NOISE_PERIOD * floor(i / NOISE_PERIOD));
}

@fragment
fn fs_main(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let p = pos.xy / texels * NOISE_PERIOD;
    let i = floor(p);
    let f = fract(p);
    let w = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let dw = 30.0 * f * f * (f * (f - 2.0) + 1.0);
    let ga = corner(i);
    let gb = corner(i + vec2f(1.0, 0.0));
    let gc = corner(i + vec2f(0.0, 1.0));
    let gd = corner(i + vec2f(1.0, 1.0));
    let va = dot(ga, f);
    let vb = dot(gb, f - vec2f(1.0, 0.0));
    let vc = dot(gc, f - vec2f(0.0, 1.0));
    let vd = dot(gd, f - vec2f(1.0, 1.0));
    let k = va - vb - vc + vd;
    let v = va + w.x * (vb - va) + w.y * (vc - va) + w.x * w.y * k;
    let d = ga + w.x * (gb - ga) + w.y * (gc - ga) + w.x * w.y * (ga - gb - gc + gd)
        + dw * (w.yx * k + vec2f(vb, vc) - va);
    return vec4f(v, d, 0.0);
}
