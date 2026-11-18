// Bakes each tower's face texture: a greyscale "mosaic" of the entry's real data —
// its name, folder contents or file text, a hex dump of the file's actual bytes —
// laid out in blocks like the film's glass towers. The shader tints it.

use crate::city::Rng;
use crate::fs_scan::{format_date, format_size, Entry};
use fontdue::{Font, FontSettings};

pub const TEX_W: u32 = 512;
pub const TEX_H: u32 = 1024;
pub const MIP_LEVELS: u32 = 11; // 512x1024 → 1x1 (height drives the count)

pub struct Fonts {
    pub wide: Font, // Michroma — big wide headings, like the film's directory names
    pub mono: Font, // Share Tech Mono — data text
}

impl Fonts {
    pub fn load() -> Fonts {
        let wide = Font::from_bytes(
            include_bytes!("../assets/fonts/Michroma-Regular.ttf") as &[u8],
            FontSettings::default(),
        )
        .expect("Michroma font");
        let mono = Font::from_bytes(
            include_bytes!("../assets/fonts/ShareTechMono-Regular.ttf") as &[u8],
            FontSettings::default(),
        )
        .expect("Share Tech Mono font");
        Fonts { wide, mono }
    }
}

#[derive(Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

struct Canvas {
    px: Vec<u8>,
}

impl Canvas {
    fn new() -> Canvas {
        Canvas { px: vec![0; (TEX_W * TEX_H) as usize] }
    }
    fn plot(&mut self, x: i32, y: i32, v: u8, clip: Rect, invert: bool) {
        if x < clip.x || y < clip.y || x >= clip.x + clip.w || y >= clip.y + clip.h {
            return;
        }
        if x < 0 || y < 0 || x >= TEX_W as i32 || y >= TEX_H as i32 {
            return;
        }
        let p = &mut self.px[(y as u32 * TEX_W + x as u32) as usize];
        *p = if invert { p.saturating_sub(v) } else { (*p).max(v) };
    }
    fn fill(&mut self, r: Rect, v: u8) {
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                self.plot(x, y, v, r, false);
            }
        }
    }
    fn frame(&mut self, r: Rect, v: u8) {
        for x in r.x..r.x + r.w {
            self.plot(x, r.y, v, r, false);
            self.plot(x, r.y + r.h - 1, v, r, false);
        }
        for y in r.y..r.y + r.h {
            self.plot(r.x, y, v, r, false);
            self.plot(r.x + r.w - 1, y, v, r, false);
        }
    }
    /// Fills `r` with one of a few greyscale patterns, so types tell apart without colour.
    fn pattern(&mut self, r: Rect, style: usize, v: u8) {
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                let (lx, ly) = (x - r.x, y - r.y);
                let on = match style % 6 {
                    0 => true,                              // solid
                    1 => (lx + ly) % 6 < 2,                 // diagonal hatch
                    2 => lx % 2 == 0,                       // fine vertical lines
                    3 => (lx / 3 + ly / 3) % 2 == 0,        // checker
                    4 => ly % 3 == 0,                       // horizontal lines
                    _ => lx % 4 < 2 && ly % 4 < 2,          // dots
                };
                if on {
                    self.plot(x, y, v, r, false);
                }
            }
        }
    }

    /// Right-pointing triangle marker (the fonts don't have ▸).
    fn marker(&mut self, x: i32, y_mid: i32, size: i32, v: u8, clip: Rect) {
        for dx in 0..size {
            let half = (size - dx) / 2;
            for dy in -half..=half {
                self.plot(x + dx, y_mid + dy, v, clip, false);
            }
        }
    }
}

fn text_width(font: &Font, px: f32, text: &str) -> f32 {
    text.chars().map(|c| font.metrics(c, px).advance_width).sum()
}

thread_local! {
    // Rasterised glyphs, per thread: (font, char, size) → bitmap. Pages and the HUD
    // draw the same few hundred glyphs over and over.
    static GLYPHS: std::cell::RefCell<std::collections::HashMap<(usize, char, u32), std::rc::Rc<(fontdue::Metrics, Vec<u8>)>>> =
        Default::default();
}

/// A glyph's metrics and coverage bitmap, rasterised once per thread.
pub fn glyph(font: &Font, ch: char, px: f32) -> std::rc::Rc<(fontdue::Metrics, Vec<u8>)> {
    let key = (font as *const Font as usize, ch, px.to_bits());
    GLYPHS.with(|g| {
        let mut g = g.borrow_mut();
        if g.len() > 20_000 {
            g.clear();
        }
        g.entry(key).or_insert_with(|| std::rc::Rc::new(font.rasterize(ch, px))).clone()
    })
}

/// Draws `text` with its baseline at `y`; returns the end x.
fn draw_text(c: &mut Canvas, font: &Font, px: f32, x: f32, y: f32, text: &str, v: u8, clip: Rect, invert: bool) -> f32 {
    let mut pen = x;
    for ch in text.chars() {
        if pen > (clip.x + clip.w) as f32 {
            break;
        }
        let gl = glyph(font, ch, px);
        let (m, bmp) = (&gl.0, &gl.1);
        let gx = pen as i32 + m.xmin;
        let gy = y as i32 - m.height as i32 - m.ymin;
        for row in 0..m.height {
            for col in 0..m.width {
                let a = bmp[row * m.width + col];
                if a > 0 {
                    c.plot(gx + col as i32, gy + row as i32, ((a as u32 * v as u32) / 255) as u8, clip, invert);
                }
            }
        }
        pen += m.advance_width;
    }
    pen
}

/// Shortens `text` with "…" until it fits `max_w`.
fn fit(font: &Font, px: f32, text: &str, max_w: f32) -> String {
    if text_width(font, px, text) <= max_w {
        return text.to_string();
    }
    let mut s: Vec<char> = text.chars().collect();
    while !s.is_empty() && text_width(font, px, &(s.iter().collect::<String>() + "…")) > max_w {
        s.pop();
    }
    s.into_iter().collect::<String>() + "…"
}

/// Where a listed name / line sits on a page (texture pixels), for clicking names.
#[derive(Clone, Copy, Debug)]
pub struct LineHit {
    pub x: i16,
    pub y: i16,
    pub w: i16,
    pub h: i16,
    pub line: u32, // index into the entry's `lines`
}

/// One baked face page: its mip chain, and where its names are.
pub struct Page {
    pub mips: Vec<u8>,
    pub hits: Vec<LineHit>,
}

/// Where we are in the entry's real content; carries on from page to page.
struct Walk {
    line: usize, // next name / text line
    hex: usize,  // next byte offset for hex dumps
    wrap: bool,  // single-page tower: loop the content to fill the page
    hits: Vec<LineHit>, // names drawn on the current page
}

impl Walk {
    fn lines_left(&self, e: &Entry) -> bool {
        self.line < e.lines.len()
    }
    fn hex_left(&self, e: &Entry) -> bool {
        self.hex < e.head.len()
    }
    /// All the real content has been shown (hex is only filler when there are lines).
    fn done(&self, e: &Entry) -> bool {
        if !e.lines.is_empty() { !self.lines_left(e) } else { !self.hex_left(e) }
    }
    fn hit(&mut self, x: i32, y: i32, w: i32, h: i32, line: usize) {
        self.hits.push(LineHit { x: x as i16, y: y as i16, w: w as i16, h: h as i16, line: line as u32 });
    }
}

/// Bakes up to `max_pages` face pages (each a full mip chain). Page 0 is the tower's
/// front page; later pages continue through the content where the last one stopped.
pub fn bake(e: &Entry, seed: f32, fonts: &Fonts, max_pages: u32) -> Vec<Page> {
    let mut rng = Rng(seed.to_bits() as u64 * 0x2545_F491_4F6C_DD1D + 7);
    let mut walk = Walk { line: 0, hex: 0, wrap: max_pages <= 1, hits: Vec::new() };
    let mut pages = Vec::new();
    for p in 0..max_pages.max(1) {
        let mut c = Canvas::new();
        if p == 0 {
            front_page(&mut c, e, &mut rng, &mut walk, fonts);
        } else {
            next_page(&mut c, e, &mut rng, &mut walk, fonts);
        }
        pages.push(Page { mips: mip_chain(&c.px), hits: std::mem::take(&mut walk.hits) });
        if walk.done(e) {
            break;
        }
    }
    pages
}

fn front_page(c: &mut Canvas, e: &Entry, rng: &mut Rng, walk: &mut Walk, fonts: &Fonts) {
    let full = Rect { x: 0, y: 0, w: TEX_W as i32, h: TEX_H as i32 };
    let m = 14; // margin

    // ── Header: the name, big and wide, and a status line ──────────────────
    let name = e.name.to_uppercase();
    let mut px = 34.0;
    while px > 15.0 && text_width(&fonts.wide, px, &name) > (TEX_W as i32 - 2 * m) as f32 {
        px -= 1.0;
    }
    let name = fit(&fonts.wide, px, &name, (TEX_W as i32 - 2 * m) as f32);
    draw_text(c, &fonts.wide, px, m as f32, 50.0, &name, 255, full, false);

    let status = if e.is_dir {
        format!(
            "DIR  {}{} ITEMS  {}  {}",
            e.child_count,
            if e.capped { "+" } else { "" },
            format_size(e.size),
            format_date(e.modified)
        )
    } else {
        format!("FILE  {}  {}", format_size(e.size), format_date(e.modified))
    };
    draw_text(c, &fonts.mono, 17.0, m as f32, 80.0, &status, 190, full, false);
    c.fill(Rect { x: m, y: 92, w: TEX_W as i32 - 2 * m, h: 2 }, 220);

    // ── Hero list (folders): child names right-aligned with ▸ markers ──────
    let mut y = 104;
    if e.is_dir && !e.lines.is_empty() {
        let list = Rect { x: m, y, w: TEX_W as i32 - 2 * m, h: 300 };
        let mut ly = y as f32 + 22.0;
        for (i, (text, is_dir)) in e.lines.iter().take(12).enumerate() {
            walk.hit(list.x, ly as i32 - 18, list.w, 24, i);
            let t = fit(&fonts.wide, 15.0, &text.to_uppercase(), (list.w - 40) as f32);
            let w = text_width(&fonts.wide, 15.0, &t);
            let right = (list.x + list.w - 22) as f32;
            draw_text(c, &fonts.wide, 15.0, right - w, ly, &t, if *is_dir { 255 } else { 175 }, list, false);
            c.marker(right as i32 + 6, ly as i32 - 6, 10, 230, list);
            ly += 24.0;
        }
        y = ly as i32 + 4;
        c.fill(Rect { x: m, y, w: TEX_W as i32 - 2 * m, h: 1 }, 150);
        y += 8;
    }

    // ── File types (folders): what the folder is made of ────────────────────
    if e.is_dir && !e.types.is_empty() {
        y = type_graph(c, e, Rect { x: m, y, w: TEX_W as i32 - 2 * m, h: 0 }, fonts);
        c.fill(Rect { x: m, y, w: TEX_W as i32 - 2 * m, h: 1 }, 150);
        y += 8;
    }

    // ── Mosaic body: blocks of real data ────────────────────────────────────
    let body = Rect { x: m, y, w: TEX_W as i32 - 2 * m, h: TEX_H as i32 - y - m };
    mosaic(c, body, e, rng, walk, fonts, 0.5);
}

/// A stylised breakdown of the folder's files by type: a stacked ribbon of shares,
/// then one row per type with a patterned swatch, a segmented meter and the numbers.
/// Shares are by bytes (by count if the files are all empty). Returns the y below it.
fn type_graph(c: &mut Canvas, e: &Entry, area: Rect, fonts: &Fonts) -> i32 {
    let full = Rect { x: 0, y: 0, w: TEX_W as i32, h: TEX_H as i32 };
    let total_bytes: u64 = e.types.iter().map(|t| t.bytes).sum();
    let total_count: usize = e.types.iter().map(|t| t.count).sum();
    let share = |t: &crate::fs_scan::TypeStat| {
        if total_bytes > 0 { t.bytes as f32 / total_bytes as f32 } else { t.count as f32 / total_count.max(1) as f32 }
    };
    let mut y = area.y + 16;

    let title = if total_bytes > 0 { "FILE TYPES  /  BY SIZE" } else { "FILE TYPES  /  BY COUNT" };
    draw_text(c, &fonts.mono, 14.0, area.x as f32, y as f32, title, 200, full, false);
    let files = format!("{total_count} FILES");
    let fw = text_width(&fonts.mono, 14.0, &files);
    draw_text(c, &fonts.mono, 14.0, (area.x + area.w) as f32 - fw, y as f32, &files, 200, full, false);
    y += 8;

    // Stacked ribbon: every type's share side by side, framed.
    let ribbon = Rect { x: area.x, y, w: area.w, h: 20 };
    c.frame(ribbon, 210);
    let inner_w = ribbon.w - 4;
    let mut x = ribbon.x + 2;
    for (i, t) in e.types.iter().enumerate() {
        let last = i + 1 == e.types.len();
        let w = if last { ribbon.x + 2 + inner_w - x } else { (share(t) * inner_w as f32).round() as i32 };
        if w > 2 {
            c.pattern(Rect { x, y: ribbon.y + 3, w: w - 2, h: ribbon.h - 6 }, i, 235);
        }
        x += w;
    }
    // Tick marks every 10% under the ribbon.
    for k in 0..=10 {
        let tx = ribbon.x + ribbon.w * k / 10;
        c.fill(Rect { x: tx.min(ribbon.x + ribbon.w - 1), y: ribbon.y + ribbon.h + 2, w: 1, h: if k % 5 == 0 { 6 } else { 3 } }, 170);
    }
    y = ribbon.y + ribbon.h + 14;

    // One row per type: swatch, label, segmented meter, count · size.
    let label_w = 64;
    let num_w = 150;
    let meter = Rect { x: area.x + 22 + label_w, y: 0, w: area.w - 22 - label_w - num_w - 8, h: 10 };
    let max_share = e.types.iter().map(|t| share(t)).fold(0.0f32, f32::max).max(1e-6);
    for (i, t) in e.types.iter().enumerate() {
        let row_mid = y + 7;
        c.frame(Rect { x: area.x, y: row_mid - 7, w: 14, h: 14 }, 220);
        c.pattern(Rect { x: area.x + 2, y: row_mid - 5, w: 10, h: 10 }, i, 235);
        let label = fit(&fonts.mono, 14.0, &t.label, label_w as f32 - 4.0);
        draw_text(c, &fonts.mono, 14.0, (area.x + 20) as f32, (row_mid + 5) as f32, &label, 240, full, false);

        // Meter: dim slots for the full scale, lit segments for this type's share.
        let seg = 5;
        let slots = meter.w / seg;
        let lit = ((share(t) / max_share) * slots as f32).ceil() as i32;
        for k in 0..slots {
            let r = Rect { x: meter.x + k * seg, y: row_mid - 4, w: seg - 2, h: 9 };
            if k < lit {
                c.fill(r, 235);
            } else {
                c.fill(Rect { y: row_mid, h: 1, ..r }, 90);
            }
        }

        let pct = (share(t) * 100.0).round() as u32;
        let nums = format!("{:>3}%  {:>4}  {}", pct, t.count, format_size(t.bytes));
        let nums = fit(&fonts.mono, 13.0, &nums, num_w as f32);
        let nw = text_width(&fonts.mono, 13.0, &nums);
        draw_text(c, &fonts.mono, 13.0, (area.x + area.w) as f32 - nw, (row_mid + 5) as f32, &nums, 200, full, false);
        y += 20;
    }
    y + 4
}

/// A continuation page: a slim header saying which part of the content this is,
/// then a mosaic that mostly carries on through the real listing.
fn next_page(c: &mut Canvas, e: &Entry, rng: &mut Rng, walk: &mut Walk, fonts: &Fonts) {
    let full = Rect { x: 0, y: 0, w: TEX_W as i32, h: TEX_H as i32 };
    let m = 14;
    let (line0, hex0) = (walk.line, walk.hex);

    let body = Rect { x: m, y: 70, w: TEX_W as i32 - 2 * m, h: TEX_H as i32 - 70 - m };
    mosaic(c, body, e, rng, walk, fonts, 0.75);

    // Header drawn after the body, so it can say what the page showed.
    let name = fit(&fonts.wide, 22.0, &e.name.to_uppercase(), (TEX_W as i32 - 2 * m) as f32);
    draw_text(c, &fonts.wide, 22.0, m as f32, 34.0, &name, 235, full, false);
    let range = if !e.lines.is_empty() {
        let what = if e.is_dir { "ITEMS" } else { "LINES" };
        let total = if e.is_dir { e.child_count.max(e.lines.len()) } else { e.lines.len() };
        format!("{what} {}-{} OF {total}", line0 + 1, walk.line.min(e.lines.len()))
    } else {
        format!("BYTES {hex0:06X}-{:06X} OF {:06X}", walk.hex.min(e.head.len()), e.head.len())
    };
    draw_text(c, &fonts.mono, 16.0, (m + 16) as f32, 56.0, &format!("CONT.  {range}"), 190, full, false);
    c.marker(m, 51, 10, 220, full);
    c.fill(Rect { x: m, y: 64, w: TEX_W as i32 - 2 * m, h: 2 }, 220);
}

fn mosaic(c: &mut Canvas, body: Rect, e: &Entry, rng: &mut Rng, walk: &mut Walk, fonts: &Fonts, list_bias: f32) {
    let mut blocks = Vec::new();
    split(body, rng, 0, &mut blocks);
    for b in blocks {
        mosaic_block(c, b, e, rng, walk, fonts, list_bias);
    }
}

// Guillotine-split a rectangle into a mosaic of blocks.
fn split(r: Rect, rng: &mut Rng, depth: u32, out: &mut Vec<Rect>) {
    let can_v = r.w > 200;
    let can_h = r.h > 140;
    if depth > 5 || (!can_v && !can_h) || (depth > 1 && rng.f32() < 0.18) {
        out.push(r);
        return;
    }
    let vertical = can_v && (!can_h || rng.f32() < r.w as f32 / (r.w + r.h) as f32);
    let gap = 6;
    if vertical {
        let at = rng.range(r.w * 3 / 10, r.w * 7 / 10);
        split(Rect { w: at - gap / 2, ..r }, rng, depth + 1, out);
        split(Rect { x: r.x + at + gap / 2, w: r.w - at - gap / 2, ..r }, rng, depth + 1, out);
    } else {
        let at = rng.range(r.h * 3 / 10, r.h * 7 / 10);
        split(Rect { h: at - gap / 2, ..r }, rng, depth + 1, out);
        split(Rect { y: r.y + at + gap / 2, h: r.h - at - gap / 2, ..r }, rng, depth + 1, out);
    }
}

fn mosaic_block(c: &mut Canvas, b: Rect, e: &Entry, rng: &mut Rng, walk: &mut Walk, fonts: &Fonts, list_bias: f32) {
    let framed = rng.f32() < 0.4;
    let invert = rng.f32() < 0.1;
    let level = 150 + (rng.f32() * 105.0) as u8;
    if invert {
        c.fill(b, level);
    }
    if framed {
        c.frame(b, 200);
    }
    let inner = Rect { x: b.x + 5, y: b.y + 4, w: b.w - 10, h: b.h - 8 };

    // Real data first: names / text lines, or the file's hex dump; then filler kinds.
    // Once a multi-page tower has shown everything, the rest of the page is filler.
    let has_lines = !e.lines.is_empty() && (walk.wrap || walk.lines_left(e));
    let has_head = !e.head.is_empty() && (walk.wrap || !e.lines.is_empty() || walk.hex_left(e));
    let pick = rng.f32();
    let kind = if has_lines && pick < list_bias {
        0
    } else if has_head && pick < list_bias + 0.3 {
        1
    } else {
        2 + (rng.f32() * 3.0) as u32
    };

    let px = if inner.w > 260 { 13.0 } else { 11.0 };
    let line_h = px + 2.0;
    let rows = ((inner.h as f32) / line_h) as usize;
    let mut y = inner.y as f32 + px;
    match kind {
        0 => {
            for _ in 0..rows {
                if walk.line >= e.lines.len() && !walk.wrap {
                    break;
                }
                let idx = walk.line % e.lines.len();
                walk.hit(inner.x, (y - px) as i32, inner.w, line_h.ceil() as i32, idx);
                let (t, is_dir) = &e.lines[idx];
                walk.line += 1;
                let t = if *is_dir { t.to_uppercase() + " >" } else { t.clone() };
                let v = if *is_dir { level } else { level.saturating_sub(40) };
                draw_text(c, &fonts.mono, px, inner.x as f32, y, &t, v, inner, invert);
                y += line_h;
            }
        }
        1 => {
            // Hex dump of the real bytes: offset, hex, ascii (as many columns as fit)
            let cw = fonts.mono.metrics('0', px).advance_width;
            let per = (((inner.w as f32 / cw) - 6.0) / 4.0).clamp(2.0, 16.0) as usize;
            // Hex walks on through the file; it only loops when it's filler.
            let loops = walk.wrap || !e.lines.is_empty();
            for _ in 0..rows {
                if walk.hex >= e.head.len() {
                    if !loops {
                        break;
                    }
                    walk.hex = 0;
                }
                let off = walk.hex;
                let chunk = &e.head[off..(off + per).min(e.head.len())];
                let hex: String = chunk.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ");
                let asc: String = chunk.iter().map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' }).collect();
                let line = format!("{off:04X} {hex:<w$} {asc}", w = per * 3 - 1);
                draw_text(c, &fonts.mono, px, inner.x as f32, y, &line, level, inner, invert);
                walk.hex += per;
                y += line_h;
            }
        }
        2 => {
            // Numeric columns
            for _ in 0..rows {
                let line = format!(
                    "{:06} {:>4}.{:02} {:08X}",
                    rng.next() % 1_000_000,
                    rng.next() % 9999,
                    rng.next() % 100,
                    rng.next() as u32
                );
                draw_text(c, &fonts.mono, px, inner.x as f32, y, &line, level, inner, invert);
                y += line_h;
            }
        }
        3 => {
            // Bar chart glyphs
            let bars = (inner.h / 9).max(1);
            for i in 0..bars {
                let w = (rng.f32() * inner.w as f32) as i32;
                c.fill(Rect { x: inner.x, y: inner.y + i * 9, w, h: 5 }, level);
            }
        }
        _ => {
            // One big word from the entry, plus a code line under it
            let word = e
                .lines
                .get(rng.next() as usize % e.lines.len().max(1))
                .map(|l| l.0.to_uppercase())
                .unwrap_or_else(|| e.name.to_uppercase());
            let word = fit(&fonts.wide, 14.0, &word, inner.w as f32);
            draw_text(c, &fonts.wide, 14.0, inner.x as f32, inner.y as f32 + 18.0, &word, 255, inner, invert);
            let code = format!("{:04X}-{:04X} SEC {:02}", rng.next() as u16, rng.next() as u16, rng.next() % 64);
            draw_text(c, &fonts.mono, 11.0, inner.x as f32, inner.y as f32 + 34.0, &code, level, inner, invert);
        }
    }
}

/// Full mip chain, level 0 first, concatenated.
fn mip_chain(base: &[u8]) -> Vec<u8> {
    let mut out = base.to_vec();
    let (mut w, mut h) = (TEX_W as usize, TEX_H as usize);
    let mut prev = base.to_vec();
    for _ in 1..MIP_LEVELS {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; nw * nh];
        for y in 0..nh {
            for x in 0..nw {
                let s = |dx: usize, dy: usize| prev[((y * 2 + dy).min(h - 1)) * w + (x * 2 + dx).min(w - 1)] as u32;
                next[y * nw + x] = ((s(0, 0) + s(1, 0) + s(0, 1) + s(1, 1) + 2) / 4) as u8;
            }
        }
        out.extend_from_slice(&next);
        prev = next;
        w = nw;
        h = nh;
    }
    out
}

/// Bytes in one page's full mip chain.
pub fn mips_len() -> usize {
    (0..MIP_LEVELS).map(|l| (mip_size(l).0 * mip_size(l).1) as usize).sum()
}

pub fn mip_size(level: u32) -> (u32, u32) {
    ((TEX_W >> level).max(1), (TEX_H >> level).max(1))
}
