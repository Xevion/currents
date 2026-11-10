struct U {
    resolution: vec2f,
    time: f32,
    fscale: f32,
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

const P_OBSTACLE_SCALE = 1;      // flow obstacle radius, relative
const P_OBSTACLE_SHIFT_X = 2;    // obstacle centre shift across the ball, in ball radii
const P_OBSTACLE_SHIFT_Y = 3;    // obstacle centre shift down the ball, in ball radii
const P_EDDY_AMP_LARGE = 4;      // strength of the large eddies
const P_EDDY_AMP_SMALL = 5;      // strength of the small eddies
const P_SINUOUS_AMP = 6;         // strength of the slow sinuous swing
const P_GLARE_GAIN = 7;          // brightness of the glare round the highlights
const P_EDDY_LEN_X = 8;          // large eddy length along the flow
const P_EDDY_LEN_Y = 9;          // large eddy length across the flow
const P_EDDY_LEN_SMALL = 10;     // small eddy size
const P_REFLECTION_BLUR = 11;    // blur of the liquid mirrored in the ball
const P_EDDY_FADE_START = 12;    // where eddies begin fading out downstream
const P_EDDY_FADE_END = 13;      // where eddies are gone and the pattern only drifts
const P_EDDY_RISE_END = 14;      // where eddies reach full strength downstream of the ball
const P_BOUNCE_TINT = 15;        // liquid colour bounced onto the ball
const P_BAND_EDGE_TONE = 16;     // tone of the lit edge of a band
const P_BAND_BODY_TONE = 17;     // tone of the body of a band
const P_BALL_SHADOW = 18;        // darkness of the ball's shadow on the liquid
const P_CORE_NOISE_DAMP = 19;    // eddy noise left along the stream core
const P_BALL_REFLECTION = 20;    // strength of the liquid mirrored in the ball
const P_WAKE_DEFICIT = 21;       // how much the liquid slows behind the ball
const P_WAKE_WIDTH = 22;         // width of the slow wake
const P_POOL_FLARE = 23;         // how far the stream flares round the obstacle
const P_STREAM_HALF_WIDTH = 24;  // half width of the coloured stream
const P_FAT_BAND_FLATTEN = 25;   // flattens airbrushed shading of fat bands
const P_WAKE_START_SHIFT = 26;   // moves the wake onset downstream, in obstacle radii
const P_BALL_SPIN = 27;          // turning rate of the ball
const P_BAND_WIDTH_JITTER = 28;  // band width variation along the flow
const P_SPRAY_MOTTLE = 29;       // raggedness of the far spray edge
const P_FLAKE_TILT = 30;         // tilt spread of the ball's metallic flakes
const P_DEBUG_BALL_OUTLINE = 31; // debug toggle: dims the ball, outlines its silhouette

// Stripe-plane homography fitted to the cover: phi = (a x + b y + c) / (g x + h y + 1).
const HA = 106.348;
const HB = -65.956;
const HC = 21.957;
const HG = -0.02587;
const HH = 1.10028;
const FOCAL = 1.2;

const BALL_C = vec2f(0.543, 0.661);
const BALL_R = 0.1205;
const REFPX = 1500.0; // "px" below means pixels of the 1500px reference cover

const FLOW_U = 4.0;     // periods per second
const STEPS = 64;

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
    qb: vec2f,   // obstacle centre on the plane
    a: f32,      // obstacle radius on the plane
    g: vec2f,    // plane direction of screen-down at the ball
}

// The ball is a screen-space circle; the flow obstacle is a plane disc under its lower half,
// so lines wrap the lower silhouette and pass behind the top.
fn scene() -> Scene {
    var s: Scene;
    s.cam = camera();
    let oc = BALL_C + BALL_R * vec2f(-0.12 + P(P_OBSTACLE_SHIFT_X), -0.25 + P(P_OBSTACLE_SHIFT_Y));
    s.qb = plane_q(s.cam, oc);
    s.a = length(plane_q(s.cam, oc + vec2f(0.84 * BALL_R, 0.0)) - s.qb) * (1.0 + P(P_OBSTACLE_SCALE));
    s.g = normalize(plane_q(s.cam, oc + vec2f(0.0, 0.01)) - s.qb);
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

const DUTY = 0.52;
