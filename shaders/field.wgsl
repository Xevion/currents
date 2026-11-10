// Eddies live between two fronts that run across the picture below the ball (distances along
// the plane direction of screen-down, in periods). Past the far front the pattern only drifts.
const EDDY_RISE_START = -2.0;
const EDDY_RISE_END = 6.0;
const EDDY_FADE_START = 12.0;
const EDDY_FADE_END = 26.0;

// Trace step in seconds, longer only when a trace would take more than MAX_TRACE_STEPS.
const MAX_TRACE_DT = 0.25;
const MAX_TRACE_STEPS = 64;
const FLOW_SPEED = 4.0;   // periods per second

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

fn wake_deficit(y: vec2f, scn: Scene) -> f32 {
    return wake_peak_deficit(y, scn).x * wake_profile(y.y).y;
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
    let n1 = gradient_noise(vec2f(xi.x / lx, xi.y / ly) + vec2f(3.7, 1.3) + vec2f(0.0, 0.04 * t));
    let n2 = gradient_noise(xi / l2 + vec2f(-8.1, 5.9) + vec2f(0.05 * t, 0.0));
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

// Follows the fluid at p back in time until it is upstream of the eddies.
// Returns where it was (xy) and how long ago (z).
fn trace_back(p: vec2f, t: f32, scn: Scene) -> vec3f {
    let ux = FLOW_SPEED * scn.down.x;
    var y = p;
    var back = 0.0;
    // below the far front the flow is uniform, so that stretch is one exact jump
    // (only drifting, slower in the middle of the wake)
    let over = dot(y, scn.down) - (EDDY_FADE_END + P(P_EDDY_FADE_END) + 0.5);
    if (over > 0.0) {
        let slow = 1.0 - wake_deficit(y, scn);
        back = over / (ux * slow);
        y.x -= FLOW_SPEED * slow * back;
    }
    let dist = dot(y, scn.down) - (EDDY_RISE_START - 3.0);
    if (dist <= 0.0) { return vec3f(y, back); }
    // long enough to get clear of the slow water too
    let span = dist / (ux * (1.0 - wake_deficit(y, scn))) * 1.1;
    // whole steps then a shorter last one, so the result varies smoothly with span; a step
    // count that jumps between neighbouring pixels leaves fixed seams in the pattern
    let h = max(MAX_TRACE_DT, span / f32(MAX_TRACE_STEPS));
    let n = clamp(i32(ceil(span / h)), 1, MAX_TRACE_STEPS);
    let last = span - f32(n - 1) * h;
    for (var i = 0; i < n; i++) {
        let dt = select(h, last, i == n - 1);
        let tt = t - back - f32(i) * h;
        let v1 = flow_velocity(y, tt, scn);
        let v2 = flow_velocity(y - v1 * (0.5 * dt), tt - 0.5 * dt, scn);
        y -= v2 * dt;
    }
    return vec3f(y, back + span);
}

@fragment
fn fs_main(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let cover = pos_to_cover(pos.xy * u.field_scale);
    let scn = scene();
    let rel = cover_to_plane(scn.cam, cover) - scn.centre;
    let tr = trace_back(rel, u.time, scn);
    let src = tr.xy;
    let a = scn.radius;
    // upstream of the eddies the flow is steady, so its stream function labels the lines
    let r2 = max(dot(src, src), 1e-6);
    let psi = src.y * (1.0 - min(a * a / r2, 1.0));
    let rr = length(rel);
    // the stream flares into a pool around the obstacle and widens a little downstream
    let pool = (0.7 + P(P_POOL_FLARE)) * smoothstep(a * 1.8, a * 1.0, rr) * smoothstep(0.9, -0.3, dot(rel, scn.down) / a);
    let hw = 0.8 + P(P_STREAM_HALF_WIDTH) + pool;
    var c = abs(psi) - hw;
    if (r2 < a * a) { c = -hw; }

    // label that rides with the fluid along each band
    let m = src.x - FLOW_SPEED * (u.time - tr.z);
    // on-screen velocity of the liquid here, in reference pixels per unit time
    let v = flow_velocity(rel, u.time, scn);
    let p0 = plane_to_cover(scn.cam, rel + scn.centre);
    let vs = (plane_to_cover(scn.cam, rel + scn.centre + v * 0.01) - p0) * (REFPX / 0.01);
    return vec4f(c, m, vs);
}
