@group(0) @binding(1) var field: texture_2d<f32>;
@group(0) @binding(2) var anchors: texture_2d<f32>;

const CELL = 4.0;
const GRID = 64;

fn bilinear(p: vec2f) -> vec4f {
    let dim = vec2i(textureDimensions(field));
    let q = p - 0.5;
    let i = vec2i(floor(q));
    let f = fract(q);
    let a = textureLoad(field, clamp(i, vec2i(0), dim - 1), 0);
    let b = textureLoad(field, clamp(i + vec2i(1, 0), vec2i(0), dim - 1), 0);
    let c = textureLoad(field, clamp(i + vec2i(0, 1), vec2i(0), dim - 1), 0);
    let d = textureLoad(field, clamp(i + vec2i(1, 1), vec2i(0), dim - 1), 0);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

@fragment
fn fs_main(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let sc = scene();
    let x = cover_of(pos.xy);
    let xi = plane_q(sc.cam, x) - sc.qb - vec2f(FLOW_U * u.time, 0.0);
    let cell = vec2i(floor(xi / CELL));
    let dir = normalize(vec2f(0.22, 1.0));
    let perp = vec2f(-dir.y, dir.x);
    var src = x;
    for (var dy = -2; dy <= 2; dy++) {
        for (var dx = -2; dx <= 2; dx++) {
            let cidx = ((cell + vec2i(dx, dy)) % GRID + GRID) % GRID;
            let an = textureLoad(anchors, cidx, 0);
            if (an.w <= 0.0) { continue; }
            let e = an.xy;
            let len = an.z;
            let r = an.w;
            // bulb: magnify around the tip
            let rb = r * (2.3 + P(26));
            let tipc = e + dir * (len - 0.6 * rb);
            let dv = x - tipc;
            let dl = length(dv);
            let mag = mix(0.42 + P(27), 1.0, smoothstep(0.0, rb * 1.5, dl));
            let xb = tipc + dv * mag;
            // finger: pull band material down along dir
            let rl = xb - e;
            let al = dot(rl, dir);
            let la = dot(rl, perp);
            let lat = exp(-(la * la) / (r * r));
            let m = 1.2 * r;
            var push = 0.0;
            if (al < 0.0) {
                push = m * smoothstep(-2.5 * r, 0.0, al);
            } else {
                push = (al + m) * (1.0 - smoothstep(len, len + 1.4 * r, al));
            }
            src += (xb - x) - dir * push * lat;
        }
    }
    var o = bilinear(pos_of(src));
    if (P(31) > 0.0) {
        for (var dy = -2; dy <= 2; dy++) {
            for (var dx = -2; dx <= 2; dx++) {
                let an = textureLoad(anchors, ((cell + vec2i(dx, dy)) % GRID + GRID) % GRID, 0);
                if (an.w != 0.0 && distance(an.xy, x) < 0.004) { o.r = -1.0; o.g = select(0.0, select(2.3, 9.0, an.w < -1.5), an.w < 0.0); if (an.w < -2.5) { o.r = 0.7; } }
            }
        }
    }
    return o;
}
