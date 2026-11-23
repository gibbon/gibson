// The 2D overlay: drives and a clickable path along the top, details of whatever is
// under the cursor at the bottom, and status messages. Drawn on the CPU into an
// RGBA image (premultiplied alpha) and only redrawn when something on it changes.

use crate::filetype::Cat;
use crate::textgen::Fonts;
use fontdue::Font;
use std::path::{Path, PathBuf};

const CYAN: [f32; 3] = [0.55, 0.93, 1.0];
const DIM: [f32; 3] = [0.30, 0.58, 0.70];
const INK: [f32; 3] = [0.01, 0.05, 0.09]; // text on a lit (inverse) button
const WARN: [f32; 3] = [1.0, 0.55, 0.35];
const AMBER: [f32; 3] = [1.0, 0.72, 0.28]; // the selected tower
const SEARCH: [f32; 3] = [0.8, 1.0, 0.62]; // search box and matches

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
    Help,        // show / hide the hotkeys
    Filter(Option<Cat>), // toggle a file type (None = show all)
    SearchFilter,        // search: hide non-matches instead of dimming them
    Effect(usize),       // viewer: pick an effect
    PlayPause,           // viewer: pause / play the video
    CloseViewer,
    TowerFx,             // next effect on the tower previews
    AutoFly,             // fly itself / stop
    Theme,               // next colour scheme
    Look,                // open / close the LOOK panel
    LookSlider(usize),   // LOOK panel: a slider (press and drag)
    LookReset,           // LOOK panel: back to the defaults
}

/// The media viewer's overlay.
#[derive(Clone, PartialEq)]
pub struct ViewerHud {
    pub name: String,
    pub index: usize,
    pub count: usize,
    pub size: Option<(u32, u32)>,
    pub effect: usize,
    pub video: Option<(f32, f32, bool)>, // time, duration, paused
    pub sound: Option<(u32, bool)>,      // volume percent, muted (None: no sound track)
    pub error: Option<String>,
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
    pub search: Option<(String, usize)>, // open search box: text, matches
    pub help: bool,                      // hotkey panel open
    pub search_filter: bool,             // search hides non-matches
    pub cats: Vec<(Cat, usize)>,         // file types in this folder, with counts
    pub filter: Vec<Cat>,                // shown types (empty = all)
    pub tower_fx: usize,                 // effect on tower previews
    pub autofly: bool,                   // flying itself
    pub theme: &'static str,             // colour scheme name
    pub viewer: Option<ViewerHud>,
    pub hover: Option<HoverInfo>,
    pub selected: Option<(String, PathBuf, bool)>, // clicked tower: name, path, is folder
    pub status: String,
    pub warn: bool,     // status is an error
    pub hot: Option<usize>, // index into `hits` under the cursor
    pub hack: Option<(f32, bool, bool)>, // garbage file copying: progress, done, blink
    pub dwell: Option<(String, f32)>,    // inside a folder tower: its name, 0..1 until it opens
    pub radio: Option<(bool, bool)>,     // YouTube player in the corner: playing yet, blink
    pub look: Option<[u32; crate::settings::LOOK_N]>, // LOOK panel open: its slider values
}

pub struct Hud {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
    /// Clickable places, and what clicking them does.
    pub hits: Vec<(Rect, HudAction)>,
    pub bar_h: i32,
    pub scrub: Option<Rect>, // viewer: the video timeline (click / drag to seek)
    pub look_rect: Option<Rect>, // the LOOK panel, while open
    help_open: bool,
    viewer_open: bool,
    last: Option<HudState>,
    scale: f32,
}

impl Hud {
    pub fn new() -> Hud {
        Hud {
            w: 0,
            h: 0,
            px: Vec::new(),
            hits: Vec::new(),
            bar_h: 0,
            scrub: None,
            look_rect: None,
            help_open: false,
            viewer_open: false,
            last: None,
            scale: 1.0,
        }
    }

    /// True if the cursor is over the HUD (so it shouldn't pick towers behind it).
    pub fn covers(&self, x: f64, y: f64) -> bool {
        self.help_open
            || self.viewer_open
            || y < self.bar_h as f64
            || self.look_rect.is_some_and(|r| r.contains(x, y))
    }

    /// The bar of a LOOK slider, for turning the cursor into a value.
    pub fn slider_bar(&self, k: usize) -> Option<Rect> {
        self.hits.iter().find(|(_, a)| *a == HudAction::LookSlider(k)).map(|(r, _)| *r)
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
        self.scrub = None;
        self.look_rect = None;
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

        // The drive we're on: the longest one the path starts with (home beats /).
        let on = st.drives.iter().filter(|d| st.path.starts_with(d)).max_by_key(|d| d.as_os_str().len()).cloned();
        for d in st.drives.clone() {
            let label = drive_label(&d);
            let tw = text_width(&fonts.mono, mono, &label).ceil() as i32;
            let r = Rect { x, y: u(8.0), w: tw + u(16.0), h: bar_h - u(16.0) };
            let here = on.as_ref() == Some(&d);
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
        let help_w = u(40.0);
        let status_w = fps_w + help_w + if st.status.is_empty() { 0 } else { text_width(&fonts.mono, mono, &st.status) as i32 + u(30.0) };
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

        // The ? button: opens the hotkey panel.
        let side = bar_h - u(16.0);
        let hb = Rect { x: fps_x as i32 - u(24.0) - side, y: u(8.0), w: side, h: side };
        let hot = st.hot == Some(self.hits.len());
        if st.help {
            self.fill(hb, CYAN, 0.85);
        } else if hot {
            self.fill(hb, CYAN, 0.22);
        }
        self.frame(hb, CYAN, if st.help || hot { 1.0 } else { 0.6 });
        let qw = text_width(&fonts.mono, mono, "?");
        let qcol = if st.help { INK } else { CYAN };
        self.text(&fonts.mono, mono, hb.x as f32 + (hb.w as f32 - qw) / 2.0, base, "?", qcol, 1.0);
        self.hits.push((hb, HudAction::Help));

        if !st.status.is_empty() {
            let tw = text_width(&fonts.mono, mono, &st.status);
            let col = if st.warn { WARN } else { CYAN };
            self.text(&fonts.mono, mono, hb.x as f32 - u(14.0) as f32 - tw, base, &st.status, col, 1.0);
        }

        // ── File-type filter strip under the bar ───────────────────────────
        let mut below = bar_h;
        if !st.cats.is_empty() && st.viewer.is_none() {
            let strip_h = u(30.0);
            self.fill(Rect { x: 0, y: bar_h, w, h: strip_h }, [0.0, 0.012, 0.035], 0.62);
            self.fill(Rect { x: 0, y: bar_h + strip_h - 1, w, h: 1 }, DIM, 0.5);
            let small = 13.0 * s;
            let by = (bar_h + strip_h / 2) as f32 + small * 0.36;
            let mut x = self.text(&fonts.mono, small, u(14.0) as f32, by, "SHOW", DIM, 1.0) as i32 + u(12.0);
            let chips: Vec<(Option<Cat>, String, bool)> = std::iter::once((
                None,
                format!("ALL {}", st.cats.iter().map(|c| c.1).sum::<usize>()),
                st.filter.is_empty(),
            ))
            .chain(st.cats.iter().map(|&(c, n)| (Some(c), format!("{} {n}", c.label()), st.filter.contains(&c))))
            .collect();
            for (cat, label, on) in chips {
                let tw = text_width(&fonts.mono, small, &label).ceil() as i32;
                let r = Rect { x, y: bar_h + u(5.0), w: tw + u(14.0), h: strip_h - u(10.0) };
                let hot = st.hot == Some(self.hits.len());
                if on {
                    self.fill(r, CYAN, 0.8);
                } else if hot {
                    self.fill(r, CYAN, 0.2);
                }
                self.frame(r, CYAN, if on || hot { 1.0 } else { 0.45 });
                self.text(&fonts.mono, small, (r.x + u(7.0)) as f32, by, &label, if on { INK } else { CYAN }, 1.0);
                self.hits.push((r, HudAction::Filter(cat)));
                x += r.w + u(6.0);
            }
            // Right end: which effect the tower previews use (click or T for the next).
            let fx = format!("TOWER FX  {}  T", crate::viewer::EFFECTS[st.tower_fx % crate::viewer::EFFECTS.len()]);
            let tw = text_width(&fonts.mono, small, &fx).ceil() as i32;
            let r = Rect { x: w - tw - u(30.0), y: bar_h + u(5.0), w: tw + u(14.0), h: strip_h - u(10.0) };
            if r.x > x + u(10.0) {
                let hot = st.hot == Some(self.hits.len());
                self.fill(r, AMBER, if hot { 0.25 } else { 0.1 });
                self.frame(r, AMBER, if hot { 1.0 } else { 0.7 });
                self.text(&fonts.mono, small, (r.x + u(7.0)) as f32, by, &fx, AMBER, 1.0);
                self.hits.push((r, HudAction::TowerFx));
            }
            // AUTOFLY switch, left of it.
            let af = if st.autofly { "AUTOFLY ON  P" } else { "AUTOFLY  P" };
            let aw = text_width(&fonts.mono, small, af).ceil() as i32;
            let ar = Rect { x: r.x - aw - u(24.0), y: r.y, w: aw + u(14.0), h: r.h };
            if ar.x > x + u(10.0) {
                let hot = st.hot == Some(self.hits.len());
                if st.autofly {
                    self.fill(ar, CYAN, 0.85);
                } else if hot {
                    self.fill(ar, CYAN, 0.2);
                }
                self.frame(ar, CYAN, if st.autofly || hot { 1.0 } else { 0.6 });
                self.text(&fonts.mono, small, (ar.x + u(7.0)) as f32, by, af, if st.autofly { INK } else { CYAN }, 1.0);
                self.hits.push((ar, HudAction::AutoFly));
            }
            // THEME, left of that.
            let tl = format!("THEME  {}  C", st.theme);
            let tw2 = text_width(&fonts.mono, small, &tl).ceil() as i32;
            let tr = Rect { x: ar.x - tw2 - u(24.0), y: r.y, w: tw2 + u(14.0), h: r.h };
            if tr.x > x + u(10.0) {
                let hot = st.hot == Some(self.hits.len());
                self.fill(tr, SEARCH, if hot { 0.25 } else { 0.08 });
                self.frame(tr, SEARCH, if hot { 1.0 } else { 0.6 });
                self.text(&fonts.mono, small, (tr.x + u(7.0)) as f32, by, &tl, SEARCH, 1.0);
                self.hits.push((tr, HudAction::Theme));
            }
            // LOOK, left of that.
            let ll = "LOOK  L";
            let lw = text_width(&fonts.mono, small, ll).ceil() as i32;
            let lr = Rect { x: tr.x - lw - u(24.0), y: r.y, w: lw + u(14.0), h: r.h };
            if lr.x > x + u(10.0) {
                let on = st.look.is_some();
                let hot = st.hot == Some(self.hits.len());
                if on {
                    self.fill(lr, CYAN, 0.85);
                } else if hot {
                    self.fill(lr, CYAN, 0.2);
                }
                self.frame(lr, CYAN, if on || hot { 1.0 } else { 0.6 });
                self.text(&fonts.mono, small, (lr.x + u(7.0)) as f32, by, ll, if on { INK } else { CYAN }, 1.0);
                self.hits.push((lr, HudAction::Look));
            }
            below = bar_h + strip_h;
        }
        self.bar_h = below;

        // ── Search box, centred under the bar ──────────────────────────────
        if let (Some((q, n)), None) = (&st.search, &st.viewer) {
            let bw = u(700.0).min(w - u(24.0));
            let r = Rect { x: (w - bw) / 2, y: below + u(10.0), w: bw, h: u(38.0) };
            self.fill(r, [0.0, 0.015, 0.04], 0.85);
            self.frame(r, SEARCH, 1.0);
            let by = (r.y + r.h / 2) as f32 + mono * 0.36;
            let x0 = self.text(&fonts.mono, mono, (r.x + u(12.0)) as f32, by, "SEARCH", SEARCH, 0.8);
            self.marker(x0 as i32 + u(8.0), r.y + r.h / 2, u(8.0), SEARCH);
            let qx = self.text(&fonts.mono, mono, x0 + 26.0 * s, by, &q.to_uppercase(), SEARCH, 1.0);
            self.fill(Rect { x: qx as i32 + u(2.0), y: r.y + u(9.0), w: u(8.0), h: r.h - u(18.0) }, SEARCH, 0.8);
            let info = if q.is_empty() {
                "TYPE A NAME   ESC CLOSE".to_string()
            } else {
                format!("{n} {}   ENTER GO   DOWN NEXT   ESC CLOSE", if *n == 1 { "MATCH" } else { "MATCHES" })
            };
            let iw = text_width(&fonts.mono, small_px(s), &info);
            let ix = (r.x + r.w) as f32 - iw - 12.0 * s;
            self.text(&fonts.mono, small_px(s), ix, by, &info, DIM, 1.0);
            // FILTER switch: hide what doesn't match, live, instead of dimming it.
            let label = if st.search_filter { "FILTER ON  TAB" } else { "FILTER OFF  TAB" };
            let fw = text_width(&fonts.mono, small_px(s), label).ceil() as i32;
            let fr = Rect { x: ix as i32 - fw - u(30.0), y: r.y + u(7.0), w: fw + u(14.0), h: r.h - u(14.0) };
            let hot = st.hot == Some(self.hits.len());
            if st.search_filter {
                self.fill(fr, SEARCH, 0.85);
            } else if hot {
                self.fill(fr, SEARCH, 0.2);
            }
            self.frame(fr, SEARCH, 0.9);
            let col = if st.search_filter { INK } else { SEARCH };
            self.text(&fonts.mono, small_px(s), (fr.x + u(7.0)) as f32, by, label, col, 1.0);
            self.hits.push((fr, HudAction::SearchFilter));
        }

        self.viewer_open = st.viewer.is_some();
        if let Some(v) = &st.viewer {
            self.viewer_overlay(v, st.hot, fonts);
            self.help_open = st.help;
            if st.help {
                self.help_panel(fonts);
            }
            return;
        }

        // ── Bottom left: what's under the cursor, or how to drive ───────────
        let hint = "? HELP   DRAG look   WASD fly   CLICK fly to   DOUBLE-CLICK open   / search   BACKSPACE up";
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

        if let Some((name, p)) = &st.dwell {
            self.dwell_meter(name, *p, fonts);
        }

        if let Some((p, done, blink)) = st.hack {
            self.hack_panel(p, done, blink, fonts);
        }

        if let Some((playing, blink)) = st.radio {
            self.radio_bezel(playing, blink, fonts);
        }

        if let (Some(vals), None) = (st.look, &st.viewer) {
            self.look_panel(&vals, st.hot, fonts);
        }

        self.help_open = st.help;
        if st.help {
            self.help_panel(fonts);
        }
    }

    /// The LOOK panel (L): sliders for the smoke, vignette, darkness, grain, CRT and glitch,
    /// under the strip on the right. Changes show at once.
    fn look_panel(&mut self, vals: &[u32; crate::settings::LOOK_N], hot: Option<usize>, fonts: &Fonts) {
        let s = self.scale;
        let u = |v: f32| (v * s).round() as i32;
        let w = self.w as i32;
        let small = 13.0 * s;
        let row = u(46.0);
        let pw = u(340.0).min(w - u(24.0));
        let n = crate::settings::LOOK.len() as i32;
        let r = Rect { x: w - pw - u(14.0), y: self.bar_h + u(10.0), w: pw, h: u(56.0) + n * row + u(48.0) };
        self.look_rect = Some(r);
        self.fill(r, [0.0, 0.012, 0.035], 0.9);
        self.frame(r, CYAN, 0.85);
        self.fill(Rect { x: r.x, y: r.y, w: r.w, h: u(3.0) }, CYAN, 0.9);
        self.text(&fonts.wide, 17.0 * s, (r.x + u(18.0)) as f32, (r.y + u(34.0)) as f32, "LOOK", CYAN, 1.0);
        let close = "L  ESC  CLOSE";
        let cw = text_width(&fonts.mono, 12.0 * s, close);
        self.text(&fonts.mono, 12.0 * s, (r.x + r.w) as f32 - cw - 18.0 * s, (r.y + u(32.0)) as f32, close, DIM, 1.0);

        for (k, (key, label, _)) in crate::settings::LOOK.iter().enumerate() {
            let colour = *key == "smoke_colour";
            let y = r.y + u(56.0) + k as i32 * row;
            let v = vals[k].min(100);
            let x0 = r.x + u(18.0);
            let bw = r.w - u(36.0);
            self.text(&fonts.mono, small, x0 as f32, (y + u(14.0)) as f32, label, CYAN, 0.95);
            let shown = match (v, colour) {
                (0, true) => "GREY".to_string(),
                (0, false) => "OFF".to_string(),
                _ => v.to_string(),
            };
            let vw = text_width(&fonts.mono, small, &shown);
            self.text(&fonts.mono, small, (x0 + bw) as f32 - vw, (y + u(14.0)) as f32, &shown, AMBER, 1.0);
            let bar = Rect { x: x0, y: y + u(24.0), w: bw, h: u(8.0) };
            let is_hot = hot == Some(self.hits.len());
            self.fill(bar, [0.0, 0.02, 0.05], 0.8);
            self.frame(bar, CYAN, if is_hot { 1.0 } else { 0.6 });
            let fill_w = (bar.w as f32 * v as f32 / 100.0).round() as i32;
            if colour {
                // The colour wheel along the bar (grey at the very start).
                for px in 0..bar.w {
                    let h = px as f32 / bar.w.max(1) as f32;
                    let c = if px < u(4.0) { [0.6, 0.62, 0.66] } else { hue_rgb(h) };
                    self.fill(Rect { x: bar.x + px, y: bar.y + 1, w: 1, h: bar.h - 2 }, c, 0.9);
                }
            } else {
                self.fill(Rect { x: bar.x, y: bar.y, w: fill_w, h: bar.h }, CYAN, 0.75);
            }
            let knob = Rect { x: bar.x + fill_w - u(3.0), y: bar.y - u(5.0), w: u(6.0), h: bar.h + u(10.0) };
            self.fill(knob, AMBER, 1.0);
            // What takes the mouse: the bar's width, and taller than it, to be easy to grab.
            self.hits.push((Rect { x: bar.x, y: bar.y - u(10.0), w: bar.w, h: bar.h + u(20.0) }, HudAction::LookSlider(k)));
        }

        // RESET, bottom left; a hint on the right.
        let rl = "RESET";
        let rw = text_width(&fonts.mono, small, rl).ceil() as i32;
        let rr = Rect { x: r.x + u(18.0), y: r.y + r.h - u(38.0), w: rw + u(16.0), h: u(24.0) };
        let is_hot = hot == Some(self.hits.len());
        if is_hot {
            self.fill(rr, CYAN, 0.2);
        }
        self.frame(rr, CYAN, if is_hot { 1.0 } else { 0.6 });
        self.text(&fonts.mono, small, (rr.x + u(8.0)) as f32, (rr.y + u(17.0)) as f32, rl, CYAN, 1.0);
        self.hits.push((rr, HudAction::LookReset));
        let hint = "DRAG THE BARS";
        let hw = text_width(&fonts.mono, 12.0 * s, hint);
        self.text(&fonts.mono, 12.0 * s, (r.x + r.w) as f32 - hw - 18.0 * s, (rr.y + u(17.0)) as f32, hint, DIM, 0.9);
    }

    /// Inside a folder tower: "ENTERING NAME" and a bar filling until it opens.
    fn dwell_meter(&mut self, name: &str, p: f32, fonts: &Fonts) {
        let s = self.scale;
        let u = |v: f32| (v * s).round() as i32;
        let (w, h) = (self.w as i32, self.h as i32);
        let px = 18.0 * s;
        let label = shorten(&fonts.wide, px, &format!("ENTERING  {}", name.to_uppercase()), (w - u(80.0)) as f32);
        let tw = text_width(&fonts.wide, px, &label) as i32;
        let bw = tw.max(u(260.0));
        let y = h / 2 + u(60.0);
        let plate = Rect { x: (w - bw) / 2 - u(18.0), y: y - u(30.0), w: bw + u(36.0), h: u(78.0) };
        self.fill(plate, [0.0, 0.012, 0.035], 0.85);
        self.frame(plate, CYAN, 0.6);
        self.text(&fonts.wide, px, ((w - tw) / 2) as f32, y as f32, &label, CYAN, 1.0);
        let bar = Rect { x: (w - bw) / 2, y: y + u(12.0), w: bw, h: u(6.0) };
        self.fill(bar, [0.0, 0.02, 0.05], 0.7);
        self.frame(bar, CYAN, 0.7);
        self.fill(Rect { x: bar.x, y: bar.y, w: (bar.w as f32 * p) as i32, h: bar.h }, CYAN, 0.95);
        let hint = "FLY ON THROUGH TO SKIP";
        let hw = text_width(&fonts.mono, 12.0 * s, hint) as i32;
        self.text(&fonts.mono, 12.0 * s, ((w - hw) / 2) as f32, (bar.y + u(24.0)) as f32, hint, CYAN, 0.7);
    }

    /// The frame around the YouTube player (a separate window on top, in the bottom-
    /// right corner): a monitor bezel with corner brackets, a label strip, and a
    /// faint glow on the floor below. The player itself is left exactly as it is.
    fn radio_bezel(&mut self, playing: bool, blink: bool, fonts: &Fonts) {
        const RED: [f32; 3] = [1.0, 0.22, 0.16];
        let s = self.scale;
        let u = |v: f32| (v * s).round() as i32;
        let (px, py, pw, ph) = crate::radio::panel(self.w, self.h, s as f64);
        let p = Rect { x: px, y: py, w: pw as i32, h: ph as i32 };
        let strip = u(24.0);
        let pad = u(6.0);
        let outer = Rect { x: p.x - pad, y: p.y - strip, w: p.w + 2 * pad, h: p.h + strip + pad };

        // Plate and frames.
        self.fill(outer, [0.0, 0.012, 0.035], 0.88);
        self.frame(outer, CYAN, 0.55);
        self.frame(Rect { x: p.x - 1, y: p.y - 1, w: p.w + 2, h: p.h + 2 }, DIM, 0.9);

        // Corner brackets, standing just off the frame.
        let (len, t, off) = (u(16.0), u(3.0).max(2), u(3.0));
        let (l, r) = (outer.x - off, outer.x + outer.w + off);
        let (top, bot) = (outer.y - off, outer.y + outer.h + off);
        for (cx, cy, sx, sy) in [(l, top, 1, 1), (r, top, -1, 1), (l, bot, 1, -1), (r, bot, -1, -1)] {
            let hx = if sx > 0 { cx } else { cx - len };
            let vy = if sy > 0 { cy } else { cy - len };
            self.fill(Rect { x: hx, y: if sy > 0 { cy } else { cy - t }, w: len, h: t }, CYAN, 0.95);
            self.fill(Rect { x: if sx > 0 { cx } else { cx - t }, y: vy, w: t, h: len }, CYAN, 0.95);
        }

        // The label strip: the name on the left, the state on the right.
        let small = 12.0 * s;
        let base = (outer.y + strip / 2) as f32 + small * 0.36;
        self.text(&fonts.mono, small, (outer.x + u(8.0)) as f32, base, "PIRATE RADIO", CYAN, 0.95);
        let (state, colour) = if playing { ("ON AIR", RED) } else { ("TUNING IN", DIM) };
        let sw = text_width(&fonts.mono, small, state);
        let sx = (outer.x + outer.w - u(8.0)) as f32 - sw;
        self.text(&fonts.mono, small, sx, base, state, colour, 0.95);
        if playing && blink || !playing {
            let d = u(7.0);
            let dot = Rect { x: sx as i32 - d - u(6.0), y: outer.y + (strip - d) / 2, w: d, h: d };
            self.fill(dot, colour, if playing { 1.0 } else { 0.6 });
        }
        // Tick marks along the strip's bottom edge, like a tuning dial.
        let ticks = 24;
        for k in 1..ticks {
            let x = p.x + p.w * k / ticks;
            let th = if k % 4 == 0 { u(4.0) } else { u(2.0) };
            self.fill(Rect { x, y: p.y - 1 - th, w: 1, h: th }, DIM, 0.7);
        }

        // A faint glow on the floor below, fading out.
        let below = outer.y + outer.h + u(2.0);
        for k in 0..((self.h as i32 - below).min(u(12.0))) {
            let a = 0.16 * (1.0 - k as f32 / u(12.0) as f32);
            self.fill(Rect { x: outer.x + u(10.0), y: below + k, w: outer.w - u(20.0), h: 1 }, CYAN, a);
        }
    }

    /// Opening the garbage file: a copy bar, and a log of what it's doing.
    fn hack_panel(&mut self, p: f32, done: bool, blink: bool, fonts: &Fonts) {
        const RED: [f32; 3] = [1.0, 0.22, 0.16];
        const TOTAL: u64 = 6_660_000;
        let s = self.scale;
        let u = |v: f32| (v * s).round() as i32;
        let (w, h) = (self.w as i32, self.h as i32);
        let pw = u(620.0).min(w - u(24.0));
        let ph = u(250.0);
        let r = Rect { x: (w - pw) / 2, y: (h - ph) / 2, w: pw, h: ph };
        self.fill(r, [0.04, 0.0, 0.005], 0.88);
        self.frame(r, RED, 0.95);
        self.fill(Rect { x: r.x, y: r.y, w: r.w, h: u(3.0) }, RED, 0.95);
        let x = (r.x + u(24.0)) as f32;
        let title = if done { "FILE COPIED" } else { "COPYING  GARBAGE" };
        if !done || blink {
            self.text(&fonts.wide, 24.0 * s, x, (r.y + u(44.0)) as f32, title, RED, 1.0);
        }
        // The bar: segmented, like an old terminal's.
        let bar = Rect { x: r.x + u(24.0), y: r.y + u(66.0), w: r.w - u(48.0), h: u(24.0) };
        self.frame(bar, RED, 0.9);
        let segs = 40;
        let sw = (bar.w - u(6.0)) / segs;
        let lit = (p * segs as f32).round() as i32;
        for k in 0..lit {
            self.fill(Rect { x: bar.x + u(3.0) + k * sw, y: bar.y + u(4.0), w: (sw - u(2.0)).max(1), h: bar.h - u(8.0) }, RED, 0.9);
        }
        let bytes = (p as f64 * TOTAL as f64) as u64;
        let line = format!("{} / {} BYTES   {:>3}%", grouped(bytes), grouped(TOTAL), (p * 100.0).floor() as u32);
        let mono = 14.0 * s;
        self.text(&fonts.mono, mono, x, (bar.y + bar.h + u(24.0)) as f32, &line, RED, 0.95);
        // A log of the copy: lines appear as it goes.
        const LOG: [&str; 9] = [
            "> HIDDEN SEGMENT 7F00 OPENED",
            "> TRACE ROUTE MASKED",
            "> READING 7F00:0000 .. 7F1A:FFFF",
            "> CHECKSUM MISMATCH, RETRYING",
            "> BLOCKS 0-511 OK",
            "> ALARM ON NODE 0x7731 IGNORED",
            "> BLOCKS 512-1023 OK",
            "> WRITING COPY TO LOCAL MEMORY",
            "> LOG ENTRIES ERASED",
        ];
        let shown = if done { LOG.len() } else { ((p * LOG.len() as f32) as usize + 1).min(LOG.len()) };
        let small = 13.0 * s;
        let first = shown.saturating_sub(4);
        for (k, l) in LOG[first..shown].iter().enumerate() {
            let y = bar.y + bar.h + u(52.0) + k as i32 * u(19.0);
            self.text(&fonts.mono, small, x, y as f32, l, RED, if first + k + 1 == shown { 0.95 } else { 0.55 });
        }
        if done {
            let t = "6,660,000 BYTES TAKEN.  NO TRACE LEFT.";
            let tw = text_width(&fonts.mono, small, t);
            self.text(&fonts.mono, small, (r.x + r.w) as f32 - tw - 24.0 * s, (r.y + r.h - u(18.0)) as f32, t, RED, 0.9);
        }
    }

    /// The viewer: title and effects along the top, the timeline at the bottom.
    fn viewer_overlay(&mut self, v: &ViewerHud, hot_hit: Option<usize>, fonts: &Fonts) {
        let s = self.scale;
        let u = |x: f32| (x * s).round() as i32;
        let (w, h) = (self.w as i32, self.h as i32);
        let mono = 15.0 * s;
        let small = 13.0 * s;

        // Title, centred under the bar.
        let top = self.bar_h + u(14.0);
        let big = 20.0 * s;
        let name = shorten(&fonts.wide, big, &v.name.to_uppercase(), (w - u(320.0)) as f32);
        let nw = text_width(&fonts.wide, big, &name);
        self.text(&fonts.wide, big, (w as f32 - nw) / 2.0, (top + u(22.0)) as f32, &name, CYAN, 1.0);
        let mut info = format!("{} / {}", v.index + 1, v.count);
        if let Some((pw, ph)) = v.size {
            info += &format!("   {pw} × {ph}");
        }
        let iw = text_width(&fonts.mono, small, &info);
        self.text(&fonts.mono, small, (w as f32 - iw) / 2.0, (top + u(42.0)) as f32, &info, DIM, 1.0);

        // Close button, top right under the bar.
        let cl = "CLOSE  ESC";
        let cw = text_width(&fonts.mono, small, cl).ceil() as i32;
        let cr = Rect { x: w - cw - u(30.0), y: top, w: cw + u(16.0), h: u(26.0) };
        let hot = hot_hit == Some(self.hits.len());
        if hot {
            self.fill(cr, CYAN, 0.22);
        }
        self.frame(cr, CYAN, 0.8);
        self.text(&fonts.mono, small, (cr.x + u(8.0)) as f32, (cr.y + u(18.0)) as f32, cl, CYAN, 1.0);
        self.hits.push((cr, HudAction::CloseViewer));

        // Effects: a row of buttons above the timeline, the current one lit.
        let mut x = u(16.0);
        let ey = h - u(98.0);
        self.text(&fonts.mono, small, x as f32, (ey + u(18.0)) as f32, "EFFECT  E", DIM, 1.0);
        x += u(84.0);
        for (i, name) in crate::viewer::EFFECTS.iter().enumerate() {
            let label = format!("{} {name}", i + 1);
            let tw = text_width(&fonts.mono, small, &label).ceil() as i32;
            let r = Rect { x, y: ey, w: tw + u(14.0), h: u(26.0) };
            let hot = hot_hit == Some(self.hits.len());
            if i == v.effect {
                self.fill(r, AMBER, 0.85);
            } else if hot {
                self.fill(r, AMBER, 0.2);
            }
            self.frame(r, AMBER, if i == v.effect || hot { 1.0 } else { 0.5 });
            self.text(&fonts.mono, small, (r.x + u(7.0)) as f32, (r.y + u(18.0)) as f32, &label, if i == v.effect { INK } else { AMBER }, 1.0);
            self.hits.push((r, HudAction::Effect(i)));
            x += r.w + u(6.0);
        }

        if let Some(e) = &v.error {
            let ew = text_width(&fonts.mono, mono, e);
            self.text(&fonts.mono, mono, (w as f32 - ew) / 2.0, h as f32 / 2.0, e, WARN, 1.0);
        }

        // Video: play / pause, timeline, times.
        let hint;
        if let Some((t, dur, paused)) = v.video {
            let y = h - u(52.0);
            let pb = Rect { x: u(16.0), y: y - u(14.0), w: u(30.0), h: u(28.0) };
            let hot = hot_hit == Some(self.hits.len());
            if hot {
                self.fill(pb, CYAN, 0.22);
            }
            self.frame(pb, CYAN, 0.8);
            let (cx, cy) = (pb.x + pb.w / 2, pb.y + pb.h / 2);
            if paused {
                // play triangle
                for dx in 0..u(12.0) {
                    let half = (u(12.0) - dx) / 2;
                    for dy in -half..=half {
                        self.blend(cx - u(5.0) + dx, cy + dy, CYAN, 1.0);
                    }
                }
            } else {
                self.fill(Rect { x: cx - u(6.0), y: cy - u(6.0), w: u(4.0), h: u(12.0) }, CYAN, 1.0);
                self.fill(Rect { x: cx + u(2.0), y: cy - u(6.0), w: u(4.0), h: u(12.0) }, CYAN, 1.0);
            }
            self.hits.push((pb, HudAction::PlayPause));

            let now = crate::viewer::clock(t);
            let total = crate::viewer::clock(dur);
            let tw = text_width(&fonts.mono, mono, &total).ceil() as i32;
            let x0 = pb.x + pb.w + u(16.0) + tw + u(12.0);
            let x1 = w - u(28.0) - tw;
            let bar = Rect { x: x0, y: y - u(3.0), w: (x1 - x0).max(10), h: u(6.0) };
            self.fill(bar, DIM, 0.45);
            let frac = if dur > 0.0 { (t / dur).clamp(0.0, 1.0) } else { 0.0 };
            let done = (bar.w as f32 * frac) as i32;
            self.fill(Rect { w: done, ..bar }, CYAN, 0.95);
            self.fill(Rect { x: bar.x + done - u(2.0), y: bar.y - u(6.0), w: u(4.0), h: bar.h + u(12.0) }, AMBER, 1.0);
            // tick every 10%
            for k in 1..10 {
                self.fill(Rect { x: bar.x + bar.w * k / 10, y: bar.y + bar.h + u(3.0), w: 1, h: u(4.0) }, DIM, 0.8);
            }
            self.text(&fonts.mono, mono, (pb.x + pb.w + u(12.0)) as f32, (y + u(5.0)) as f32, &now, CYAN, 1.0);
            self.text(&fonts.mono, mono, (x1 + u(14.0)) as f32, (y + u(5.0)) as f32, &total, DIM, 1.0);
            self.scrub = Some(Rect { y: bar.y - u(12.0), h: bar.h + u(24.0), ..bar });
            // Sound: volume bars (or MUTED / NO SOUND) above the timeline's right end.
            let label = match v.sound {
                Some((_, true)) => "MUTED  M".to_string(),
                Some((vol, false)) => format!("VOL {vol}%  M  + -"),
                None => "NO SOUND".to_string(),
            };
            let lw = text_width(&fonts.mono, small, &label);
            let lx = (w - u(28.0)) as f32 - lw;
            let col = if matches!(v.sound, Some((_, true)) | None) { DIM } else { CYAN };
            self.text(&fonts.mono, small, lx, (y - u(26.0)) as f32, &label, col, 1.0);
            if let Some((vol, false)) = v.sound {
                for k in 0..10 {
                    let on = (k as u32) < (vol + 5) / 10;
                    let bh = u(3.0) + k * u(1.0);
                    let r = Rect { x: lx as i32 - u(84.0) + k * u(7.0), y: y - u(26.0) - bh, w: u(5.0), h: bh };
                    self.fill(r, CYAN, if on { 0.95 } else { 0.25 });
                }
            }
            hint = "SPACE pause   LEFT / RIGHT 5 s (SHIFT 30 s)   DRAG timeline   M mute   + - volume   [ ] previous / next   E effect   O open in app   ESC close";
        } else {
            hint = "[ ] previous / next   E effect   1-6 pick an effect   O open in app   ESC close";
        }
        self.text(&fonts.mono, small, u(16.0) as f32, (h - u(12.0)) as f32, hint, DIM, 0.9);
    }

    /// The hotkey panel, centred, in two columns of sections.
    fn help_panel(&mut self, fonts: &Fonts) {
        let s = self.scale;
        let u = |v: f32| (v * s).round() as i32;
        let (w, h) = (self.w as i32, self.h as i32);
        let (key_px, txt_px, head_px) = (14.0 * s, 14.0 * s, 13.0 * s);
        let row = u(22.0);
        let key_col = u(230.0);
        let col_w = u(560.0);
        // Sections fill the left column, then the right.
        let rows_of = |sec: &(&str, &[(&str, &str)])| sec.1.len() as i32 + 2;
        let total: i32 = HELP.iter().map(rows_of).sum();
        let mut split = 0;
        let mut acc = 0;
        for (k, sec) in HELP.iter().enumerate() {
            if acc >= total / 2 {
                break;
            }
            acc += rows_of(sec);
            split = k + 1;
        }
        let left_rows = acc;
        let right_rows = total - acc;
        let pw = (col_w * 2 + u(60.0)).min(w - u(24.0));
        let ph = (left_rows.max(right_rows) * row + u(80.0)).min(h - self.bar_h - u(20.0));
        let r = Rect { x: (w - pw) / 2, y: self.bar_h + (h - self.bar_h - ph) / 2, w: pw, h: ph };
        self.fill(r, [0.0, 0.012, 0.035], 0.93);
        self.frame(r, CYAN, 0.9);
        self.fill(Rect { x: r.x, y: r.y, w: r.w, h: u(3.0) }, CYAN, 0.9);
        self.text(&fonts.wide, 20.0 * s, (r.x + u(22.0)) as f32, (r.y + u(38.0)) as f32, "HOTKEYS", CYAN, 1.0);
        let close = "?  F1  ESC  CLOSE";
        let cw = text_width(&fonts.mono, head_px, close);
        self.text(&fonts.mono, head_px, (r.x + r.w) as f32 - cw - 22.0 * s, (r.y + u(36.0)) as f32, close, DIM, 1.0);

        let draw_col = |hud: &mut Hud, secs: &[(&str, &[(&str, &str)])], x: i32| {
            let mut y = r.y + u(74.0);
            for (title, items) in secs {
                hud.text(&fonts.mono, head_px, x as f32, y as f32, title, DIM, 1.0);
                hud.fill(Rect { x, y: y + u(5.0), w: col_w - u(20.0), h: 1 }, DIM, 0.6);
                y += row;
                for (keys, what) in items.iter() {
                    hud.text(&fonts.mono, key_px, x as f32, y as f32, keys, AMBER, 1.0);
                    hud.text(&fonts.mono, txt_px, (x + key_col) as f32, y as f32, &what.to_uppercase(), CYAN, 0.9);
                    y += row;
                }
                y += row;
            }
        };
        draw_col(self, &HELP[..split], r.x + u(24.0));
        draw_col(self, &HELP[split..], r.x + u(24.0) + col_w + u(20.0));
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

/// The help panel: (section, [(keys, what they do)]).
const HELP: &[(&str, &[(&str, &str)])] = &[
    ("FLYING", &[
        ("W A S D  /  ARROWS", "fly forward, back, left, right"),
        ("E  /  SPACE", "up"),
        ("Q  /  CTRL", "down"),
        ("SHIFT", "fly 4x faster"),
        ("DRAG (EITHER BUTTON)", "look around"),
        ("MOUSE WHEEL", "flying speed"),
        ("P", "fly itself (any key stops)"),
        ("C  /  SHIFT + C", "colour scheme"),
        ("N", "music on / off: a file or YouTube link (see settings)"),
        ("L", "look: smoke (amount, colour, opacity, height), vignette, darkness, grain, CRT, glitch, softness"),
    ]),
    ("TOWERS", &[
        ("CLICK", "fly to a tower (selects it, amber)"),
        ("DOUBLE-CLICK", "open a folder, file or listed name"),
        ("ENTER", "go into the hovered tower"),
        ("R", "show it in Explorer"),
        ("ESC", "clear the selection"),
    ]),
    ("FOLDERS", &[
        ("BACKSPACE", "up to the parent folder"),
        ("ALT + LEFT  /  MOUSE BACK", "back to the previous folder"),
        ("TOP BAR", "click a drive or part of the path"),
        ("FLY INTO A FOLDER", "stay inside a moment: opens it"),
    ]),
    ("SEARCH", &[
        ("/  OR  CTRL + F", "search this folder by name"),
        ("ENTER", "fly to the best match, close"),
        ("SHIFT + ENTER  /  DOWN", "next match, keep searching"),
        ("ESC", "close the search"),
    ]),
    ("VIEWER (IMAGES, VIDEO)", &[
        ("V  /  DOUBLE-CLICK", "open the full picture or video"),
        ("SPACE  /  LEFT RIGHT", "pause, jump 5 s (shift: 30 s)"),
        ("E  /  1-6", "change the effect"),
        ("T  /  SHIFT + T", "effect on the towers' previews"),
        ("[  ]", "previous / next in the folder"),
        ("M  /  +  -", "mute, volume"),
    ]),
    ("HELP", &[("?  /  F1", "show or hide this")]),
];

/// Cuts `text` down with "…" until it fits `max_w`.
fn shorten(font: &Font, px: f32, text: &str, max_w: f32) -> String {
    if text_width(font, px, text) <= max_w {
        return text.to_string();
    }
    let mut v: Vec<char> = text.chars().collect();
    while !v.is_empty() && text_width(font, px, &(v.iter().collect::<String>() + "…")) > max_w {
        v.pop();
    }
    v.into_iter().collect::<String>() + "…"
}

/// A colour round the wheel (0..1), as the smoke shader makes it.
fn hue_rgb(h: f32) -> [f32; 3] {
    let f = |k: f32| (((h + k).fract() * 6.0 - 3.0).abs() - 1.0).clamp(0.0, 1.0) * 0.8 + 0.2;
    [f(0.0), f(2.0 / 3.0), f(1.0 / 3.0)]
}

fn grouped(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn small_px(scale: f32) -> f32 {
    13.0 * scale
}

fn text_width(font: &Font, px: f32, text: &str) -> f32 {
    text.chars().map(|c| font.metrics(c, px).advance_width).sum()
}

/// "C:\" → "C:", "/" → "/", the home folder → "HOME", "/Volumes/Data" → "DATA".
fn drive_label(d: &Path) -> String {
    if std::env::var_os("HOME").is_some_and(|h| Path::new(&h) == d) && !cfg!(windows) {
        return "HOME".into();
    }
    if d.parent().is_some_and(|p| p == Path::new("/Volumes") || p == Path::new("/media") || p == Path::new("/mnt")) {
        return d.file_name().map(|n| n.to_string_lossy().to_uppercase()).unwrap_or_default();
    }
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
