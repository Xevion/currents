// Eddies live between two fronts that run across the picture below the ball (distances along
// the plane direction of screen-down, in periods). Past the far front the pattern only drifts.
const D0 = -2.0;
const D1 = 6.0;
const D2 = 12.0;
const D3 = 26.0;

const DT = 0.25;      // longest trace step, seconds

// smoothstep and its slope
fn ssd(a: f32, b: f32, x: f32) -> vec2f {
    let t = clamp((x - a) / (b - a), 0.0, 1.0);
    return vec2f(t * t * (3.0 - 2.0 * t), 6.0 * t * (1.0 - t) / (b - a));
}

// Envelopes of the eddy stream function (x for the noise, y for the sinuous mode) and their gradients.
struct Env {
    e: vec2f,
    gx: vec2f,
    gy: vec2f,
}

fn envs(y: vec2f, wk: Scene) -> Env {
    let dn = dot(y, wk.g);
    let f1 = ssd(D0, D1 + P(14), dn);
    let f2 = ssd(D2 + P(12), D3 + P(13), dn);
    let front = f1.x * (1.0 - f2.x);
    let dfront = (f1.y * (1.0 - f2.x) - f1.x * f2.y) * wk.g;
    // nothing is stirred toward the left and right of the picture
    let p3 = wk.cam.eu * (y.x + wk.qb.x) + wk.cam.ev * (y.y + wk.qb.y) + wk.cam.n * wk.cam.d;
    let sx = p3.x / p3.z * FOCAL + 0.5 - BALL_C.x;
    let dsx = FOCAL * (vec2f(wk.cam.eu.x, wk.cam.ev.x) * p3.z - p3.x * vec2f(wk.cam.eu.z, wk.cam.ev.z)) / (p3.z * p3.z);
    let ed = select(ssd(0.42, 0.14, sx), ssd(-0.48, -0.16, sx), sx < 0.0);
    let r = max(length(y), 1e-4);
    let nr = ssd(wk.a * 1.05, wk.a * 1.6, r);
    let e = front * ed.x * nr.x;
    let de = (dfront * ed.x + front * ed.y * dsx) * nr.x + front * ed.x * nr.y * y / r;
    // the stream itself stays smooth: only the slow sinuous mode bends it
    let k = 0.12 + P(19);
    let cd = ssd(0.5, 5.0, abs(y.y));
    let damp = mix(k, 1.0, cd.x);
    var o: Env;
    o.e = vec2f(e * damp, e);
    o.gx = de * damp + e * vec2f(0.0, (1.0 - k) * cd.y * sign(y.y));
    o.gy = de;
    return o;
}

// The liquid slows behind the ball, so lines spread apart there. Stream function
// -U D s tanh(y/s): a deficit D sech^2(y/s) across the wake, rising once clear of the ball.
// Returns D and its gradient.
fn wake_d(y: vec2f, wk: Scene) -> vec3f {
    let dm = 0.55 + P(21);
    let ax = ssd((0.7 + P(26)) * wk.a, (3.7 + P(26)) * wk.a, y.x);
    let bn = ssd(-5.0, 0.0, dot(y, wk.g));
    return dm * vec3f(ax.x * bn.x, vec2f(ax.y * bn.x, 0.0) + ax.x * bn.y * wk.g);
}

fn wake_deficit(y: vec2f, wk: Scene) -> f32 {
    let ch = 1.0 / cosh(y.y / (10.0 + P(22)));
    return wake_d(y, wk).x * ch * ch;
}

fn wake_vel(y: vec2f, wk: Scene) -> vec2f {
    let s = 10.0 + P(22);
    let d = wake_d(y, wk);
    let th = tanh(y.y / s);
    let ig = s * th;
    return -FLOW_U * vec2f(d.z * ig + d.x * (1.0 - th * th), -d.y * ig);
}

// Velocity at lab position y (relative to the ball): potential flow round the disc plus the
// curl of an eddy stream function. Both parts are divergence free, so areas are preserved.
fn vel(y: vec2f, t: f32, wk: Scene) -> vec2f {
    let r2 = max(dot(y, y), 1e-4);
    let q = min(wk.a * wk.a / r2, 1.0);
    var v = FLOW_U * vec2f(1.0 - q + 2.0 * q * y.y * y.y / r2, -2.0 * q * y.x * y.y / r2);
    v += wake_vel(y, wk);

    let xi = y - vec2f(FLOW_U * t, 0.0);
    let lx = 9.0 + P(8);
    let ly = 12.0 + P(9);
    let l2 = 5.5 + P(10);
    let a1 = 1.4 + P(4);
    let a2 = 0.4 + P(5);
    let n1 = noised(vec2f(xi.x / lx, xi.y / ly) + vec2f(3.7, 1.3) + vec2f(0.0, 0.04 * t));
    let n2 = noised(xi / l2 + vec2f(-8.1, 5.9) + vec2f(0.05 * t, 0.0));
    let nv = a1 * lx * n1.x + a2 * l2 * n2.x;
    let ng = vec2f(a1 * n1.y + a2 * n2.y, a1 * lx / ly * n1.z + a2 * n2.z);

    let km = 6.2831853 / 26.0;
    let hm = 9.0;
    let am = 0.5 + P(6);
    let gy = exp(-(y.y * y.y) / (hm * hm));
    let sv = am / km * sin(km * xi.x) * gy;
    let sg = vec2f(am * cos(km * xi.x) * gy, sv * (-2.0 * y.y / (hm * hm)));

    let en = envs(y, wk);
    let grad = en.e.x * ng + nv * en.gx + en.e.y * sg + sv * en.gy;
    return v + vec2f(grad.y, -grad.x);
}

// Follows the fluid at p back in time until it is upstream of the eddies.
// Returns where it was (xy) and how long ago (z).
fn trace(p: vec2f, t: f32, wk: Scene) -> vec3f {
    let ux = FLOW_U * wk.g.x;
    var y = p;
    var back = 0.0;
    // below the far front the flow is uniform, so that stretch is one exact jump
    // (only drifting, slower in the middle of the wake)
    let over = dot(y, wk.g) - (D3 + P(13) + 0.5);
    if (over > 0.0) {
        let slow = 1.0 - wake_deficit(y, wk);
        back = over / (ux * slow);
        y.x -= FLOW_U * slow * back;
    }
    let dist = dot(y, wk.g) - (D0 - 3.0);
    if (dist <= 0.0) { return vec3f(y, back); }
    // long enough to get clear of the slow water too
    let span = dist / (ux * (1.0 - wake_deficit(y, wk))) * 1.1;
    // whole steps then a shorter last one, so the result varies smoothly with span; a step
    // count that jumps between neighbouring pixels leaves fixed seams in the pattern
    let h = max(DT, span / f32(STEPS));
    let n = clamp(i32(ceil(span / h)), 1, STEPS);
    let last = span - f32(n - 1) * h;
    for (var i = 0; i < n; i++) {
        let dt = select(h, last, i == n - 1);
        let tt = t - back - f32(i) * h;
        let v1 = vel(y, tt, wk);
        let v2 = vel(y - v1 * (0.5 * dt), tt - 0.5 * dt, wk);
        y -= v2 * dt;
    }
    return vec3f(y, back + span);
}

@fragment
fn fs_main(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let cover = cover_of(pos.xy * u.fscale);
    let sc = scene();
    let rel = plane_q(sc.cam, cover) - sc.qb;
    let tr = trace(rel, u.time, sc);
    let src = tr.xy;
    let a = sc.a;
    // upstream of the eddies the flow is steady, so its stream function labels the lines
    let r2 = max(dot(src, src), 1e-6);
    let psi = src.y * (1.0 - min(a * a / r2, 1.0));
    let rr = length(rel);
    let s_down = rel.x;
    // the stream flares into a pool around the obstacle and widens a little downstream
    let pool = (0.7 + P(23)) * smoothstep(a * 1.8, a * 1.0, rr) * smoothstep(0.9, -0.3, dot(rel, sc.g) / a);
    let hw = 0.8 + P(24) + pool;
    var c = abs(psi) - hw;
    if (r2 < a * a) { c = -hw; }

    // label that rides with the fluid along each band
    let m = src.x - FLOW_U * (u.time - tr.z);
    // on-screen velocity of the liquid here, in reference pixels per unit time
    let v = vel(rel, u.time, sc);
    let p0 = project(sc.cam, rel + sc.qb);
    let vs = (project(sc.cam, rel + sc.qb + v * 0.01) - p0) * (REFPX / 0.01);
    return vec4f(c, m, vs);
}
