// The clock restarts from zero every EPOCH seconds of flow, which keeps it precise however
// long the picture runs. Everything that drifts with time is given where it had drifted to at
// the restart. A restart moves every flow label by FLOW_SPEED * EPOCH = 6400, and every
// pattern drawn from the labels repeats over that, so it does not show.
const EPOCH = 1600.0;
const FLOW_SPEED = 4.0;   // periods per second

struct Uniforms {
    resolution: vec2f,      // shade pixels
    time: f32,              // seconds since the clock restarted
    field_scale: f32,       // shade pixels per field texel
    view: vec4f,
    supersampling: f32,     // shade pixels per output pixel
    key_time: f32,          // time of the key the field pass reads
    key_size: vec2f,        // texels across the key; zero when none is held
    key_view: vec4f,        // part of the cover the key spans
    prev_view: vec4f,       // the view of the frame before
    jitter: vec2f,          // shift of this frame's shading points, in shade pixels
    elapsed: f32,           // time since the frame before
    history: f32,           // frames blended so far that this one builds on
    eddy_drift: vec4f,      // the eddy noise (xyz) and sinuous swing (w) at the restart
    spin: f32,              // the ball's turn at the restart
    glare_drift: f32,       // the glare's flicker at the restart
    key_shift: f32,         // added to labels read from the key, whose clock restarted earlier
    epoch: f32,             // restarts so far; for the renderer, not read here
    params: array<vec4f, 8>,
}
@group(0) @binding(0) var<uniform> u: Uniforms;

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

const DUTY = 0.52;

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

fn cover_to_plane(c: Cam, cover: vec2f) -> vec2f {
    let r = ray(cover);
    let hit = r * (c.d / dot(c.n, r));
    return vec2f(dot(hit, c.eu), dot(hit, c.ev));
}

fn plane_to_cover(c: Cam, q: vec2f) -> vec2f {
    let p3 = c.eu * q.x + c.ev * q.y + c.n * c.d;
    return p3.xy / p3.z * FOCAL + 0.5;
}

fn pos_to_cover(pos: vec2f) -> vec2f {
    return u.view.xy + pos / u.resolution * (u.view.zw - u.view.xy);
}

fn cover_to_pos(cover: vec2f) -> vec2f {
    return (cover - u.view.xy) / (u.view.zw - u.view.xy) * u.resolution;
}

fn rgb255(r: f32, g: f32, b: f32) -> vec3f { return vec3f(r, g, b) / 255.0; }

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

// Gradient noise: value in x, derivative in yz. Read from a tile the renderer bakes once,
// NOISE_PERIOD lattice cells across, so the pattern repeats that far apart.
const NOISE_PERIOD = 64.0;
@group(0) @binding(1) var noise_tile: texture_2d<f32>;
@group(0) @binding(2) var noise_smp: sampler;

fn gradient_noise(p: vec2f) -> vec3f {
    return textureSampleLevel(noise_tile, noise_smp, p / NOISE_PERIOD, 0.0).xyz;
}

// The noise blended across four texels each way by a cubic B-spline rather than two by a
// line. Linear blending leaves creases, and a trace that crosses them lands differently
// depending on where its steps fall. Four sampler taps placed between texel pairs do the
// sixteen-texel blend.
fn gradient_noise_smooth(p: vec2f) -> vec3f {
    let size = vec2f(textureDimensions(noise_tile));
    let t = p / NOISE_PERIOD * size - 0.5;
    let i = floor(t);
    let f = t - i;
    let g = 1.0 - f;
    let w0 = g * g * g / 6.0;
    let w1 = (3.0 * f * f * f - 6.0 * f * f + 4.0) / 6.0;
    let w3 = f * f * f / 6.0;
    let w2 = 1.0 - w0 - w1 - w3;
    let lo = w0 + w1;
    let hi = w2 + w3;
    let at_lo = (i - 0.5 + w1 / lo) / size;
    let at_hi = (i + 1.5 + w3 / hi) / size;
    let a = textureSampleLevel(noise_tile, noise_smp, vec2f(at_lo.x, at_lo.y), 0.0).xyz;
    let b = textureSampleLevel(noise_tile, noise_smp, vec2f(at_hi.x, at_lo.y), 0.0).xyz;
    let c = textureSampleLevel(noise_tile, noise_smp, vec2f(at_lo.x, at_hi.y), 0.0).xyz;
    let d = textureSampleLevel(noise_tile, noise_smp, vec2f(at_hi.x, at_hi.y), 0.0).xyz;
    return lo.y * (lo.x * a + hi.x * b) + hi.y * (lo.x * c + hi.x * d);
}

// The same noise where neighbouring pixels are `stride` lattice cells apart: read from a
// level of the tile about that coarse, so a minified pattern does not skip across the tile.
fn gradient_noise_spaced(p: vec2f, stride: f32) -> vec3f {
    let texels = f32(textureDimensions(noise_tile).x) / NOISE_PERIOD;
    return textureSampleLevel(noise_tile, noise_smp, p / NOISE_PERIOD, log2(max(stride * texels, 1.0))).xyz;
}

// Value noise repeating every `period` cells along x, centred on zero.
fn value_noise(p: vec2f, period: f32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let w = f * f * (3.0 - 2.0 * f);
    let x = vec2f(i.x, i.x + 1.0);
    let c = x - period * floor(x / period + 0.5);
    return mix(mix(hash12(vec2f(c.x, i.y)), hash12(vec2f(c.y, i.y)), w.x),
               mix(hash12(vec2f(c.x, i.y + 1.0)), hash12(vec2f(c.y, i.y + 1.0)), w.x), w.y);
}

struct Scene {
    cam: Cam,
    centre: vec2f, // obstacle centre on the plane
    radius: f32,   // obstacle radius on the plane
    down: vec2f,   // plane direction of screen-down at the ball
}

// The ball is a screen-space circle; the flow obstacle is a plane disc under its lower half,
// so lines wrap the lower silhouette and pass behind the top.
fn scene() -> Scene {
    var s: Scene;
    s.cam = camera();
    let oc = BALL_C + BALL_R * vec2f(-0.12 + P(P_OBSTACLE_SHIFT_X), -0.25 + P(P_OBSTACLE_SHIFT_Y));
    s.centre = cover_to_plane(s.cam, oc);
    s.radius = length(cover_to_plane(s.cam, oc + vec2f(0.84 * BALL_R, 0.0)) - s.centre) * (1.0 + P(P_OBSTACLE_SCALE));
    s.down = normalize(cover_to_plane(s.cam, oc + vec2f(0.0, 0.01)) - s.centre);
    return s;
}
