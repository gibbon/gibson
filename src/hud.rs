// The 2D overlay: drives and a clickable path along the top, details of whatever is
// under the cursor at the bottom, and status messages. Drawn on the CPU into an
// RGBA image (premultiplied alpha) and only redrawn when something on it changes.

use crate::textgen::Fonts;
use fontdue::Font;
use std::path::{Path, PathBuf};

const CYAN: [f32; 3] = [0.55, 0.93, 1.0];
const DIM: [f32; 3] = [0.30, 0.58, 0.70];
const INK: [f32; 3] = [0.01, 0.05, 0.09]; // text on a lit (inverse) button
const WARN: [f32; 3] = [1.0, 0.55, 0.35];
const AMBER: [f32; 3] = [1.0, 0.72, 0.28]; // the selected tower

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x as f64 && y >= self.y as f64 && x < (self.x + self.w) as f64 && y < (self.y + self.h) as f64
    }
}

/// What a click on the HUD does.
#[derive(Clone, Debug, PartialEq)]
pub enum HudAction {
    Go(PathBuf), // open this folder
    Back,        // the folder before this one
    Up,          // the parent folder
}

/// What's under the cursor, already worded for the panel.
#[derive(Clone, PartialEq)]
pub struct HoverInfo {
    pub name: String,
    pub detail: String,
}

/// Everything the HUD shows; it's redrawn when this (or the window size) changes.
#[derive(Clone, PartialEq)]
pub struct HudState {
    pub path: PathBuf,
    pub drives: Vec<PathBuf>,
    pub can_back: bool,
    pub can_up: bool,
    pub fps: u32,
    pub hover: Option<HoverInfo>,
    pub selected: Option<(String, PathBuf, bool)>, // clicked tower: name, path, is folder
    pub status: String,
    pub warn: bool,     // status is an error
    pub hot: Option<usize>, // index into `hits` under the cursor
}

pub struct Hud {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
    /// Clickable places, and what clicking them does.
    pub hits: Vec<(Rect, HudAction)>,
    pub bar_h: i32,
    last: Option<HudState>,
    scale: f32,
}

impl Hud {
    pub fn new() -> Hud {
        Hud { w: 0, h: 0, px: Vec::new(), hits: Vec::new(), bar_h: 0, last: None, scale: 1.0 }
    }

    /// True if the cursor is over the HUD (so it shouldn't pick towers behind it).
    pub fn covers(&self, _x: f64, y: f64) -> bool {
        y < self.bar_h as f64
    }

    /// Redraws if anything changed; returns true when `px` is new.
    pub fn update(&mut self, st: &HudState, w: u32, h: u32, scale: f32, fonts: &Fonts) -> bool {
        if self.w == w && self.h == h && self.scale == scale && self.last.as_ref() == Some(st) {
            return false;
        }
        self.w = w;
        self.h = h;
        self.scale = scale;
        self.px.clear();
        self.px.resize((w * h * 4) as usize, 0);
        self.hits.clear();
        self.draw(st, fonts);
        self.last = Some(st.clone());
        true
    }

    fn draw(&mut self, st: &HudState, fonts: &Fonts) {
        let s = self.scale;
        let (w, h) = (self.w as i32, self.h as i32);
        let u = |v: f32| (v * s).round() as i32;

        // ── Top bar: drives, then the path as clickable segments ────────────
        let bar_h = u(40.0);
        self.bar_h = bar_h;
        self.fill(Rect { x: 0, y: 0, w, h: bar_h }, [0.0, 0.015, 0.04], 0.72);
        self.fill(Rect { x: 0, y: bar_h - 1, w, h: 1 }, DIM, 0.9);

        let mono = 15.0 * s;
        let base = (bar_h as f32 * 0.5 + mono * 0.36).round();
        let mut x = u(12.0);

        // Back and Up buttons, with drawn arrows (the fonts have no arrow glyphs).
        for (label, action, enabled) in [("BACK", HudAction::Back, st.can_back), ("UP", HudAction::Up, st.can_up)] {
            let tw = text_width(&fonts.mono, mono, label).ceil() as i32;
            let r = Rect { x, y: u(8.0), w: tw + u(34.0), h: bar_h - u(16.0) };
            let hot = enabled && st.hot == Some(self.hits.len());
            let a = if enabled { 1.0 } else { 0.3 };
            if hot {
                self.fill(r, CYAN, 0.22);
            }
            self.frame(r, CYAN, if hot { 1.0 } else { 0.6 * a });
            let (ax, ay, sz) = (r.x + u(9.0), r.y + r.h / 2, u(9.0));
            if action == HudAction::Back {
                self.arrow_left(ax, ay, sz, CYAN, a);
            } else {
                self.arrow_up(ax + sz / 2, ay, sz, CYAN, a);
            }
            self.text(&fonts.mono, mono, (r.x + u(24.0)) as f32, base, label, CYAN, a);
            if enabled {
                self.hits.push((r, action));
            }
            x += r.w + u(6.0);
        }
        x += u(10.0);
        self.fill(Rect { x, y: u(10.0), w: 1, h: bar_h - u(20.0) }, DIM, 0.8);
        x += u(12.0);

        for d in st.drives.clone() {
            let label = drive_label(&d);
            let tw = text_width(&fonts.mono, mono, &label).ceil() as i32;
            let r = Rect { x, y: u(8.0), w: tw + u(16.0), h: bar_h - u(16.0) };
            let here = st.path.starts_with(&d);
            let hot = st.hot == Some(self.hits.len());
            if here {
                self.fill(r, CYAN, 0.85);
            } else if hot {
                self.fill(r, CYAN, 0.22);
            }
            self.frame(r, CYAN, if here || hot { 1.0 } else { 0.6 });
            let col = if here { INK } else { CYAN };
            self.text(&fonts.mono, mono, (r.x + u(8.0)) as f32, base, &label, col, 1.0);
            self.hits.push((r, HudAction::Go(d)));
            x += r.w + u(6.0);
        }
        x += u(14.0);

        // Path segments, dropping the leading ones (shown as …) if they don't fit.
        let segs: Vec<PathBuf> = {
            let mut v: Vec<PathBuf> = st.path.ancestors().map(Path::to_path_buf).collect();
            v.reverse();
            v
        };
        let fps_label = format!("{:>3} FPS", st.fps);
        let fps_w = text_width(&fonts.mono, mono, &fps_label) as i32 + u(28.0);
        let status_w = fps_w + if st.status.is_empty() { 0 } else { text_width(&fonts.mono, mono, &st.status) as i32 + u(30.0) };
        let sel_label = st.selected.as_ref().map(|(n, _, _)| n.to_uppercase());
        let sel_w = sel_label.as_ref().map_or(0.0, |l| text_width(&fonts.mono, mono, l) + 40.0 * s);
        let room = ((w - x - status_w - u(12.0)) as f32 - sel_w).max(0.0);
        let gap = 22.0 * s;
        let widths: Vec<f32> = segs.iter().map(|p| text_width(&fonts.mono, mono, &seg_label(p)) + gap).collect();
        let mut first = 0;
        while first + 1 < segs.len() && widths[first..].iter().sum::<f32>() + gap > room {
            first += 1;
        }
        if first > 0 {
            x = self.text(&fonts.mono, mono, x as f32, base, "…", DIM, 1.0) as i32 + u(10.0);
        }
        for (i, p) in segs.iter().enumerate().skip(first) {
            let label = seg_label(p);
            let last = i + 1 == segs.len();
            let tw = text_width(&fonts.mono, mono, &label).ceil() as i32;
            let r = Rect { x: x - u(4.0), y: u(6.0), w: tw + u(8.0), h: bar_h - u(12.0) };
            let hot = st.hot == Some(self.hits.len());
            if hot && !last {
                self.fill(r, CYAN, 0.18);
            }
            let (col, a) = if last { (CYAN, 1.0) } else { (CYAN, if hot { 1.0 } else { 0.7 }) };
            let end = self.text(&fonts.mono, mono, x as f32, base, &label, col, a);
            if last {
                self.fill(Rect { x, y: bar_h - u(9.0), w: tw, h: u(2.0).max(1) }, CYAN, 0.9);
            } else {
                self.hits.push((r, HudAction::Go(p.clone())));
                self.marker(end as i32 + u(6.0), bar_h / 2, u(8.0), DIM);
            }
            x = end as i32 + gap as i32;
        }

        // The selected tower, in amber: where you are looking. A folder is clickable (enters it).
        if let (Some((_, path, is_dir)), Some(label)) = (&st.selected, sel_label) {
            x -= gap as i32;
            self.marker(x + u(6.0), bar_h / 2, u(8.0), AMBER);
            x += u(22.0);
            let tw = text_width(&fonts.mono, mono, &label).ceil() as i32;
            let r = Rect { x: x - u(6.0), y: u(8.0), w: tw + u(12.0), h: bar_h - u(16.0) };
            let hot = *is_dir && st.hot == Some(self.hits.len());
            self.fill(r, AMBER, if hot { 0.3 } else { 0.12 });
            self.frame(r, AMBER, 0.9);
            self.text(&fonts.mono, mono, x as f32, base, &label, AMBER, 1.0);
            if *is_dir {
                self.hits.push((r, HudAction::Go(path.clone())));
            }
        }

        // Frame rate at the far right, status just left of it.
        let fw = text_width(&fonts.mono, mono, &fps_label);
        let fps_x = w as f32 - fw - 14.0 * s;
        let fps_col = if st.fps >= 55 { DIM } else if st.fps >= 30 { AMBER } else { WARN };
        self.text(&fonts.mono, mono, fps_x, base, &fps_label, fps_col, 1.0);
        self.fill(Rect { x: fps_x as i32 - u(12.0), y: u(10.0), w: 1, h: bar_h - u(20.0) }, DIM, 0.8);
        if !st.status.is_empty() {
            let tw = text_width(&fonts.mono, mono, &st.status);
            let col = if st.warn { WARN } else { CYAN };
            self.text(&fonts.mono, mono, fps_x - u(26.0) as f32 - tw, base, &st.status, col, 1.0);
        }

        // ── Bottom left: what's under the cursor, or how to drive ───────────
        let hint = "DRAG look   WASD/QE fly   CLICK fly to   DOUBLE-CLICK / ENTER go in   BACKSPACE up   ALT+← / MOUSE BACK back";
        let small = 13.0 * s;
        match &st.hover {
            Some(hv) => {
                let big = 22.0 * s;
                let name = hv.name.to_uppercase();
                let pw = (text_width(&fonts.wide, big, &name).max(text_width(&fonts.mono, mono, &hv.detail)) as i32
                    + u(28.0))
                    .min(w - u(24.0));
                let r = Rect { x: u(12.0), y: h - u(104.0), w: pw, h: u(78.0) };
                self.fill(r, [0.0, 0.015, 0.04], 0.72);
                self.frame(r, CYAN, 0.8);
                self.fill(Rect { x: r.x, y: r.y, w: u(4.0), h: r.h }, CYAN, 0.9);
                self.text(&fonts.wide, big, (r.x + u(16.0)) as f32, (r.y + u(32.0)) as f32, &name, CYAN, 1.0);
                self.text(&fonts.mono, mono, (r.x + u(16.0)) as f32, (r.y + u(56.0)) as f32, &hv.detail, CYAN, 0.8);
                self.text(&fonts.mono, small, u(14.0) as f32, (h - u(10.0)) as f32, hint, DIM, 0.85);
            }
            None => {
                self.text(&fonts.mono, small, u(14.0) as f32, (h - u(10.0)) as f32, hint, DIM, 0.9);
            }
        }
    }

    // ── Drawing (premultiplied RGBA, "over" blending) ──────────────────────
    fn blend(&mut self, x: i32, y: i32, c: [f32; 3], a: f32) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 || a <= 0.0 {
            return;
        }
        let i = ((y as u32 * self.w + x as u32) * 4) as usize;
        let p = &mut self.px[i..i + 4];
        for k in 0..3 {
            p[k] = ((c[k] * a * 255.0) + p[k] as f32 * (1.0 - a)).round().min(255.0) as u8;
        }
        p[3] = ((a * 255.0) + p[3] as f32 * (1.0 - a)).round().min(255.0) as u8;
    }

    fn fill(&mut self, r: Rect, c: [f32; 3], a: f32) {
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                self.blend(x, y, c, a);
            }
        }
    }

    fn frame(&mut self, r: Rect, c: [f32; 3], a: f32) {
        self.fill(Rect { h: 1, ..r }, c, a);
        self.fill(Rect { y: r.y + r.h - 1, h: 1, ..r }, c, a);
        self.fill(Rect { w: 1, y: r.y + 1, h: r.h - 2, ..r }, c, a);
        self.fill(Rect { x: r.x + r.w - 1, w: 1, y: r.y + 1, h: r.h - 2, ..r }, c, a);
    }

    fn arrow_left(&mut self, x: i32, y_mid: i32, size: i32, c: [f32; 3], a: f32) {
        for dx in 0..size {
            let half = dx / 2;
            for dy in -half..=half {
                self.blend(x + dx, y_mid + dy, c, a);
            }
        }
    }

    fn arrow_up(&mut self, x_mid: i32, y_mid: i32, size: i32, c: [f32; 3], a: f32) {
        for dy in 0..size {
            let half = dy / 2;
            for dx in -half..=half {
                self.blend(x_mid + dx, y_mid - size / 2 + dy, c, a);
            }
        }
    }

    fn marker(&mut self, x: i32, y_mid: i32, size: i32, c: [f32; 3]) {
        for dx in 0..size {
            let half = (size - dx) / 2;
            for dy in -half..=half {
                self.blend(x + dx, y_mid + dy, c, 0.9);
            }
        }
    }

    /// Draws text with its baseline at `y`; returns the end x.
    fn text(&mut self, font: &Font, px: f32, x: f32, y: f32, text: &str, c: [f32; 3], a: f32) -> f32 {
        let mut pen = x;
        for ch in text.chars() {
            let gl = crate::textgen::glyph(font, ch, px);
            let (m, bmp) = (&gl.0, &gl.1);
            let gx = pen.round() as i32 + m.xmin;
            let gy = y as i32 - m.height as i32 - m.ymin;
            for row in 0..m.height {
                for col in 0..m.width {
                    let cov = bmp[row * m.width + col] as f32 / 255.0;
                    self.blend(gx + col as i32, gy + row as i32, c, cov * a);
                }
            }
            pen += m.advance_width;
        }
        pen
    }
}

fn text_width(font: &Font, px: f32, text: &str) -> f32 {
    text.chars().map(|c| font.metrics(c, px).advance_width).sum()
}

/// "C:\" → "C:", "/" → "/".
fn drive_label(d: &Path) -> String {
    let s = d.display().to_string();
    let t = s.trim_end_matches(['\\', '/']);
    if t.is_empty() { "/".into() } else { t.to_uppercase() }
}

fn seg_label(p: &Path) -> String {
    match p.file_name() {
        Some(n) => n.to_string_lossy().to_uppercase(),
        None => drive_label(p),
    }
}
