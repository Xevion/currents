// Blends each frame into the ones before it. Frames shade points a fraction of a pixel apart,
// so the blend is the picture sampled several times a pixel: detail finer than a pixel
// settles to its average instead of flickering.
@group(0) @binding(3) var shaded: texture_2d<f32>;
@group(0) @binding(4) var before: texture_2d<f32>;
@group(0) @binding(5) var smp: sampler;
@group(0) @binding(6) var field: texture_2d<f32>;

// Share of each new frame in the blend. Small, so that the shift of each frame's shading
// points averages out instead of showing as a shake.
const FRESH = 0.1;
// How far the picture shown is pushed away from the average of its neighbours, which undoes
// the softening the blend picks up from being read between pixels as it follows the flow.
const SHARPEN = 0.8;

fn to_ycocg(c: vec3f) -> vec3f {
    return vec3f(0.25 * c.r + 0.5 * c.g + 0.25 * c.b, 0.5 * c.r - 0.5 * c.b, -0.25 * c.r + 0.5 * c.g - 0.25 * c.b);
}

fn from_ycocg(c: vec3f) -> vec3f {
    return vec3f(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

// The blend so far at p (texels), through a cubic so that reading it between texels while
// the view moves does not soften it. Five filtered taps; the four corners carry little
// weight and are left out.
fn before_at(p: vec2f) -> vec3f {
    let size = vec2f(textureDimensions(before));
    let c = floor(p - 0.5) + 0.5;
    let f = p - c;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let t0 = (c - 1.0) / size;
    let t3 = (c + 2.0) / size;
    let t12 = (c + w2 / w12) / size;
    let sum = textureSampleLevel(before, smp, vec2f(t12.x, t0.y), 0.0).rgb * (w12.x * w0.y)
        + textureSampleLevel(before, smp, vec2f(t0.x, t12.y), 0.0).rgb * (w0.x * w12.y)
        + textureSampleLevel(before, smp, t12, 0.0).rgb * (w12.x * w12.y)
        + textureSampleLevel(before, smp, vec2f(t3.x, t12.y), 0.0).rgb * (w3.x * w12.y)
        + textureSampleLevel(before, smp, vec2f(t12.x, t3.y), 0.0).rgb * (w12.x * w3.y);
    let weight = w12.x * w0.y + w0.x * w12.y + w12.x * w12.y + w3.x * w12.y + w12.x * w3.y;
    return max(sum / weight, vec3f(0.0));
}

@fragment
fn fs_main(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let at = vec2i(pos.xy);
    let last = vec2i(u.resolution) - 1;
    let frame = textureLoad(shaded, at, 0);
    let now = frame.rgb;
    var colour = now;
    let cover = pos_to_cover(pos.xy);
    if (u.history > 0.5) {
        // where what this pixel shows was on screen a frame ago. The ball (the frame's alpha)
        // moves only with the view; liquid lapping over it has also flowed, by the velocity
        // the field pass wrote
        let flow = textureLoad(field, min(vec2i(pos.xy / u.field_scale), vec2i(textureDimensions(field)) - 1), 0).ba;
        let moved = flow * ((1.0 - frame.a) * u.elapsed / REFPX);
        let was = (cover - moved - u.prev_view.xy) / (u.prev_view.zw - u.prev_view.xy) * u.resolution;
        if (all(was > vec2f(1.0)) && all(was < u.resolution - 1.0)) {
            // what this pixel showed before is trusted only as far as its neighbours now
            // could show it, which drops whatever the motion did not carry here
            var lo = to_ycocg(now);
            var hi = lo;
            for (var j = -1; j <= 1; j++) {
                for (var i = -1; i <= 1; i++) {
                    let near = to_ycocg(textureLoad(shaded, clamp(at + vec2i(i, j), vec2i(0), last), 0).rgb);
                    lo = min(lo, near);
                    hi = max(hi, near);
                }
            }
            let kept = from_ycocg(clamp(to_ycocg(before_at(was)), lo, hi));
            // the first few frames count equally, so the blend settles without a lurch
            colour = mix(kept, now, max(FRESH, 1.0 / (u.history + 1.0)));
        }
    }
    return vec4f(colour, 1.0);
}

// The picture shown: the blend, sharpened, and held within what its neighbours show so that
// no edge gains a halo.
@fragment
fn fs_show(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let at = vec2i(pos.xy);
    let last = vec2i(u.resolution) - 1;
    let c = textureLoad(before, at, 0).rgb;
    let n = textureLoad(before, max(at - vec2i(0, 1), vec2i(0)), 0).rgb;
    let s = textureLoad(before, min(at + vec2i(0, 1), last), 0).rgb;
    let w = textureLoad(before, max(at - vec2i(1, 0), vec2i(0)), 0).rgb;
    let e = textureLoad(before, min(at + vec2i(1, 0), last), 0).rgb;
    let lo = min(c, min(min(n, s), min(w, e)));
    let hi = max(c, max(max(n, s), max(w, e)));
    return vec4f(clamp(c + SHARPEN * (c - 0.25 * (n + s + w + e)), lo, hi), 1.0);
}
