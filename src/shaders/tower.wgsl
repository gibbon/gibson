// Glass towers: translucent tinted body + emissive text mosaic on every face.
// Premultiplied-alpha blending, drawn back to front (back faces, then front faces).

struct Globals {
    view_proj: mat4x4<f32>,
    cam: vec4<f32>,     // xyz camera, w = time
    city: vec4<f32>,    // origin x, origin z, cell size, city radius
    haze: vec4<f32>,    // rgb haze colour
    screen: vec4<f32>,  // w, h, 1/w, 1/h
    hl: vec4<f32>,      // hovered name: texture layer, on (1/0), -, -
    hl_rect: vec4<f32>, // its box on the page: u0, v0, u1, v1
};
@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var faces: texture_2d_array<f32>;
@group(1) @binding(1) var samp: sampler;
@group(1) @binding(2) var previews: texture_2d_array<f32>; // colour pictures / video frames

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

struct VIn {
    @location(0) pos: vec3<f32>,      // unit box: x,z in −0.5..0.5, y in 0..1
    @location(1) nrm: vec3<f32>,
    @location(2) base: vec3<f32>,
    @location(3) size: vec3<f32>,
    @location(4) text_col: vec4<f32>, // rgb, a = appear (fade-in)
    @location(5) body_col: vec4<f32>, // rgb, a = hover
    @location(6) misc: vec4<f32>,     // layer, seed, has_texture, pages
    @location(7) pv: vec4<f32>,       // preview: layer (−1 none), used width, used height, is video
    @location(8) pv_rect: vec4<f32>,  // its box on the page: u0, v0, u1, v1
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) fuv: vec2<f32>,      // position on the face, world units (u across, v down from top)
    @location(2) @interpolate(flat) fsize: vec3<f32>, // face width, face height, face id
    @location(3) @interpolate(flat) text_col: vec4<f32>,
    @location(4) @interpolate(flat) body_col: vec4<f32>,
    @location(5) @interpolate(flat) misc: vec4<f32>,
    @location(6) @interpolate(flat) pv: vec4<f32>,
    @location(7) @interpolate(flat) pv_rect: vec4<f32>,
};

@vertex
fn vs(v: VIn) -> VOut {
    var o: VOut;
    let world = v.base + v.pos * v.size;
    o.world = world;
    o.clip = g.view_proj * vec4<f32>(world, 1.0);

    // Texture runs across each face so text reads correctly from outside.
    let n = v.nrm;
    if (abs(n.y) > 0.5) {
        o.fuv = vec2<f32>((v.pos.x + 0.5) * v.size.x, (v.pos.z + 0.5) * v.size.z);
        o.fsize = vec3<f32>(v.size.x, v.size.z, 4.0);
    } else if (abs(n.x) > 0.5) {
        let u = select((v.pos.z + 0.5), (0.5 - v.pos.z), n.x > 0.0);
        o.fuv = vec2<f32>(u * v.size.z, (1.0 - v.pos.y) * v.size.y);
        o.fsize = vec3<f32>(v.size.z, v.size.y, select(1.0, 3.0, n.x > 0.0));
    } else {
        let u = select((0.5 - v.pos.x), (v.pos.x + 0.5), n.z > 0.0);
        o.fuv = vec2<f32>(u * v.size.x, (1.0 - v.pos.y) * v.size.y);
        o.fsize = vec3<f32>(v.size.x, v.size.y, select(2.0, 0.0, n.z > 0.0));
    }
    o.text_col = v.text_col;
    o.body_col = v.body_col;
    o.misc = v.misc;
    o.pv = v.pv;
    o.pv_rect = v.pv_rect;
    return o;
}

@fragment
fn fs(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let t = g.cam.w;
    let fw = i.fsize.x;
    let fh = i.fsize.y;
    let face = i.fsize.z;
    let seed = i.misc.y;
    let top = face > 3.5;

    // One page is a face wide and two faces wide tall. Pages run down the front face,
    // then on around the tower (front → right → back → left), so tall towers keep
    // reading through the real content; only towers taller than their content repeat.
    let page_h = fw * 2.0;
    let tv = i.fuv.y / page_h;
    let tile = floor(tv);
    let per_face = max(ceil(fh / page_h - 0.001), 1.0);
    let pages = max(i.misc.w, 1.0);
    let around = f32((4u - u32(face)) % 4u);      // face ids 0 +z, 1 −x, 2 −z, 3 +x
    let page = select((around * per_face + tile) % pages, 0.0, top);
    // Gradients from the continuous coordinate, so mips don't seam at page edges.
    let cont = vec2<f32>(i.fuv.x / fw, tv);
    let puv = vec2<f32>(cont.x, tv - tile);
    let gx = dpdx(cont);
    let gy = dpdy(cont);
    let text = textureSampleGrad(faces, samp, puv, i32(i.misc.x + page), gx, gy).r * i.misc.z;

    // ── Preview picture (images, video frames), in its box at the top of every side,
    // as a glitching hologram: torn rows, jumped blocks, RGB split, scanlines, flicker.
    var pic = vec3<f32>(0.0);
    var pic_mask = 0.0;
    let pv = i.pv;
    if (pv.x >= 0.0 && !top && tile < 0.5) {
        let pr = i.pv_rect;
        let in_pic = step(pr.x, puv.x) * step(puv.x, pr.z) * step(pr.y, puv.y) * step(puv.y, pr.w);
        if (in_pic > 0.5) {
            let box = pr.zw - pr.xy;
            var q = (puv - pr.xy) / box; // 0..1 across the picture
            let tick = floor(t * 14.0);
            // Now and then a fit; and always on a video's montage cuts (pv.w > 1).
            let burst = max(step(0.84, hash(vec2<f32>(floor(t * 2.5), seed))), step(1.5, pv.w));
            let band = floor(q.y * 36.0);
            let jag = hash(vec2<f32>(band, tick + seed));
            q.x += (jag - 0.5) * 0.12 * burst * step(0.55, jag) + (jag - 0.5) * 0.004;
            let blk = hash(floor(q * vec2<f32>(8.0, 14.0)) + vec2<f32>(tick, seed));
            q.x += 0.04 * burst * step(0.92, blk);
            let scale = pv.yz; // the part of the layer the picture fills
            let uvq = clamp(q, vec2<f32>(0.0), vec2<f32>(1.0)) * scale;
            let ggx = gx / box * scale;
            let ggy = gy / box * scale;
            let split = vec2<f32>((0.002 + 0.012 * burst) * (1.0 + min(pv.w, 1.0)) * scale.x, 0.0);
            let lay = i32(pv.x);
            pic = vec3<f32>(
                textureSampleGrad(previews, samp, uvq + split, lay, ggx, ggy).r,
                textureSampleGrad(previews, samp, uvq, lay, ggx, ggy).g,
                textureSampleGrad(previews, samp, uvq - split, lay, ggx, ggy).b);
            let scan = 0.84 + 0.16 * sin(q.y * 560.0);
            let flick = 0.9 + 0.1 * hash(vec2<f32>(tick, seed * 3.1));
            let roll = exp(-abs(fract(q.y - t * 0.22 + seed) - 0.5) * 36.0);
            pic = pic * scan * flick + vec3<f32>(0.45, 0.9, 1.0) * roll * 0.12;
            pic *= 1.0 - 0.9 * step(0.95, hash(vec2<f32>(band, tick * 1.3 + seed))) * burst; // dropped rows
            let lum = dot(pic, vec3<f32>(0.299, 0.587, 0.114));
            pic = mix(pic, vec3<f32>(0.35, 0.85, 1.0) * lum * 1.4, 0.28); // glass-cyan cast
            pic_mask = 1.0;
        }
    }

    // The name under the mouse: an amber bar behind it, its letters lit up.
    let r = g.hl_rect;
    let in_box = step(r.x, puv.x) * step(puv.x, r.z) * step(r.y, puv.y) * step(puv.y, r.w);
    let hl = in_box * g.hl.y * select(0.0, 1.0, abs(i.misc.x + page - g.hl.x) < 0.5 && !top);

    // Edges: a crisp line plus a soft glow, the brightest thing on the glass.
    let d = min(min(i.fuv.x, fw - i.fuv.x), min(i.fuv.y, fh - i.fuv.y));
    let aa = max(fwidth(d), 1e-4);
    let edge = (1.0 - smoothstep(0.05, 0.05 + aa * 1.5, d)) * 1.0 + exp(-d * 6.0) * 0.25;

    // Lit base where the tower meets the floor, and a highlight bar sweeping down.
    let base_glow = select(exp(-i.world.y * 0.9) * 1.6, 0.0, top);
    let sweep_pos = fract(t * 0.035 + seed * 0.13) * (fh + 20.0) - 10.0;
    let sweep = exp(-abs(i.fuv.y - sweep_pos) * 1.2) * 0.25 * select(1.0, 0.0, top);

    let hover = i.body_col.a;
    let face_k = select(0.18, 1.0, front);                     // text bleeding through from behind is dimmer
    var tcol = i.text_col.rgb;
    if (top) { tcol = mix(tcol, vec3<f32>(0.24, 0.91, 0.78), 0.6); } // tops read teal from above

    var emissive = tcol * (text * 1.05 * face_k * (1.0 + sweep) + edge * (0.7 + hover) + base_glow * 0.6);
    emissive += vec3<f32>(0.75, 0.38, 1.0) * text * hover * 1.2 * face_k;   // hovered: magenta lift
    emissive = mix(emissive, vec3<f32>(1.0, 0.7, 0.26) * (0.3 + text * 2.4) * face_k, hl);

    emissive = mix(emissive, pic * 1.1 * face_k, pic_mask);

    var alpha = select(0.25, 0.5, front); // darker glass in front = more contrast
    alpha = mix(alpha, select(0.3, 0.88, front), pic_mask);
    var body = i.body_col.rgb * (1.0 + hover * 0.8);

    // Distance haze: emissive fades to the haze colour, then everything to black.
    let dist = length(i.world - g.cam.xyz);
    let fog = exp(-dist / 260.0);
    let far = exp(-dist / 900.0);
    var col = (body * alpha + emissive) * fog + g.haze.rgb * (1.0 - fog) * (alpha + text * 0.25);
    col *= far;

    let appear = i.text_col.a;
    return vec4<f32>(col * appear, alpha * appear);
}
