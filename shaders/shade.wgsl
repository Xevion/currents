@group(0) @binding(1) var field: texture_2d<f32>;

fn fl(p: vec2i) -> vec4f {
    let dim = vec2i(textureDimensions(field));
    return textureLoad(field, clamp(p, vec2i(0), dim - 1), 0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let vsize = u.view.zw - u.view.xy;
    let cover = cover_of(pos.xy);
    let opx = u.resolution.x / vsize.x;   // output pixels per cover unit
    let sc0 = scene();
    let a = 1.0;
    let ip = vec2i(pos.xy);
    let fv = fl(ip);
    let c = fv.r;
    let s_down = fv.g;
    let rr = fv.a;
    let inside = false;
    let gs = vec2f(fl(ip + vec2i(1, 0)).r - fl(ip - vec2i(1, 0)).r,
                   fl(ip + vec2i(0, 1)).r - fl(ip - vec2i(0, 1)).r) * 0.5 * opx / REFPX;
    let gl = max(length(gs), 1e-5);
    let gh = gs / gl;

    // grain, anchored to the reference pixel grid
    let gp = cover * REFPX;
    let fine = hash12(floor(gp * 1.0) + 17.0);
    let mid = vnoise(gp / 1.7 + 31.0);
    let clump = vnoise(gp / 7.0 + 5.0);
    let grain = 0.55 * fine + 0.45 * mid;

    let black = srgb(12.0, 10.0, 11.0);
    var col = black;

    if (c < 0.0 || inside) {
        // flat coloured stream
        let red = srgb(232.0, 38.0, 56.0);
        let pink = srgb(240.0, 84.0, 140.0);
        let amber = srgb(236.0, 166.0, 46.0);
        let orange = srgb(240.0, 100.0, 40.0);
        var sc = mix(red, pink, smoothstep(a * 4.5, a * 1.3, rr) * (1.0 - smoothstep(a * 1.2, a * 2.4, s_down)));
        sc = mix(sc, orange, smoothstep(a * 1.9, a * 2.7, s_down));
        sc = mix(sc, amber, smoothstep(a * 2.5, a * 3.6, s_down));
        let edge = select(-c / gl, 1e3, inside);
        let cov = clamp(edge * opx / REFPX + 0.5, 0.0, 1.0);
        col = mix(black, sc * (0.93 + 0.14 * grain), cov);
    } else {
        let duty = DUTY;
        let f = fract(c);
        if (f >= 1.0 - duty) {
            let t = (f - (1.0 - duty)) / duty;       // 0..1 across the band
            let dpx = min(f - (1.0 - duty), 1.0 - f) / gl;  // distance to nearest edge, ref px
            let wpx = duty / gl;                     // band width, ref px
            let x = 2.0 * t - 1.0;
            // tube-like cross-section, flatter for thin bands
            let infl = smoothstep(5.0, 30.0, wpx);
            let k = mix(0.7, 0.98, infl) + P(16);
            let xs = sign(x) * pow(abs(x), 0.9 + P(17));
            let nxy = gh * xs * k;
            let n = vec3f(nxy, sqrt(max(1.0 - dot(nxy, nxy), 0.0)));
            let l = normalize(vec3f(-0.52, -0.66, 0.55));
            let hv = normalize(l + vec3f(0.0, 0.0, 1.0));
            let dif = dot(n, l);
            let spec = pow(max(dot(n, hv), 0.0), 26.0);

            let body = srgb(152.0, 112.0, 184.0);
            let pale = srgb(238.0, 216.0, 242.0);
            let dark = srgb(42.0, 24.0, 84.0);

            // stippled shadow: threshold the shade against clumpy grain
            let shade = smoothstep(-0.4, 0.5, dif);
            let nz = (0.3 * fine + 0.4 * mid + 0.3 * clump - 0.5) / 0.16;
            let zt = mix(1.7, -1.45, shade);
            let clean = smoothstep(zt - 0.35, zt + 0.35, nz);
            var bcol = mix(mix(dark, srgb(84.0, 54.0, 128.0), shade * shade), body, clean);
            bcol = mix(bcol, black, smoothstep(0.4, 0.0, shade) * 0.85 * infl);

            let hl = max(smoothstep(0.44 + P(18), 0.86, dif), spec * 1.2) * mix(0.55, 1.0, infl);
            let rw = 0.8 + 0.03 * wpx;
            let rim = (1.0 - smoothstep(rw * 0.5, rw * 1.6, dpx)) * mix(0.9, 0.7, smoothstep(0.0, 0.6, -dif));
            bcol = mix(bcol, pale, clamp(max(hl, rim), 0.0, 1.0));
            let cov = clamp(dpx * opx / REFPX + 0.5, 0.0, 1.0);
            col = mix(black, bcol * (0.92 + 0.16 * grain), cov);
        }
    }
    col += (grain - 0.5) * 0.035;

    // far fade toward the upper right: highlights go first, then a blotchy dissolve
    let fs = 0.45 * cover.x - 0.89 * cover.y - 0.22 * (cover.x - 0.6) * (cover.x - 0.6);
    let fd = 1.0 - smoothstep(-0.14, 0.4, fs);
    let lum = dot(col, vec3f(0.3, 0.6, 0.1));
    let violet = srgb(70.0, 44.0, 120.0) * lum * 1.6;
    col = mix(violet, col, smoothstep(0.35, 0.85, fd));
    let blot = 0.6 * clump + 0.4 * mid;
    col = mix(black, col, smoothstep(blot * 0.5 - 0.1, blot * 0.5 + 0.12, fd));

    // sphere
    if (fv.b > 0.0) {
        let r = ray(cover);
        let rd = normalize(r);
        let b = dot(rd, sc0.bc);
        let disc = max(b * b - dot(sc0.bc, sc0.bc) + sc0.br * sc0.br, 0.0);
        let ts = b - sqrt(disc);
        let n = normalize(rd * ts - sc0.bc);
        let d2 = n.xy;
        let rd2 = length(d2);
        var g = 0.36 - 0.2 * (0.62 * d2.x + 0.55 * d2.y);
        g *= 1.0 - 0.5 * smoothstep(0.5, 1.0, rd2) * smoothstep(-0.9, 0.5, d2.x + d2.y);
        g += 0.2 * smoothstep(0.8, 0.99, rd2) * smoothstep(0.3, -0.5, d2.x) * smoothstep(-0.7, 0.2, d2.y);
        g += 0.07 * exp(-dot(d2 - vec2f(-0.3, 0.62), d2 - vec2f(-0.3, 0.62)) / 0.06);
        let q1 = d2 - vec2f(-0.37, -0.27);
        let q2 = d2 - vec2f(0.07, -0.44);
        g += 0.7 * exp(-dot(q1, q1) / (2.0 * 0.17 * 0.17));
        g += 0.85 * exp(-dot(q2, q2) / (2.0 * 0.075 * 0.075));
        g = clamp(g, 0.0, 0.97);
        let bg = 0.6 * fine + 0.4 * mid;
        g = g * (0.8 + 0.4 * bg) - 0.03 * (1.0 - bg);
        let scol = clamp(vec3f(g, g, g * 1.015), vec3f(0.02), vec3f(0.98));
        col = mix(col, scol, fv.b);
    }

    return vec4f(col, 1.0);
}
