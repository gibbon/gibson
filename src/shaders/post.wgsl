// Bloom (bright pass → mip-chain down → tent up) and the final composite:
// tonemap, a touch of film grain and vignette.

struct Post {
    texel: vec4<f32>,   // source texel size xy; z = threshold / strength; w = time
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
    let scene = textureSample(src, samp, i.uv).rgb;
    let glow = textureSample(bloom, samp, i.uv).rgb;
    var c = scene + glow * pp.texel.z;
    c = aces(c * 0.9);
    // Vignette and a little film grain.
    let v = i.uv - 0.5;
    c *= 1.0 - dot(v, v) * 0.55;
    let n = fract(sin(dot(i.clip.xy + pp.texel.w * 61.0, vec2(12.9898, 78.233))) * 43758.5453);
    c += (n - 0.5) * 0.025;
    return vec4<f32>(max(c, vec3(0.0)), 1.0);
}
