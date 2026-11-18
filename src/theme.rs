// Colour schemes: the glass, the text, the floor and the haze. Cycled with C.

pub struct Theme {
    pub name: &'static str,
    pub kind_tint: f32,       // 1: colour by file type; 0: everything in `text` / `body`
    pub text: [f32; 3],       // text colour (when not by file type)
    pub body: [f32; 3],       // glass tint (when not by file type)
    pub text_gain: f32,       // text brightness
    pub lit_var: f32,         // 0: all towers equally lit; 1: some blaze, some nearly dark
    pub glass_alpha: f32,     // how much the glass hides what's behind (darker glass)
    pub edge_gain: f32,       // brightness of the tower edges
    pub edge: [f32; 3],       // edge colour ...
    pub edge_mix: f32,        // ... and how much it replaces the text colour on edges
    pub floor_trace: [f32; 3],
    pub floor_lane: [f32; 3],
    pub floor_gain: f32,
    pub haze: [f32; 3],
    pub bloom: f32,
}

pub const THEMES: [Theme; 4] = [
    Theme {
        name: "NEON",
        kind_tint: 1.0,
        text: [0.28, 0.91, 1.0],
        body: [0.055, 0.165, 0.415],
        text_gain: 1.0,
        lit_var: 0.0,
        glass_alpha: 1.0,
        edge_gain: 1.0,
        edge: [0.0; 3],
        edge_mix: 0.0,
        floor_trace: [0.36, 0.25, 0.91],
        floor_lane: [0.30, 0.55, 1.0],
        floor_gain: 1.0,
        haze: [0.06, 0.19, 0.63],
        bloom: 0.45,
    },
    // Dark glass seen by its edges, pale blue-white text, some towers lit, some dark.
    Theme {
        name: "GLASS",
        kind_tint: 0.08,
        text: [0.6, 0.86, 1.0],
        body: [0.0, 0.004, 0.012],
        text_gain: 1.45,
        lit_var: 0.95,
        glass_alpha: 1.85,
        edge_gain: 0.85,
        edge: [0.5, 0.8, 1.0],
        edge_mix: 0.75,
        floor_trace: [0.3, 0.2, 0.95],
        floor_lane: [0.2, 0.45, 1.0],
        floor_gain: 0.32,
        haze: [0.008, 0.025, 0.09],
        bloom: 0.5,
    },
    // The film's "under attack" look: orange data in violet glass, blue traces.
    Theme {
        name: "ATTACK",
        kind_tint: 0.0,
        text: [1.0, 0.45, 0.18],
        body: [0.09, 0.02, 0.15],
        text_gain: 1.5,
        lit_var: 0.35,
        glass_alpha: 1.25,
        edge_gain: 0.6,
        edge: [0.55, 0.4, 1.0],
        edge_mix: 0.85,
        floor_trace: [0.2, 0.3, 1.0],
        floor_lane: [0.3, 0.7, 1.0],
        floor_gain: 0.6,
        haze: [0.14, 0.02, 0.16],
        bloom: 0.5,
    },
    Theme {
        name: "MATRIX",
        kind_tint: 0.08,
        text: [0.35, 1.0, 0.45],
        body: [0.0, 0.04, 0.015],
        text_gain: 1.15,
        lit_var: 0.65,
        glass_alpha: 1.3,
        edge_gain: 1.0,
        edge: [0.3, 1.0, 0.5],
        edge_mix: 0.5,
        floor_trace: [0.08, 0.42, 0.18],
        floor_lane: [0.2, 0.9, 0.4],
        floor_gain: 0.6,
        haze: [0.015, 0.1, 0.04],
        bloom: 0.5,
    },
];

/// How lit a tower is in themes with `lit_var`: most dim, some blazing. From its seed.
pub fn lit(seed: f32, var: f32) -> f32 {
    let h = ((seed * 12.9898).sin() * 43758.547).fract().abs();
    let f = if h > 0.62 { 1.4 } else { 0.18 + h * 0.55 };
    1.0 + var * (f - 1.0)
}
