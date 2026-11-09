@group(0) @binding(1) var field: texture_2d<f32>;

const CELL = 4.0;
const GRID = 64;

fn fload(p: vec2f) -> vec4f {
    let dim = vec2i(textureDimensions(field));
    return textureLoad(field, clamp(vec2i(p), vec2i(0), dim - 1), 0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let sc = scene();
    let opx = u.resolution.x / (u.view.z - u.view.x);
    let ij = vec2i(pos.xy);
    let centre = plane_q(sc.cam, 0.5 * (u.view.xy + u.view.zw)) - sc.qb - vec2f(FLOW_U * u.time, 0.0);
    let cc = vec2i(floor(centre / CELL));
    var d = ((ij - cc) % GRID + GRID) % GRID;
    if (d.x >= GRID / 2) { d.x -= GRID; }
    if (d.y >= GRID / 2) { d.y -= GRID; }
    let cell = cc + d;
    let h = hash22(vec2f(cell) * 1.37 + 9.1) * 0.5 + 0.5;
    let h2 = hash22(vec2f(cell) * 2.11 - 3.7) * 0.5 + 0.5;
    if (h.x > 0.7 + P(24)) { return vec4f(0.0); }
    let xi = (vec2f(cell) + 0.5 + 0.7 * (h2 - 0.5)) * CELL;
    let rel = xi + vec2f(FLOW_U * u.time, 0.0);
    let cov0 = project(sc.cam, rel + sc.qb);
    let p0 = pos_of(cov0);
    if (any(p0 < vec2f(0.0)) || any(p0 >= u.resolution)) { return vec4f(0.0); }

    let dir = normalize(vec2f(0.22, 1.0));
    let step = max(1.2 * opx / REFPX, 1.0);
    var prev = false;
    var found = -1.0;
    for (var i = 0; i < 60; i++) {
        let p = p0 + dir * step * f32(i);
        let f = fload(p);
        let lilac = f.r > 0.0 && fract(f.r) >= 1.0 - DUTY;
        if (prev && !lilac && f.r > 0.0) { found = f32(i) - 0.5; break; }
        prev = lilac;
    }
    if (found < 0.0) { return vec4f(cov0, 0.0, select(0.0, -1.0, P(31) > 0.5)); }
    let pe = p0 + dir * step * found;
    let e = 1.5;
    let gx = fload(pe + vec2f(e, 0.0)).r - fload(pe - vec2f(e, 0.0)).r;
    let gy = fload(pe + vec2f(0.0, e)).r - fload(pe - vec2f(0.0, e)).r;
    let gl = max(length(vec2f(gx, gy)) / (2.0 * e) * opx / REFPX, 1e-5);
    let wpx = DUTY / gl;
    let fe = fload(pe);
    if (wpx < 15.0 + P(25) || fe.b > 0.3) { return vec4f(cover_of(pe), 0.0, select(0.0, -2.0, P(31) > 0.5)); }
    let wc = min(wpx, 42.0);
    let grow = smoothstep(3.0, 28.0, dot(rel, sc.g));
    let len = wc * (0.9 + 1.2 * h.y) * grow / REFPX;
    let rad = wc * 0.25 / REFPX;
    if (len < rad) { return vec4f(cover_of(pe), 0.0, select(0.0, -3.0, P(31) > 0.5)); }
    return vec4f(cover_of(pe), len, rad);
}
