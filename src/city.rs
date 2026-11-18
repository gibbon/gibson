// Lays a folder's entries out as a grid of glass towers with streets between them.

use crate::fs_scan::Entry;
use glam::{Vec2, Vec3};

pub const CELL: f32 = 26.0; // tower spacing; streets run along the cell boundaries
pub const LAYER_BUDGET: u32 = 768; // face pages per city (~0.7 MB of GPU memory each)
// Rough content per page, used to reserve pages (baking stops early if content runs out).
const LINES_PER_PAGE: usize = 80;
const HEX_BYTES_PER_PAGE: usize = 600;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Folder,
    Text,
    Media,
    Binary,
    Other,
}

pub struct Tower {
    pub entry: Entry,
    pub base: Vec3, // centre of the footprint at ground level
    pub size: Vec3, // width, height, depth
    pub kind: Kind,
    pub seed: f32,
    pub first_layer: u32,   // reserved texture-array layers: first_layer .. first_layer + max_pages
    pub max_pages: u32,
    pub pages: u32,         // pages actually baked
    pub layer: Option<u32>, // = Some(first_layer) once baked + uploaded
    pub appear: f32,        // 0 → 1 fade-in after the texture arrives
    pub hover: f32,         // eased highlight
    pub select: f32,        // eased "selected" tint
    pub found: f32,         // eased search match (1 name, 0.5 contents)
    pub dim: f32,           // eased: searching and this doesn't match
    pub page_hits: Vec<Vec<crate::textgen::LineHit>>, // where names sit on each page
    pub pv_kind: Option<crate::preview::PvKind>, // image / video file
    pub pv_layer: Option<u32>,          // reserved preview layer
    pub pv_size: Option<(u32, u32)>,    // preview picture size, once uploaded
    pub pv_video: bool,
    pub pv_duration: f32,               // video length in seconds (0 if unknown)
    pub pv_cut: f32,                    // video: 1 for a moment at each montage cut
    pub pv_still: Option<std::sync::Arc<crate::preview::Preview>>, // shown when not playing
}

impl Tower {
    pub fn min(&self) -> Vec3 {
        Vec3::new(self.base.x - self.size.x / 2.0, 0.0, self.base.z - self.size.z / 2.0)
    }
    pub fn max(&self) -> Vec3 {
        Vec3::new(self.base.x + self.size.x / 2.0, self.size.y, self.base.z + self.size.z / 2.0)
    }

    /// Pages worth baking: enough to cover all four faces (a page is one face wide and
    /// two faces wide tall), but no more than the real content fills.
    fn wanted_pages(&self) -> u32 {
        let per_face = (self.size.y / (self.size.x * 2.0) - 0.001).ceil().max(1.0) as u32;
        let e = &self.entry;
        let content = if !e.lines.is_empty() {
            e.lines.len().div_ceil(LINES_PER_PAGE)
        } else {
            e.head.len().div_ceil(HEX_BYTES_PER_PAGE)
        };
        (content as u32).clamp(1, per_face * 4)
    }
}

pub struct City {
    pub towers: Vec<Tower>,
    pub cols: usize,
    pub rows: usize,
    pub origin: Vec2, // world x,z of cell (0,0) centre
    pub half_extent: Vec2,
}

impl City {
    /// Texture-array layers this city needs.
    pub fn layers(&self) -> u32 {
        self.towers.iter().map(|t| t.max_pages).sum()
    }

    pub fn build(entries: Vec<Entry>, layer_budget: u32) -> City {
        let n = entries.len().max(1);
        let cols = ((n as f32 * 0.8).sqrt().ceil() as usize).max(1);
        let rows = n.div_ceil(cols);
        let origin = Vec2::new(-((cols - 1) as f32) * CELL / 2.0, -((rows - 1) as f32) * CELL / 2.0);
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ n as u64);

        let mut towers: Vec<Tower> = entries
            .into_iter()
            .enumerate()
            .map(|(i, e)| {
                let (col, row) = (i % cols, i / cols);
                let (w, h) = if e.is_dir {
                    // Tall square prisms, ~3–5:1; height grows with how much is inside.
                    let items = e.child_count as f32 + if e.capped { 2000.0 } else { 0.0 };
                    let l = (1.0 + items).log2();
                    let w = 8.0 + (l * 0.45).min(4.0);
                    (w, w * (2.4 + (l * 0.32).min(3.2)))
                } else {
                    let kb = e.size as f32 / 1024.0;
                    let w = 5.0 + ((1.0 + kb).log10() * 0.8).min(3.0);
                    (w, w * (1.1 + ((1.0 + kb / 10.0).log10() * 0.8).min(3.0)))
                };
                let pv_kind = if e.is_dir { None } else { crate::preview::kind(&e.path) };
                let jitter = Vec3::new(rng.f32() - 0.5, 0.0, rng.f32() - 0.5) * 2.0;
                Tower {
                    kind: if e.is_dir { Kind::Folder } else { kind_of(&e.name) },
                    entry: e,
                    base: Vec3::new(origin.x + col as f32 * CELL, 0.0, origin.y + row as f32 * CELL) + jitter,
                    size: Vec3::new(w, h, w),
                    seed: rng.f32() * 100.0,
                    first_layer: 0,
                    max_pages: 1,
                    pages: 0,
                    layer: None,
                    appear: 0.0,
                    hover: 0.0,
                    select: 0.0,
                    found: 0.0,
                    dim: 0.0,
                    page_hits: Vec::new(),
                    pv_kind,
                    pv_layer: None,
                    pv_size: None,
                    pv_video: false,
                    pv_duration: 0.0,
                    pv_cut: 0.0,
                    pv_still: None,
                }
            })
            .collect();

        // Share the page budget: trim the biggest towers first, everyone keeps one page.
        for t in &mut towers {
            t.max_pages = t.wanted_pages();
        }
        let mut total: u32 = towers.iter().map(|t| t.max_pages).sum();
        while total > layer_budget.max(towers.len() as u32) {
            let Some(big) = towers.iter_mut().filter(|t| t.max_pages > 1).max_by_key(|t| t.max_pages) else { break };
            big.max_pages -= 1;
            total -= 1;
        }
        // Previews: the first PV_LAYERS image/video towers get a layer.
        let mut pv_next = 0;
        for t in &mut towers {
            if t.pv_kind.is_some() && pv_next < crate::preview::PV_LAYERS {
                t.pv_layer = Some(pv_next);
                t.pv_video = t.pv_kind == Some(crate::preview::PvKind::Video);
                pv_next += 1;
            }
        }

        let mut next = 0;
        for t in &mut towers {
            t.first_layer = next;
            next += t.max_pages;
        }

        City {
            towers,
            cols,
            rows,
            origin,
            half_extent: Vec2::new(cols as f32 * CELL / 2.0, rows as f32 * CELL / 2.0),
        }
    }

    /// Closest tower hit by a ray, and the point where the ray meets it.
    pub fn pick(&self, o: Vec3, d: Vec3) -> Option<(usize, Vec3)> {
        let mut best = None;
        let mut best_t = f32::MAX;
        for (i, t) in self.towers.iter().enumerate() {
            if let Some(hit) = ray_box(o, d, t.min(), t.max()) {
                if hit < best_t {
                    best_t = hit;
                    best = Some(i);
                }
            }
        }
        best.map(|i| (i, o + d * best_t))
    }
}

fn kind_of(name: &str) -> Kind {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "rs" | "cs" | "cpp" | "c" | "h" | "js" | "ts" | "py" | "go" | "java" | "json" | "xml" | "md" | "txt"
        | "html" | "css" | "hlsl" | "wgsl" | "yml" | "yaml" | "toml" | "log" | "sh" | "ps1" => Kind::Text,
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "mp4" | "mkv" | "mov" | "mp3" | "wav" | "flac" => {
            Kind::Media
        }
        "exe" | "dll" | "zip" | "7z" | "rar" | "iso" | "msi" | "bin" | "pak" | "so" | "dylib" => Kind::Binary,
        _ => Kind::Other,
    }
}

fn ray_box(o: Vec3, d: Vec3, mn: Vec3, mx: Vec3) -> Option<f32> {
    let (mut t0, mut t1) = (0.0f32, f32::MAX);
    for a in 0..3 {
        if d[a].abs() < 1e-8 {
            if o[a] < mn[a] || o[a] > mx[a] {
                return None;
            }
            continue;
        }
        let (mut ta, mut tb) = ((mn[a] - o[a]) / d[a], (mx[a] - o[a]) / d[a]);
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
        }
        t0 = t0.max(ta);
        t1 = t1.min(tb);
        if t0 > t1 {
            return None;
        }
    }
    Some(t0)
}

/// Tiny deterministic RNG (splitmix64) so a folder always builds the same city.
pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn f32(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.next() % (hi - lo).max(1) as u64) as i32
    }
}
