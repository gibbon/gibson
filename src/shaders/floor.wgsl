// The circuit board the city stands on: violet Manhattan traces, pads, bright
// lane edges along the streets, and cyan pulses racing down the traces.

struct Globals {
    view_proj: mat4x4<f32>,
    cam: vec4<f32>,
    city: vec4<f32>,    // origin x, origin z, cell size, city radius
    haze: vec4<f32>,
    screen: vec4<f32>,
    hl: vec4<f32>,
    hl_rect: vec4<f32>,
    th0: vec4<f32>,     // theme: glass alpha, edge gain, text gain, colour-by-type
    th1: vec4<f32>,     // theme: floor trace rgb, floor gain
    th2: vec4<f32>,     // theme: floor lane rgb, -
    th3: vec4<f32>,     // theme: edge rgb, edge mix
};
@group(0) @binding(0) var<uniform> g: Globals;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    // Big quad centred under the camera.
    var corners = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
        vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0));
    let c = corners[vi] * 3000.0 + g.cam.xz;
    var o: VOut;
    o.world = vec3<f32>(c.x, 0.0, c.y);
    o.clip = g.view_proj * vec4<f32>(o.world, 1.0);
    return o;
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

// Anti-aliased line of half-width w at distance d, plus a soft halo.
fn trace(d: f32, w: f32, aa: f32) -> vec2<f32> {
    let core = 1.0 - smoothstep(w, w + aa, d);
    let halo = exp(-d * 1.6) * 0.18;
    return vec2<f32>(core, halo);
}

// One layer of traces: tracks run along x and z at fixed offsets in each cell;
// a track segment exists in a cell at random, so runs join up into routes.
fn circuit(p: vec2<f32>, scale: f32, t: f32, aa: f32) -> vec3<f32> {
    let q = p / scale;
    let cell = floor(q);
    let f = fract(q);
    var line = 0.0;
    var halo = 0.0;
    var pulse = 0.0;
    var pad = 0.0;
    for (var k = 0; k < 3; k++) {
        let off = (f32(k) + 0.5) / 3.0;
        let kf = f32(k);
        // along x (track k of this row)
        let on_x = step(0.5, hash(cell + vec2<f32>(kf * 17.3, 3.1)));
        let dx = abs(f.y - off) * scale;
        let lx = trace(dx, 0.09, aa) * on_x;
        line += lx.x; halo += lx.y;
        let row_seed = hash(vec2<f32>(cell.y, kf + 40.0));
        let ph_x = fract(q.x * 0.06 - t * (0.25 + row_seed * 0.3) + row_seed * 9.0);
        pulse += lx.x * smoothstep(0.02, 0.0, abs(ph_x - 0.5)) * step(0.6, row_seed);
        // along z
        let on_z = step(0.55, hash(cell + vec2<f32>(5.7, kf * 23.9)));
        let dz = abs(f.x - off) * scale;
        let lz = trace(dz, 0.09, aa) * on_z;
        line += lz.x; halo += lz.y;
        let col_seed = hash(vec2<f32>(cell.x + 70.0, kf));
        let ph_z = fract(q.y * 0.06 + t * (0.25 + col_seed * 0.3) + col_seed * 5.0);
        pulse += lz.x * smoothstep(0.02, 0.0, abs(ph_z - 0.5)) * step(0.6, col_seed);
        // pads/vias where tracks cross
        let pc = vec2<f32>(off, (f32((k + 1) % 3) + 0.5) / 3.0);
        let pr = length((f - pc) * scale);
        let has_pad = step(0.8, hash(cell + vec2<f32>(kf, 91.0)));
        pad += has_pad * (1.0 - smoothstep(0.26, 0.26 + aa, pr)) * smoothstep(0.1, 0.1 + aa, pr)
             + has_pad * (1.0 - smoothstep(0.08, 0.08 + aa, pr));
    }
    return vec3<f32>(min(line, 1.0) + halo, pulse, min(pad, 1.0));
}

@fragment
fn fs(i: VOut) -> @location(0) vec4<f32> {
    let t = g.cam.w;
    let p = i.world.xz;
    let aa = max(fwidth(p.x), fwidth(p.y)) * 1.2;

    let fine = circuit(p, 2.2, t, aa);
    let coarse = circuit(p + vec2<f32>(31.0, 17.0), 6.5, t * 0.7, aa);

    // Street lanes: double neon edges along every cell boundary, with fast streaks.
    let cell = g.city.z;
    let rel = (p - g.city.xy) / cell + 0.5;           // cell boundaries at integers
    let dl = abs(fract(rel) - 0.5) * cell;              // distance from street centre line
    let lane = vec2<f32>(
        1.0 - smoothstep(0.12, 0.12 + aa, abs(abs(dl.y) - 3.2)),   // lanes running along x
        1.0 - smoothstep(0.12, 0.12 + aa, abs(abs(dl.x) - 3.2)));  // lanes running along z
    let sx = fract(p.x * 0.012 - t * 0.6 + floor(rel.y) * 0.37);
    let sz = fract(p.y * 0.012 + t * 0.5 + floor(rel.x) * 0.61);
    let streak = lane.x * smoothstep(0.08, 0.0, abs(sx - 0.5)) + lane.y * smoothstep(0.08, 0.0, abs(sz - 0.5));

    // Colours from the theme.
    let trace_col = g.th1.rgb * 1.6;
    let pad_col = mix(g.th1.rgb, vec3<f32>(1.0), 0.35) * 1.6;
    let pulse_col = mix(g.th2.rgb, vec3<f32>(1.0), 0.65) * 3.0;
    let lane_col = g.th2.rgb * 1.4;

    // Kept dim so the board reads as background; only pulses and streaks get bright.
    var col = trace_col * (fine.x * 0.18 + coarse.x * 0.35)
            + pad_col * (fine.z * 0.15 + coarse.z * 0.35)
            + pulse_col * (fine.y + coarse.y) * 0.6
            + lane_col * max(lane.x, lane.y) * 0.5
            + pulse_col * streak * 0.9;

    col *= g.th1.w;

    // Calm the board outside the city, and fade with distance (haze, then black).
    let city_r = g.city.w;
    let out_d = max(length(p - g.cam.xz) * 0.0, max(abs(p.x) - city_r, abs(p.y) - city_r));
    col *= mix(1.0, 0.35, smoothstep(0.0, 120.0, out_d));
    let dist = length(i.world - g.cam.xyz);
    let fog = exp(-dist / 230.0);
    col = col * fog + g.haze.rgb * (1.0 - fog) * 0.12 * (1.0 - exp(-dist / 400.0));
    col *= exp(-dist / 1100.0);
    // Smoke lying on the board.
    let sm = smoke(i.world, g.cam.xyz, g.cam.w);
    col = mix(col, smoke_colour(g.haze.rgb, g.th1.rgb, sm.y), sm.x);
    return vec4<f32>(col, 1.0);
}
