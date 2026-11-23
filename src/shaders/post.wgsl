// Bloom (bright pass → mip-chain down → tent up) and the final composite:
// tonemap, a touch of film grain and vignette.

struct Post {
    texel: vec4<f32>,   // source texel size xy; z = threshold / strength; w = time
    fx: vec4<f32>,      // x = alarm: the garbage file's red glitch (0 = calm)
};
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> pp: Post;
@group(0) @binding(3) var bloom: texture_2d<f32>;   // composite only (dummy elsewhere)

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var o: VOut;
    o.clip = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, y);
    return o;
}

// 13-tap downsample (CoD: AW style), keeps bloom stable when things move.
fn down13(uv: vec2<f32>) -> vec3<f32> {
    let t = pp.texel.xy;
    let a = textureSample(src, samp, uv + t * vec2(-2.0, -2.0)).rgb;
    let b = textureSample(src, samp, uv + t * vec2( 0.0, -2.0)).rgb;
    let c = textureSample(src, samp, uv + t * vec2( 2.0, -2.0)).rgb;
    let d = textureSample(src, samp, uv + t * vec2(-2.0,  0.0)).rgb;
    let e = textureSample(src, samp, uv).rgb;
    let f = textureSample(src, samp, uv + t * vec2( 2.0,  0.0)).rgb;
    let gg = textureSample(src, samp, uv + t * vec2(-2.0,  2.0)).rgb;
    let h = textureSample(src, samp, uv + t * vec2( 0.0,  2.0)).rgb;
    let i = textureSample(src, samp, uv + t * vec2( 2.0,  2.0)).rgb;
    let j = textureSample(src, samp, uv + t * vec2(-1.0, -1.0)).rgb;
    let k = textureSample(src, samp, uv + t * vec2( 1.0, -1.0)).rgb;
    let l = textureSample(src, samp, uv + t * vec2(-1.0,  1.0)).rgb;
    let m = textureSample(src, samp, uv + t * vec2( 1.0,  1.0)).rgb;
    return e * 0.125 + (a + c + gg + i) * 0.03125 + (b + d + f + h) * 0.0625 + (j + k + l + m) * 0.125;
}

@fragment
fn fs_bright(i: VOut) -> @location(0) vec4<f32> {
    let c = down13(i.uv);
    let lum = max(c.r, max(c.g, c.b));
    let knee = 0.5;
    let th = pp.texel.z;
    var soft = clamp(lum - th + knee, 0.0, 2.0 * knee);
    soft = soft * soft / (4.0 * knee + 1e-4);
    let w = max(soft, lum - th) / max(lum, 1e-4);
    return vec4<f32>(c * w, 1.0);
}

@fragment
fn fs_down(i: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(down13(i.uv), 1.0);
}

// 3x3 tent upsample, added onto the next-larger mip (blend: one + one).
@fragment
fn fs_up(i: VOut) -> @location(0) vec4<f32> {
    let t = pp.texel.xy;
    var s = textureSample(src, samp, i.uv).rgb * 4.0;
    s += (textureSample(src, samp, i.uv + vec2(-t.x, 0.0)).rgb + textureSample(src, samp, i.uv + vec2(t.x, 0.0)).rgb
        + textureSample(src, samp, i.uv + vec2(0.0, -t.y)).rgb + textureSample(src, samp, i.uv + vec2(0.0, t.y)).rgb) * 2.0;
    s += textureSample(src, samp, i.uv + vec2(-t.x, -t.y)).rgb + textureSample(src, samp, i.uv + vec2(t.x, -t.y)).rgb
       + textureSample(src, samp, i.uv + vec2(-t.x, t.y)).rgb + textureSample(src, samp, i.uv + vec2(t.x, t.y)).rgb;
    return vec4<f32>(s / 16.0, 1.0);
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3(0.0), vec3(1.0));
}

@fragment
fn fs_composite(i: VOut) -> @location(0) vec4<f32> {
    let a = pp.fx.x;
    let t = pp.texel.w;
    let tick = floor(t * 14.0);
    let jolt = fract(sin(tick * 3.71 + 0.3) * 9631.13);
    var uv = i.uv;
    if (a > 0.001) {
        // Torn bands sliding sideways, and now and then the whole frame jumps.
        let band = floor(uv.y * 46.0);
        let r = fract(sin(band * 12.9898 + tick * 78.233) * 43758.5453);
        uv.x += step(1.0 - 0.3 * a, r) * (r - 0.5) * 0.09 * a;
        uv.y += step(0.9, jolt) * (jolt - 0.95) * 0.25 * a;
    }
    // Colour fringes pull apart with the alarm.
    let split = vec2<f32>(0.007 * a, 0.0);
    let scene = vec3<f32>(
        textureSample(src, samp, uv + split).r,
        textureSample(src, samp, uv).g,
        textureSample(src, samp, uv - split).b,
    );
    let glow = textureSample(bloom, samp, uv).rgb;
    var c = scene + glow * pp.texel.z;
    c = aces(c * 0.8);
    if (a > 0.001) {
        let lum = dot(c, vec3(0.3, 0.55, 0.15));
        c = mix(c, vec3(lum * 1.5, lum * 0.22, lum * 0.18), 0.5 * a);
        c *= 1.0 - 0.18 * a * (0.5 + 0.5 * sin(i.clip.y * 2.2 + t * 40.0));
        c += vec3(0.3, 0.0, 0.02) * a * step(0.96, jolt);
    }
    // Seedier: a touch darker and less saturated, blacks crushed, a cold green
    // cast in the shadows.
    let l = dot(c, vec3(0.3, 0.55, 0.15));
    c = mix(vec3(l), c, 0.85) * 0.88;
    // (linear light: tiny numbers here are big ones on screen)
    c = max(c - vec3(0.003), vec3(0.0)) * 1.003;
    c += vec3(0.0, 0.0012, 0.001) * (1.0 - smoothstep(0.0, 0.1, l));
    // A heavy vignette: the edges fall away into the dark.
    let v = i.uv - 0.5;
    let r = length(v) * 1.414; // 0 centre, 1 corner
    let vig = smoothstep(0.12, 0.95, r);
    c *= mix(1.0, 0.04, vig * vig * (3.0 - 2.0 * vig));
    // Film grain, stronger in the dark.
    let n = fract(sin(dot(i.clip.xy + pp.texel.w * 61.0, vec2(12.9898, 78.233))) * 43758.5453);
    c += (n - 0.5) * mix(0.012, 0.03, smoothstep(0.0, 0.4, l));
    return vec4<f32>(max(c, vec3(0.0)), 1.0);
}
