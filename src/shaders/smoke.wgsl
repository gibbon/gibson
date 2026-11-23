// Low-lying smoke, shared by the floor and tower shaders (prepended to both).
// A layer of haze that's thickest on the ground and thins with height, broken
// into drifting wisps by noise. It lives in the world, not on the screen: you
// fly through it, and the towers' bases stand in it.

const SMOKE_DENSITY: f32 = 0.035;  // per unit of distance, at ground level
const SMOKE_MAX: f32 = 0.8;       // never hides things completely

fn smoke_hash(p: vec3<f32>) -> f32 {
    let q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    let r = q + dot(q, q.yxz + 33.33);
    return fract((r.x + r.y) * r.z);
}

fn smoke_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(smoke_hash(i), smoke_hash(i + vec3(1.0, 0.0, 0.0)), u.x),
                mix(smoke_hash(i + vec3(0.0, 1.0, 0.0)), smoke_hash(i + vec3(1.0, 1.0, 0.0)), u.x), u.y);
    let b = mix(mix(smoke_hash(i + vec3(0.0, 0.0, 1.0)), smoke_hash(i + vec3(1.0, 0.0, 1.0)), u.x),
                mix(smoke_hash(i + vec3(0.0, 1.0, 1.0)), smoke_hash(i + vec3(1.0, 1.0, 1.0)), u.x), u.y);
    return mix(a, b, u.z);
}

// Wisps: a few octaves of noise, drifting along (a couple of units a second)
// and rolling over as they go.
fn smoke_wisps(p: vec3<f32>, t: f32) -> f32 {
    var q = vec3<f32>(p.x * 0.04 + t * 0.07, p.y * 0.08 - t * 0.03, p.z * 0.04 - t * 0.045);
    var v = 0.0;
    var amp = 0.55;
    for (var k = 0; k < 3; k++) {
        v += smoke_noise(q + vec3<f32>(0.0, 0.0, t * 0.09 * f32(k + 1))) * amp;
        q = q * 2.07 + vec3<f32>(17.1, 3.7, 9.3);
        amp *= 0.5;
    }
    return v; // about 0..1, mostly 0.3..0.7
}

// How much smoke lies between the camera and a point (x: 0 none .. SMOKE_MAX),
// and how thick its wisps are there (y: 0..1, for its colour).
// `amount` scales it (the LOOK panel's SMOKE: 1 = default, 0 = none); `height`
// is how high it rises (SMOKE HEIGHT, 0..1).
fn smoke(world: vec3<f32>, cam: vec3<f32>, t: f32, amount: f32, height: f32) -> vec2<f32> {
    if (amount <= 0.0) {
        return vec2<f32>(0.0);
    }
    // It thins by e every this many units up: low on the floor, or up the towers.
    let SMOKE_HEIGHT = 4.0 + height * 40.0;
    let d = world - cam;
    let dist = length(d);
    // Exponential height fog, integrated exactly along the ray.
    let y0 = max(cam.y, 0.0);
    let y1 = max(world.y, 0.0);
    let dy = y1 - y0;
    var column: f32;
    if (abs(dy) < 0.01) {
        column = exp(-y0 / SMOKE_HEIGHT);
    } else {
        column = SMOKE_HEIGHT * (exp(-y0 / SMOKE_HEIGHT) - exp(-y1 / SMOKE_HEIGHT)) / dy;
    }
    // Wisps, sampled near the point and part-way along the ray (for parallax).
    let w = smoke_wisps(world, t) * 0.6 + smoke_wisps(mix(cam, world, 0.45), t) * 0.4;
    // A steady haze everywhere, thicker in the wisps (so no big bare patches).
    let thick = 0.4 + smoothstep(0.28, 0.72, w) * 2.0;
    let optical = SMOKE_DENSITY * amount * column * dist * thick;
    return vec2<f32>((1.0 - exp(-optical)) * SMOKE_MAX, smoothstep(0.3, 0.75, w));
}

fn smoke_hue(h: f32) -> vec3<f32> {
    let k = vec3<f32>(0.0, 2.0 / 3.0, 1.0 / 3.0);
    let c = clamp(abs(fract(h + k) * 6.0 - 3.0) - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
    return mix(vec3<f32>(1.0), c, 0.8); // a little short of fully saturated
}

// The smoke's own colour: near black where it's thin; where it's thick it
// catches the city's glow (the theme's haze and floor light), so the wisps show.
// `hue`: 0 = grey (with the theme's tint), else a colour round the wheel (0..1).
fn smoke_colour(haze: vec3<f32>, glow: vec3<f32>, wisp: f32, hue: f32) -> vec3<f32> {
    // (linear light, so these small numbers are dim on screen)
    var dark = vec3<f32>(0.004, 0.0045, 0.006);
    var lit = vec3<f32>(0.03, 0.032, 0.042) + haze * 0.05 + glow * 0.07;
    if (hue > 0.0) {
        let c = smoke_hue(hue);
        dark = c * 0.005;
        lit = c * 0.075 + glow * 0.02;
    }
    return mix(dark, lit, wisp);
}

// How much of the smoke shows: its amount times the OPACITY slider (1 = default),
// up to fully opaque.
fn smoke_opacity(amount: f32, opacity: f32) -> f32 {
    return min(amount * opacity, 1.0);
}
