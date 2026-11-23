// Bloom (bright pass → mip-chain down → tent up) and the final composite:
// tonemap, a touch of film grain and vignette.

struct Post {
    texel: vec4<f32>,   // source texel size xy; z = threshold / strength; w = time
    fx: vec4<f32>,      // x = alarm: the garbage file's red glitch (0 = calm);
                        // the LOOK panel: y = vignette (1 = default), z = darkness 0..1,
                        // w = grain (1 = default)
    look: vec4<f32>,    // LOOK panel: x = CRT 0..1, y = glitch 0..1, z = softness 0..1
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

fn hash1(x: f32) -> f32 {
    return fract(sin(x * 12.9898 + 4.1) * 43758.5453);
}

@fragment
fn fs_composite(i: VOut) -> @location(0) vec4<f32> {
    let t = pp.texel.w;
    let crt = pp.look.x;
    let gl = pp.look.y;
    // GLITCH: now and then a short burst (more often, and harder, as it goes up).
    let slot = floor(t * 3.0);
    let burst_on = step(1.0 - 0.22 * gl, hash1(slot)) * step(fract(t * 3.0), 0.35 + 0.4 * hash1(slot + 7.0));
    let burst = burst_on * gl * (0.35 + 0.65 * hash1(slot + 3.0));
    // The alarm (garbage file) and the bursts share the tearing.
    let a = pp.fx.x;
    let tear = max(a, burst * 0.8);
    let tick = floor(t * 14.0);
    let jolt = fract(sin(tick * 3.71 + 0.3) * 9631.13);
    var uv = i.uv;
    // CRT: the picture bulges a little, like old curved glass.
    let cc = uv - 0.5;
    uv = 0.5 + cc * (1.0 + dot(cc, cc) * 0.22 * crt) / (1.0 + 0.055 * crt);
    let outside = any(uv < vec2(0.0)) || any(uv > vec2(1.0));
    if (tear > 0.001) {
        // Torn bands sliding sideways, and now and then the whole frame jumps.
        let band = floor(uv.y * 46.0);
        let r = fract(sin(band * 12.9898 + tick * 78.233) * 43758.5453);
        uv.x += step(1.0 - 0.3 * tear, r) * (r - 0.5) * 0.09 * tear;
        uv.y += step(0.9, jolt) * (jolt - 0.95) * 0.25 * a;
        // Blocks of the picture shifted out of place (bursts only).
        let blk = floor(uv * vec2(9.0, 16.0));
        let br = hash1(blk.x * 7.0 + blk.y * 31.0 + tick);
        uv.x += step(1.0 - 0.12 * burst, br) * (hash1(br) - 0.5) * 0.12 * burst;
    }
    // Colour fringes: a little always with CRT, apart with the alarm and bursts.
    let split = vec2<f32>(0.007 * a + 0.0012 * crt + 0.012 * burst, 0.0);
    var scene = vec3<f32>(
        textureSample(src, samp, uv + split).r,
        textureSample(src, samp, uv).g,
        textureSample(src, samp, uv - split).b,
    );
    // SOFTNESS: a slight blur (takes the hard, stepped edges off) and more glow
    // spilling around bright things, like a mist filter on a lens.
    let soft = pp.look.z;
    let o = pp.texel.xy * (0.75 + 1.25 * soft);
    let blur = (textureSample(src, samp, uv + vec2(o.x, o.y)).rgb
              + textureSample(src, samp, uv + vec2(-o.x, o.y)).rgb
              + textureSample(src, samp, uv + vec2(o.x, -o.y)).rgb
              + textureSample(src, samp, uv + vec2(-o.x, -o.y)).rgb) * 0.25;
    scene = mix(scene, blur, clamp(soft * 1.4, 0.0, 0.85));
    let glow = textureSample(bloom, samp, uv).rgb;
    var c = scene + glow * pp.texel.z * (1.0 + 1.6 * soft);
    c = aces(c * 0.8);
    // GLITCH bursts also flash a band brighter, and drop some lines to black.
    if (burst > 0.001) {
        let line = floor(i.uv.y * 120.0);
        c *= 1.0 - step(0.93, hash1(line + tick * 13.0)) * burst;
        c += vec3(0.0, 0.25, 0.3) * step(0.985, hash1(floor(i.uv.y * 30.0) + tick)) * burst;
    }
    if (a > 0.001) {
        let lum = dot(c, vec3(0.3, 0.55, 0.15));
        c = mix(c, vec3(lum * 1.5, lum * 0.22, lum * 0.18), 0.5 * a);
        c *= 1.0 - 0.18 * a * (0.5 + 0.5 * sin(i.clip.y * 2.2 + t * 40.0));
        c += vec3(0.3, 0.0, 0.02) * a * step(0.96, jolt);
    }
    // Seedier: a touch darker and less saturated, blacks crushed, a cold green
    // cast in the shadows.
    let d = pp.fx.z;
    let l = dot(c, vec3(0.3, 0.55, 0.15));
    c = mix(vec3(l), c, 1.0 - 0.25 * d) * (1.0 - 0.2 * d);
    // (linear light: tiny numbers here are big ones on screen)
    c = max(c - vec3(0.005 * d), vec3(0.0)) * (1.0 + 0.005 * d);
    c += vec3(0.0, 0.0012, 0.001) * d * (1.0 - smoothstep(0.0, 0.1, l));
    // A heavy vignette: the edges fall away into the dark.
    let v = i.uv - 0.5;
    let r = length(v) * 1.414; // 0 centre, 1 corner
    let vig = smoothstep(0.12, 0.95, r);
    c *= mix(1.0, 0.04, clamp(vig * vig * (3.0 - 2.0 * vig) * pp.fx.y, 0.0, 1.0));
    // CRT: scanlines, a shadow mask, a faint flicker, and dark outside the glass.
    if (crt > 0.001) {
        let scan = 0.5 + 0.5 * sin(i.clip.y * 3.14159 * 0.5);
        c *= 1.0 - 0.45 * crt * scan;
        let m = u32(i.clip.x) % 3u;
        var mask = vec3(1.0 - 0.35 * crt);
        if (m == 0u) { mask.r = 1.0; } else if (m == 1u) { mask.g = 1.0; } else { mask.b = 1.0; }
        c *= mask * (1.0 + 0.3 * crt);
        c *= 1.0 - 0.025 * crt * (0.5 + 0.5 * sin(t * 47.0));
        if (outside) { c = vec3(0.0); }
    }
    // Film grain, stronger in the dark.
    let n = fract(sin(dot(i.clip.xy + pp.texel.w * 61.0, vec2(12.9898, 78.233))) * 43758.5453);
    c += (n - 0.5) * mix(0.012, 0.03, smoothstep(0.0, 0.4, l)) * pp.fx.w;
    return vec4<f32>(max(c, vec3(0.0)), 1.0);
}
