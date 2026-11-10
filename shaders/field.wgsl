// Eddies live between two fronts that run across the picture below the ball (distances along
// the plane direction of screen-down, in periods). Past the far front the pattern only drifts.
const EDDY_RISE_START = -2.0;
const EDDY_RISE_END = 6.0;
const EDDY_FADE_START = 12.0;
const EDDY_FADE_END = 26.0;

// Trace step in seconds, and the most steps a trace takes to get upstream of the eddies.
// Only liquid all but stopped behind the ball needs them all.
const MAX_TRACE_DT = 0.25;
const MAX_TRACE_STEPS = 192;
const FLOW_SPEED = 4.0;   // periods per second

// Set for frames that cannot build on the one before (the camera is moving): they trace
// every pixel in full, so the trace gives up the accuracy that keeps successive keys in
// agreement for about four times the speed.
override rough: bool = false;

fn flow_noise(p: vec2f) -> vec3f {
    if (rough) { return gradient_noise(p); }
    return gradient_noise_smooth(p);
}

// smoothstep and its slope
fn smoothstep_with_slope(a: f32, b: f32, x: f32) -> vec2f {
    let t = clamp((x - a) / (b - a), 0.0, 1.0);
    return vec2f(t * t * (3.0 - 2.0 * t), 6.0 * t * (1.0 - t) / (b - a));
}

// Envelopes of the eddy stream function (x for the noise, y for the sinuous mode) and their gradients.
struct EddyEnvelope {
    amp: vec2f,
    grad_noise: vec2f,
    grad_sinuous: vec2f,
}

fn eddy_envelope(y: vec2f, scn: Scene) -> EddyEnvelope {
    let dn = dot(y, scn.down);
    let f1 = smoothstep_with_slope(EDDY_RISE_START, EDDY_RISE_END + P(P_EDDY_RISE_END), dn);
    let f2 = smoothstep_with_slope(EDDY_FADE_START + P(P_EDDY_FADE_START), EDDY_FADE_END + P(P_EDDY_FADE_END), dn);
    let front = f1.x * (1.0 - f2.x);
    let dfront = (f1.y * (1.0 - f2.x) - f1.x * f2.y) * scn.down;
    // nothing is stirred toward the left and right of the picture
    let p3 = scn.cam.eu * (y.x + scn.centre.x) + scn.cam.ev * (y.y + scn.centre.y) + scn.cam.n * scn.cam.d;
    let sx = p3.x / p3.z * FOCAL + 0.5 - BALL_C.x;
    let dsx = FOCAL * (vec2f(scn.cam.eu.x, scn.cam.ev.x) * p3.z - p3.x * vec2f(scn.cam.eu.z, scn.cam.ev.z)) / (p3.z * p3.z);
    let ed = select(smoothstep_with_slope(0.42, 0.14, sx), smoothstep_with_slope(-0.48, -0.16, sx), sx < 0.0);
    let r = max(length(y), 1e-4);
    let nr = smoothstep_with_slope(scn.radius * 1.05, scn.radius * 1.6, r);
    let e = front * ed.x * nr.x;
    let de = (dfront * ed.x + front * ed.y * dsx) * nr.x + front * ed.x * nr.y * y / r;
    // the stream itself stays smooth: only the slow sinuous mode bends it
    let k = 0.12 + P(P_CORE_NOISE_DAMP);
    let cd = smoothstep_with_slope(0.5, 5.0, abs(y.y));
    let damp = mix(k, 1.0, cd.x);
    var o: EddyEnvelope;
    o.amp = vec2f(e * damp, e);
    o.grad_noise = de * damp + e * vec2f(0.0, (1.0 - k) * cd.y * sign(y.y));
    o.grad_sinuous = de;
    return o;
}

// The liquid slows behind the ball, so lines spread apart there. Stream function
// -U D s tanh(y/s): a deficit D sech^2(y/s) across the wake, rising once clear of the ball.
// Returns D and its gradient.
fn wake_peak_deficit(y: vec2f, scn: Scene) -> vec3f {
    let dm = 0.55 + P(P_WAKE_DEFICIT);
    let ax = smoothstep_with_slope((0.7 + P(P_WAKE_START_SHIFT)) * scn.radius, (3.7 + P(P_WAKE_START_SHIFT)) * scn.radius, y.x);
    let bn = smoothstep_with_slope(-5.0, 0.0, dot(y, scn.down));
    return dm * vec3f(ax.x * bn.x, vec2f(ax.y * bn.x, 0.0) + ax.x * bn.y * scn.down);
}

// Cross-stream profile of the wake stream function, s tanh(y/s) faded out a few wake widths
// to the side so the deficit's ramps do not push the liquid sideways across the whole picture.
// Returns the profile and its slope.
fn wake_profile(yy: f32) -> vec2f {
    let s = 10.0 + P(P_WAKE_WIDTH);
    let w = 3.0 * s;
    let th = tanh(yy / s);
    let fade = exp(-(yy * yy) / (w * w));
    return vec2f(s * th * fade, ((1.0 - th * th) - s * th * 2.0 * yy / (w * w)) * fade);
}

fn wake_velocity(y: vec2f, scn: Scene) -> vec2f {
    let d = wake_peak_deficit(y, scn);
    let g = wake_profile(y.y);
    return -FLOW_SPEED * vec2f(d.z * g.x + d.x * g.y, -d.y * g.x);
}

// Velocity at lab position y (relative to the ball): potential flow round the disc plus the
// curl of an eddy stream function. Both parts are divergence free, so areas are preserved.
fn flow_velocity(y: vec2f, t: f32, scn: Scene) -> vec2f {
    let r2 = max(dot(y, y), 1e-4);
    let q = min(scn.radius * scn.radius / r2, 1.0);
    var v = FLOW_SPEED * vec2f(1.0 - q + 2.0 * q * y.y * y.y / r2, -2.0 * q * y.x * y.y / r2);
    v += wake_velocity(y, scn);

    let xi = y - vec2f(FLOW_SPEED * t, 0.0);
    let lx = 9.0 + P(P_EDDY_LEN_X);
    let ly = 12.0 + P(P_EDDY_LEN_Y);
    let l2 = 5.5 + P(P_EDDY_LEN_SMALL);
    let a1 = 1.4 + P(P_EDDY_AMP_LARGE);
    let a2 = 0.4 + P(P_EDDY_AMP_SMALL);
    let n1 = flow_noise(vec2f(xi.x / lx, xi.y / ly) + vec2f(3.7, 1.3) + vec2f(0.0, 0.04 * t));
    let n2 = flow_noise(xi / l2 + vec2f(-8.1, 5.9) + vec2f(0.05 * t, 0.0));
    let nv = a1 * lx * n1.x + a2 * l2 * n2.x;
    let ng = vec2f(a1 * n1.y + a2 * n2.y, a1 * lx / ly * n1.z + a2 * n2.z);

    let km = 6.2831853 / 26.0;
    let hm = 9.0;
    let am = 0.5 + P(P_SINUOUS_AMP);
    let gy = exp(-(y.y * y.y) / (hm * hm));
    let sv = am / km * sin(km * xi.x) * gy;
    let sg = vec2f(am * cos(km * xi.x) * gy, sv * (-2.0 * y.y / (hm * hm)));

    let en = eddy_envelope(y, scn);
    let grad = en.amp.x * ng + nv * en.grad_noise + en.amp.y * sg + sv * en.grad_sinuous;
    return v + vec2f(grad.y, -grad.x);
}

// Distance downstream of the line the eddies begin after; at or above it (zero or less) the
// flow is steady.
fn inflow_distance(y: vec2f, scn: Scene) -> f32 {
    return dot(y, scn.down) - (EDDY_RISE_START - 3.0);
}

// The liquid at y in the steady flow at time t: the stream function there, which labels its
// line (x), and a label that rides with it along the line (y).
fn steady_liquid(y: vec2f, t: f32, scn: Scene) -> vec2f {
    let r2 = max(dot(y, y), 1e-6);
    return vec2f(y.y * (1.0 - min(scn.radius * scn.radius / r2, 1.0)), y.x - FLOW_SPEED * t);
}

// Where the liquid at y was dt before time t. Fourth order: a trace has to land in the same
// place wherever its steps fall, or the picture shifts each time a key retraces it.
fn step_back(y: vec2f, t: f32, dt: f32, scn: Scene) -> vec2f {
    let half = 0.5 * dt;
    let v1 = flow_velocity(y, t, scn);
    let v2 = flow_velocity(y - v1 * half, t - half, scn);
    if (rough) { return y - v2 * dt; }
    let v3 = flow_velocity(y - v2 * half, t - half, scn);
    let v4 = flow_velocity(y - v3 * dt, t - dt, scn);
    return y - (v1 + 2.0 * (v2 + v3) + v4) * (dt / 6.0);
}

// The liquid met where a step back from y (inflow distance d, at time t) to a point at distance
// dn, dt earlier, crossed into the steady flow. Both of its labels keep from there on, so every
// trace of the same liquid agrees on them however long it runs. The crossing is stepped to
// rather than read off the chord, which cuts the corner where the flow bends round the ball.
fn liquid_at_inflow(y: vec2f, d: f32, dn: f32, t: f32, dt: f32, scn: Scene) -> vec2f {
    let first = dt * d / (d - dn);
    // one correction for the flow speeding up or slowing down across the step
    let part = first * d / (d - inflow_distance(step_back(y, t, first, scn), scn));
    return steady_liquid(step_back(y, t, part, scn), t - part, scn);
}

// Follows the liquid at p back from time t until it is upstream of the eddies, and returns it
// as steady_liquid does.
fn traced(p: vec2f, t: f32, scn: Scene) -> vec2f {
    var y = p;
    var d = inflow_distance(y, scn);
    if (d <= 0.0) { return steady_liquid(y, t, scn); }
    // every trace runs until it is clear, however slow the water: one that stopped among the
    // eddies would give a different answer for the same liquid from a different start
    // a rough trace stops at a third of the steps, leaving only the slowest water unfinished
    let steps = select(MAX_TRACE_STEPS, MAX_TRACE_STEPS / 3, rough);
    for (var i = 0; i < steps; i++) {
        let tt = t - f32(i) * MAX_TRACE_DT;
        let yn = step_back(y, tt, MAX_TRACE_DT, scn);
        let dn = inflow_distance(yn, scn);
        if (dn <= 0.0) { return liquid_at_inflow(y, d, dn, tt, MAX_TRACE_DT, scn); }
        y = yn;
        d = dn;
    }
    return steady_liquid(y, t - f32(steps) * MAX_TRACE_DT, scn);
}

// A key is the traced liquid over the view and a margin round it, at a time a little before
// the frame's. While one is held, a frame follows the liquid back only as far as the key's
// time and reads the rest of its history there. Liquid from outside the key is traced in full.
@group(0) @binding(3) var key: texture_2d<f32>;

fn catmull_rom(f: f32) -> vec4f {
    let f2 = f * f;
    let f3 = f2 * f;
    return vec4f(-0.5 * f3 + f2 - 0.5 * f, 1.5 * f3 - 2.5 * f2 + 1.0, -1.5 * f3 + 2.0 * f2 + 0.5 * f, 0.5 * f3 - 0.5 * f2);
}

// The liquid at p now, followed back to the key's time: where it was then, or, when the way
// there took it into the steady flow, what it is.
struct Hop {
    at: vec2f,
    liquid: vec2f,
    steady: bool,
}

fn hop_back(p: vec2f, scn: Scene) -> Hop {
    var o: Hop;
    var y = p;
    var d = inflow_distance(y, scn);
    o.steady = true;
    if (d <= 0.0) {
        o.liquid = steady_liquid(y, u.time, scn);
        return o;
    }
    let span = u.time - u.key_time;
    let n = max(i32(ceil(span / MAX_TRACE_DT)), 1);
    let h = span / f32(n);
    for (var i = 0; i < n; i++) {
        let tt = u.time - f32(i) * h;
        let yn = step_back(y, tt, h, scn);
        let dn = inflow_distance(yn, scn);
        if (dn <= 0.0) {
            o.liquid = liquid_at_inflow(y, d, dn, tt, h, scn);
            return o;
        }
        y = yn;
        d = dn;
    }
    o.steady = false;
    o.at = y;
    return o;
}

// The liquid that was at y at the key's time.
fn key_liquid(y: vec2f, scn: Scene) -> vec2f {
    let cover = plane_to_cover(scn.cam, y + scn.centre);
    let g = (cover - u.key_view.xy) / (u.key_view.zw - u.key_view.xy) * u.key_size - 0.5;
    let cell = floor(g);
    if (any(cell < vec2f(1.0)) || any(cell > u.key_size - 3.0)) { return traced(y, u.key_time, scn); }
    let wx = catmull_rom(g.x - cell.x);
    let wy = catmull_rom(g.y - cell.y);
    var acc = vec2f(0.0);
    for (var j = 0; j < 4; j++) {
        var row = vec2f(0.0);
        for (var i = 0; i < 4; i++) {
            row += wx[i] * textureLoad(key, vec2i(cell) + vec2i(i - 1, j - 1), 0).xy;
        }
        acc += wy[j] * row;
    }
    return acc;
}

// The liquid at p now, by way of the key.
fn carried(p: vec2f, scn: Scene) -> vec2f {
    let hop = hop_back(p, scn);
    if (hop.steady) { return hop.liquid; }
    return key_liquid(hop.at, scn);
}

// On-screen velocity of the liquid at rel, in reference pixels per unit time.
fn screen_velocity(rel: vec2f, scn: Scene) -> vec2f {
    let v = flow_velocity(rel, u.time, scn);
    let p0 = plane_to_cover(scn.cam, rel + scn.centre);
    return (plane_to_cover(scn.cam, rel + scn.centre + v * 0.01) - p0) * (REFPX / 0.01);
}

// The way back to the key's time bends little over a few pixels, so while a key is held it is
// worked out once every `hop_step` field texels each way, as the shift from here to there, and
// the field pass blends between those. The key itself is still read at every texel.
override hop_step: f32 = 4.0;
@group(0) @binding(4) var hops: texture_2d<f32>;

// Marks a hop that ended in the steady flow, or has no plane under it: it has no shift to blend.
const NO_HOP = 1e30;

@fragment
fn fs_hop(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let scn = scene();
    // texel centres sit a texel outside the picture at its top and left, so every field
    // texel has one on each side
    let cover = pos_to_cover((pos.xy - 1.5) * hop_step * u.field_scale);
    if (dot(scn.cam.n, ray(cover)) <= 1e-4) { return vec4f(NO_HOP); }
    let rel = cover_to_plane(scn.cam, cover) - scn.centre;
    let hop = hop_back(rel, scn);
    let shift = select(hop.at - rel, vec2f(NO_HOP), hop.steady);
    return vec4f(shift, screen_velocity(rel, scn));
}

// A key, drawn with the key's own span of the cover as the view and its time as the frame's.
// Given the key before it, it is that key carried forward: the liquid's history is traced once,
// when it first comes into a key, and handed on from each key to the next.
@fragment
fn fs_key(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let scn = scene();
    let rel = cover_to_plane(scn.cam, pos_to_cover(pos.xy)) - scn.centre;
    if (u.key_size.x > 0.0) { return vec4f(carried(rel, scn), 0.0, 0.0); }
    return vec4f(traced(rel, u.time, scn), 0.0, 0.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let cover = pos_to_cover(pos.xy * u.field_scale);
    let scn = scene();
    let rel = cover_to_plane(scn.cam, cover) - scn.centre;
    var liquid: vec2f;
    var vs: vec2f;
    if (u.key_size.x > 0.0) {
        let g = pos.xy / hop_step + 1.0;
        let cell = vec2i(floor(g));
        let f = fract(g);
        let h00 = textureLoad(hops, cell, 0);
        let h10 = textureLoad(hops, cell + vec2i(1, 0), 0);
        let h01 = textureLoad(hops, cell + vec2i(0, 1), 0);
        let h11 = textureLoad(hops, cell + vec2i(1, 1), 0);
        let acc = mix(mix(h00, h10, f.x), mix(h01, h11, f.x), f.y);
        let whole = max(max(h00.x, h10.x), max(h01.x, h11.x)) < 0.5 * NO_HOP;
        // by a hop that ended in the steady flow, each texel follows its own liquid back
        if (whole) {
            liquid = key_liquid(rel + acc.xy, scn);
        } else {
            liquid = carried(rel, scn);
        }
        if (all(abs(acc.zw) < vec2f(1e9))) {
            vs = acc.zw;
        } else {
            vs = screen_velocity(rel, scn);
        }
    } else {
        liquid = traced(rel, u.time, scn);
        vs = screen_velocity(rel, scn);
    }
    let a = scn.radius;
    let rr = length(rel);
    // the stream flares into a pool around the obstacle and widens a little downstream
    let pool = (0.7 + P(P_POOL_FLARE)) * smoothstep(a * 1.8, a * 1.0, rr) * smoothstep(0.9, -0.3, dot(rel, scn.down) / a);
    let hw = 0.8 + P(P_STREAM_HALF_WIDTH) + pool;
    let c = abs(liquid.x) - hw;
    return vec4f(c, liquid.y, vs);
}
