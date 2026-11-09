struct U {
    resolution: vec2f,
    time: f32,
    _pad: f32,
    view: vec4f,
    params: array<vec4f, 8>,
}
@group(0) @binding(0) var<uniform> u: U;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}

fn P(i: i32) -> f32 { return u.params[i / 4][i % 4]; }

// Stripe-plane homography fitted to the cover: phi = (a x + b y + c) / (g x + h y + 1).
const HA = 106.348;
const HB = -65.956;
const HC = 21.957;
const HG = -0.02587;
const HH = 1.10028;
const FOCAL = 1.2;

const BALL_C = vec2f(0.537, 0.631);
const BALL_R = 0.118;
const REFPX = 1500.0; // "px" below means pixels of the 1500px reference cover

const FLOW_U = 4.0;     // periods per second
const STEPS = 24;
const TAU_MAX = 7.0;

struct Cam {
    n: vec3f,   // unit plane normal, pointing away from camera
    d: f32,     // plane distance; world unit = one stripe period
    eu: vec3f,  // downstream
    ev: vec3f,  // across
}

fn camera() -> Cam {
    let np = vec3f(HG, HH, (1.0 + 0.5 * (HG + HH)) / FOCAL);
    let a = vec3f(HA, HB, (HC + 0.5 * (HA + HB)) / FOCAL);
    var c: Cam;
    c.n = normalize(np);
    let ap = a - dot(a, c.n) * c.n;
    c.d = length(ap) / length(np);
    c.ev = normalize(ap);
    c.eu = cross(c.ev, c.n);
    if (c.eu.x < 0.0) { c.eu = -c.eu; }
    return c;
}

fn ray(cover: vec2f) -> vec3f { return vec3f(cover - 0.5, FOCAL); }

fn plane_q(c: Cam, cover: vec2f) -> vec2f {
    let r = ray(cover);
    let hit = r * (c.d / dot(c.n, r));
    return vec2f(dot(hit, c.eu), dot(hit, c.ev));
}

fn srgb(r: f32, g: f32, b: f32) -> vec3f { return vec3f(r, g, b) / 255.0; }

fn hash22(p: vec2f) -> vec2f {
    var p3 = fract(vec3f(p.xyx) * vec3f(0.1031, 0.1030, 0.0973));
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy) * 2.0 - 1.0;
}

fn hash12(p: vec2f) -> f32 {
    var p3 = fract(vec3f(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Gradient noise: value in x, derivative in yz.
fn noised(p: vec2f) -> vec3f {
    let i = floor(p);
    let f = fract(p);
    let w = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let dw = 30.0 * f * f * (f * (f - 2.0) + 1.0);
    let ga = hash22(i);
    let gb = hash22(i + vec2f(1.0, 0.0));
    let gc = hash22(i + vec2f(0.0, 1.0));
    let gd = hash22(i + vec2f(1.0, 1.0));
    let va = dot(ga, f);
    let vb = dot(gb, f - vec2f(1.0, 0.0));
    let vc = dot(gc, f - vec2f(0.0, 1.0));
    let vd = dot(gd, f - vec2f(1.0, 1.0));
    let k = va - vb - vc + vd;
    let v = va + w.x * (vb - va) + w.y * (vc - va) + w.x * w.y * k;
    let d = ga + w.x * (gb - ga) + w.y * (gc - ga) + w.x * w.y * (ga - gb - gc + gd)
        + dw * (w.yx * k + vec2f(vb, vc) - va);
    return vec3f(v, d);
}

fn vnoise(p: vec2f) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash12(i), hash12(i + vec2f(1.0, 0.0)), w.x),
               mix(hash12(i + vec2f(0.0, 1.0)), hash12(i + vec2f(1.0, 1.0)), w.x), w.y);
}


struct Scene {
    cam: Cam,
    bc: vec3f,   // sphere centre (camera space)
    br: f32,     // sphere radius
    tb: f32,
    qb: vec2f,   // flow disc centre on the plane
    a: f32,      // flow disc radius
    g: vec2f,    // plane direction of screen-down at the ball
}

fn scene() -> Scene {
    var s: Scene;
    s.cam = camera();
    let rb = ray(BALL_C);
    let sink = 0.6 + P(0);
    s.tb = s.cam.d / (dot(s.cam.n, rb) + sink * BALL_R);
    s.bc = rb * s.tb;
    s.br = BALL_R * s.tb;
    let foot = s.bc + s.cam.n * (sink * s.br);
    s.a = s.br * (0.9 + P(1));
    let qc = plane_q(s.cam, BALL_C);
    s.g = normalize(plane_q(s.cam, BALL_C + vec2f(0.0, 0.01)) - qc);
    s.qb = vec2f(dot(foot, s.cam.eu), dot(foot, s.cam.ev)) - s.g * s.br * (0.2 + P(2)) + vec2f(s.br * (-0.2 + P(3)), 0.0);
    return s;
}

fn project(c: Cam, q: vec2f) -> vec2f {
    let p3 = c.eu * q.x + c.ev * q.y + c.n * c.d;
    return p3.xy / p3.z * FOCAL + 0.5;
}

fn cover_of(pos: vec2f) -> vec2f {
    return u.view.xy + pos / u.resolution * (u.view.zw - u.view.xy);
}

fn pos_of(cover: vec2f) -> vec2f {
    return (cover - u.view.xy) / (u.view.zw - u.view.xy) * u.resolution;
}

const DUTY = 0.6;
