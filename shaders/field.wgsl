// Perturbation velocity at lab position y (relative to the ball) and time t.
fn pert(y: vec2f, t: f32, wk: Scene) -> vec2f {
    let xi = y - vec2f(FLOW_U * t, 0.0);
    let lx = 4.5 + P(8);
    let ly = 26.0 + P(9);
    let l2 = 11.0 + P(10);
    let n1 = noised(vec2f(xi.x / lx, xi.y / ly) + vec2f(3.7, 1.3) + vec2f(0.0, 0.013 * t));
    let n2 = noised(xi / l2 + vec2f(-8.1, 5.9) + vec2f(0.021 * t, 0.0));
    let a1 = 1.7 + P(4);
    let a2 = 0.6 + P(5);
    // curl of the potential
    var w = a1 * vec2f(n1.z * lx / ly, -n1.y) + a2 * vec2f(n2.z, -n2.y);
    // irrotational part: pushes that bunch lines up in places and fatten them elsewhere
    let l3 = 12.0 + P(20);
    let n3 = noised(xi / l3 + vec2f(11.3, -4.2) + vec2f(0.009 * t, 0.017 * t));
    w += (0.3 + P(15)) * vec2f(n3.y, n3.z) + P(19) * vec2f(n2.y, n2.z);
    // sinuous shedding along the centreline
    let km = 6.2831853 / 26.0;
    let hm = 9.0;
    let gy = exp(-(y.y * y.y) / (hm * hm));
    let am = 0.8 + P(6);
    // potential am/km * sin(km xi.x) * gy
    w += am * vec2f(sin(km * xi.x) * gy * (-2.0 * y.y / (hm * hm)) / km, -cos(km * xi.x) * gy);
    let dn = dot(y, wk.g);
    let env = smoothstep(-2.0, 9.0 + P(14), dn) * (1.0 + (0.7 + P(21)) * smoothstep(8.0, 40.0, dn)) * smoothstep(wk.a * 1.0, wk.a * 1.35, length(y));
    let rr = length(y);
    let om = (0.085 + P(12)) / (1.0 + rr / 70.0);
    w += om * vec2f(-y.y, y.x) * smoothstep(-4.0, 10.0, -y.y);
    return w * env;
}

fn wake_disp(p: vec2f, t: f32, wk: Scene) -> vec2f {
    let tau = TAU_MAX + P(7);
    let dt = tau / f32(STEPS);
    var y = p;
    var disp = vec2f(0.0);
    for (var i = 0; i < STEPS; i++) {
        let tt = t - (f32(i) + 0.5) * dt;
        let w1 = pert(y, tt + 0.5 * dt, wk);
        let ym = y - (vec2f(FLOW_U, 0.0) + w1) * (0.5 * dt);
        let w = pert(ym, tt, wk);
        y -= (vec2f(FLOW_U, 0.0) + w) * dt;
        disp -= w * dt;
    }
    return disp;
}

@fragment
fn fs_main(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let cover = cover_of(pos.xy);
    let opx = u.resolution.x / (u.view.z - u.view.x);
    let sc = scene();
    let cam = sc.cam;
    let r = ray(cover);
    let hit = r * (cam.d / dot(cam.n, r));
    let q = vec2f(dot(hit, cam.eu), dot(hit, cam.ev));

    let rel = q - sc.qb;
    let disp = wake_disp(rel, u.time, sc);
    let pw = rel + disp;
    let r2 = max(dot(pw, pw), 1e-6);
    let a = sc.a;
    let psi = pw.y * (1.0 - a * a / r2);
    let inside = r2 < a * a;

    let s_down = rel.x;
    let hw = 0.8 + 0.9 * smoothstep(a * 1.0, a * 3.5, s_down);
    // fat core: bands widen inside the wake and bunch up at its fringe
    let dnl = dot(rel, sc.g);
    let core_w = mix(26.0, 50.0 + P(13), smoothstep(2.0, -6.0, psi));
    let core_k = (0.15 + P(11)) * smoothstep(2.0, 30.0, dnl);
    let apsi = abs(psi);
    var c = apsi - core_k * apsi * (1.0 - smoothstep(core_w, core_w * 1.9, apsi)) - hw;
    if (inside) { c = -hw; }

    // sphere coverage
    var ball = 0.0;
    let rd = normalize(r);
    let b = dot(rd, sc.bc);
    let disc = b * b - dot(sc.bc, sc.bc) + sc.br * sc.br;
    if (disc > 0.0 && b - sqrt(disc) < length(hit)) {
        ball = clamp(sqrt(disc) * opx / sc.tb * FOCAL * 0.5, 0.0, 1.0);
    }
    return vec4f(c, s_down / a, ball, length(rel) / a);
}
