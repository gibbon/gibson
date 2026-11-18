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
    wide: Font, // Michroma — big wide headings, like the film's directory names
    mono: Font, // Share Tech Mono — data text
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

/// Draws `text` with its baseline at `y`; returns the end x.
fn draw_text(c: &mut Canvas, font: &Font, px: f32, x: f32, y: f32, text: &str, v: u8, clip: Rect, invert: bool) -> f32 {
    let mut pen = x;
    for ch in text.chars() {
        if pen > (clip.x + clip.w) as f32 {
            break;
        }
        let (m, bmp) = font.rasterize(ch, px);
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

pub fn bake(e: &Entry, seed: f32, fonts: &Fonts) -> Vec<u8> {
    let mut c = Canvas::new();
    let mut rng = Rng(seed.to_bits() as u64 * 0x2545_F491_4F6C_DD1D + 7);
    let full = Rect { x: 0, y: 0, w: TEX_W as i32, h: TEX_H as i32 };
    let m = 14; // margin

    // ── Header: the name, big and wide, and a status line ──────────────────
    let name = e.name.to_uppercase();
    let mut px = 34.0;
    while px > 15.0 && text_width(&fonts.wide, px, &name) > (TEX_W as i32 - 2 * m) as f32 {
        px -= 1.0;
    }
    let name = fit(&fonts.wide, px, &name, (TEX_W as i32 - 2 * m) as f32);
    draw_text(&mut c, &fonts.wide, px, m as f32, 50.0, &name, 255, full, false);

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
    draw_text(&mut c, &fonts.mono, 17.0, m as f32, 80.0, &status, 190, full, false);
    c.fill(Rect { x: m, y: 92, w: TEX_W as i32 - 2 * m, h: 2 }, 220);

    // ── Hero list (folders): child names right-aligned with ▸ markers ──────
    let mut y = 104;
    if e.is_dir && !e.lines.is_empty() {
        let list = Rect { x: m, y, w: TEX_W as i32 - 2 * m, h: 300 };
        let mut ly = y as f32 + 22.0;
        for (text, is_dir) in e.lines.iter().take(12) {
            let t = fit(&fonts.wide, 15.0, &text.to_uppercase(), (list.w - 40) as f32);
            let w = text_width(&fonts.wide, 15.0, &t);
            let right = (list.x + list.w - 22) as f32;
            draw_text(&mut c, &fonts.wide, 15.0, right - w, ly, &t, if *is_dir { 255 } else { 175 }, list, false);
            c.marker(right as i32 + 6, ly as i32 - 6, 10, 230, list);
            ly += 24.0;
        }
        y = ly as i32 + 4;
        c.fill(Rect { x: m, y, w: TEX_W as i32 - 2 * m, h: 1 }, 150);
        y += 8;
    }

    // ── Mosaic body: blocks of real data ────────────────────────────────────
    let body = Rect { x: m, y, w: TEX_W as i32 - 2 * m, h: TEX_H as i32 - y - m };
    let mut blocks = Vec::new();
    split(body, &mut rng, 0, &mut blocks);
    let mut cursor = 0usize; // walks through e.lines across blocks
    for b in blocks {
        mosaic_block(&mut c, b, e, &mut rng, &mut cursor, fonts);
    }

    mip_chain(&c.px)
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

fn mosaic_block(c: &mut Canvas, b: Rect, e: &Entry, rng: &mut Rng, cursor: &mut usize, fonts: &Fonts) {
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
    let has_lines = !e.lines.is_empty();
    let has_head = !e.head.is_empty();
    let pick = rng.f32();
    let kind = if has_lines && pick < 0.5 {
        0
    } else if has_head && pick < 0.8 {
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
                let (t, is_dir) = &e.lines[*cursor % e.lines.len()];
                *cursor += 1;
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
            let mut off = (rng.next() as usize % e.head.len().max(1)) / per * per;
            for _ in 0..rows {
                if off >= e.head.len() {
                    off = 0;
                }
                let chunk = &e.head[off..(off + per).min(e.head.len())];
                let hex: String = chunk.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ");
                let asc: String = chunk.iter().map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' }).collect();
                let line = format!("{off:04X} {hex:<w$} {asc}", w = per * 3 - 1);
                draw_text(c, &fonts.mono, px, inner.x as f32, y, &line, level, inner, invert);
                off += per;
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

pub fn mip_size(level: u32) -> (u32, u32) {
    ((TEX_W >> level).max(1), (TEX_H >> level).max(1))
}
