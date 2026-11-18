// Media viewer: the picture fitted into a box over a darkened city, through one of
// several effects. Output is premultiplied alpha, drawn over the final image.

struct V {
    rect: vec4<f32>,   // picture box on screen, pixels: x0, y0, x1, y1
    screen: vec4<f32>, // screen w, h; picture texture w, h
    p: vec4<f32>,      // effect, time, open (0..1), -
};
@group(0) @binding(0) var<uniform> v: V;
@group(0) @binding(1) var img: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var glyphs: texture_2d<f32>; // density ramp of characters, one row

const GLYPHS: f32 = 11.0;

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn lum(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.299, 0.587, 0.114));
}

fn tex(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(img, samp, uv, 0.0).rgb;
}

fn thermal(l: f32) -> vec3<f32> {
    let a = vec3<f32>(0.0, 0.0, 0.08);
    let b = vec3<f32>(0.15, 0.0, 0.55);
    let c = vec3<f32>(0.85, 0.05, 0.55);
    let d = vec3<f32>(1.0, 0.55, 0.0);
    let e = vec3<f32>(1.0, 1.0, 0.75);
    if (l < 0.25) { return mix(a, b, l / 0.25); }
    if (l < 0.5) { return mix(b, c, (l - 0.25) / 0.25); }
    if (l < 0.75) { return mix(c, d, (l - 0.5) / 0.25); }
    return mix(d, e, (l - 0.75) / 0.25);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let t = v.p.y;
    let open = v.p.z;
    let effect = i32(v.p.x + 0.5);
    let veil = 0.97 * open;

    // Opening: the box grows from a thin line, tearing as it settles.
    let r = v.rect;
    let mid = (r.y + r.w) * 0.5;
    let half_h = (r.w - r.y) * 0.5 * (0.04 + 0.96 * open * open);
    let box = vec4<f32>(r.x, mid - half_h, r.z, mid + half_h);
    if (pos.x < box.x || pos.x > box.z || pos.y < box.y || pos.y > box.w) {
        return vec4<f32>(0.0, 0.004, 0.012, 1.0) * veil;
    }
    var uv = (pos.xy - box.xy) / (box.zw - box.xy);
    let settle = 1.0 - open;
    let tick = floor(t * 18.0);
    let row = floor(uv.y * 40.0);
    uv.x += (hash(vec2<f32>(row, tick)) - 0.5) * 0.3 * settle * settle;

    let texel = 1.0 / v.screen.zw;
    var col = vec3<f32>(0.0);

    if (effect == 1) {
        // Hologram: the towers' look. Torn rows in bursts, RGB split, scanlines, cyan.
        let burst = step(0.82, hash(vec2<f32>(floor(t * 2.5), 7.0)));
        let jag = hash(vec2<f32>(floor(uv.y * 60.0), tick));
        var q = uv;
        q.x += (jag - 0.5) * 0.06 * burst * step(0.5, jag) + (jag - 0.5) * 0.002;
        let split = vec2<f32>(0.002 + 0.01 * burst, 0.0);
        col = vec3<f32>(tex(q + split).r, tex(q).g, tex(q - split).b);
        col *= 0.82 + 0.18 * sin(pos.y * 1.6);
        col *= 0.93 + 0.07 * hash(vec2<f32>(tick, 3.0));
        col += vec3<f32>(0.4, 0.9, 1.0) * exp(-abs(fract(uv.y - t * 0.15) - 0.5) * 30.0) * 0.08;
        col = mix(col, vec3<f32>(0.35, 0.85, 1.0) * lum(col) * 1.4, 0.3);
    } else if (effect == 2) {
        // CRT: curved glass, RGB phosphor stripes, scanlines, bleed, vignette.
        var c = uv * 2.0 - 1.0;
        c *= 1.0 + 0.07 * dot(c, c);
        let q = c * 0.5 + 0.5;
        if (q.x < 0.0 || q.x > 1.0 || q.y < 0.0 || q.y > 1.0) {
            return vec4<f32>(0.0, 0.0, 0.0, 1.0) * veil;
        }
        let bleed = vec2<f32>(texel.x * 2.0, 0.0);
        col = vec3<f32>(tex(q + bleed).r, tex(q).g, tex(q - bleed).b);
        let m = i32(pos.x) % 3;
        var mask = vec3<f32>(0.75);
        if (m == 0) { mask.r = 1.15; } else if (m == 1) { mask.g = 1.15; } else { mask.b = 1.15; }
        col *= mask;
        col *= 0.7 + 0.3 * pow(abs(sin(q.y * v.screen.w * 1.2)), 0.6);
        col *= 1.0 - 0.45 * dot(c * 0.7, c * 0.7);
        col *= 1.05 + 0.03 * sin(t * 50.0);
    } else if (effect == 3) {
        // Thermal: brightness as a heat palette.
        col = thermal(clamp(lum(tex(uv)) * 1.1, 0.0, 1.0));
    } else if (effect == 4) {
        // Data mosaic: the picture redrawn as glowing characters, like the tower faces.
        let cell = vec2<f32>(9.0, 16.0);
        let px = pos.xy - box.xy;
        let ci = floor(px / cell);
        let centre = (ci + 0.5) * cell / (box.zw - box.xy);
        let c = textureSampleLevel(img, samp, centre, 3.0).rgb;
        let l = clamp(pow(lum(c), 0.6) * 1.1, 0.0, 0.999);
        let g = floor(l * GLYPHS);
        let f = fract(px / cell);
        let cov = textureSampleLevel(glyphs, samp, vec2<f32>((g + f.x) / GLYPHS, f.y), 0.0).r;
        let ink = mix(vec3<f32>(0.35, 0.95, 1.0) * l, c / max(lum(c), 0.05) * l, 0.6);
        col = ink * cov * 2.6 + c * 0.1;
    } else if (effect == 5) {
        // Edge glow: neon outlines on black.
        let o = texel * 1.5;
        let tl = lum(tex(uv + vec2<f32>(-o.x, -o.y)));
        let tc = lum(tex(uv + vec2<f32>(0.0, -o.y)));
        let tr = lum(tex(uv + vec2<f32>(o.x, -o.y)));
        let ml = lum(tex(uv + vec2<f32>(-o.x, 0.0)));
        let mr = lum(tex(uv + vec2<f32>(o.x, 0.0)));
        let bl = lum(tex(uv + vec2<f32>(-o.x, o.y)));
        let bc = lum(tex(uv + vec2<f32>(0.0, o.y)));
        let br = lum(tex(uv + vec2<f32>(o.x, o.y)));
        let gx = (tr + 2.0 * mr + br) - (tl + 2.0 * ml + bl);
        let gy = (bl + 2.0 * bc + br) - (tl + 2.0 * tc + tr);
        let e = clamp(length(vec2<f32>(gx, gy)) * 2.5, 0.0, 1.0);
        let neon = mix(vec3<f32>(0.2, 0.95, 1.0), vec3<f32>(1.0, 0.25, 0.85), uv.y);
        col = neon * e * 1.3 + tex(uv) * 0.06;
    } else {
        col = tex(uv);
    }

    // A bright seam while it opens.
    col += vec3<f32>(0.6, 0.95, 1.0) * settle * exp(-abs(uv.y - 0.5) * 12.0) * 0.8;
    return vec4<f32>(col * open + vec3<f32>(0.0, 0.004, 0.012) * veil * (1.0 - open), 1.0) * max(veil, open);
}
