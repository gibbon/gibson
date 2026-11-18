// Where the camera goes when you pick a tower: a viewpoint square-on to the face
// nearest you (so the text reads straight), or a path into the tower to enter it.

use crate::city::Tower;
use crate::textgen::{TEX_H, TEX_W};
use glam::Vec3;

/// The face of `t` that points most towards `from` (outward normal, on the ground plane).
fn facing_normal(t: &Tower, from: Vec3) -> Vec3 {
    let d = from - t.base;
    if d.x.abs() > d.z.abs() { Vec3::X * d.x.signum() } else { Vec3::Z * d.z.signum() }
}

/// A viewpoint in front of the tower's face nearest `from`, looking at the top of the
/// face, where the name, the listing and the file-type graph are.
pub fn view_point(t: &Tower, from: Vec3) -> (Vec3, Vec3) {
    let n = facing_normal(t, from);
    let (w, h) = (t.size.x, t.size.y);
    let look_y = (h - w * 0.8).max(h * 0.5).max(2.0);
    let dist = w * 1.25 + 3.0;
    let face = t.base + n * (w * 0.5);
    let eye = face + n * dist + Vec3::Y * (look_y + w * 0.1);
    let look = face + Vec3::Y * look_y;
    (eye, look)
}

/// A flight that ends inside the tower (entering it), coming in through the near face.
pub fn enter_point(t: &Tower, from: Vec3) -> (Vec3, Vec3) {
    let n = facing_normal(t, from);
    let mid = t.base + Vec3::Y * (t.size.y * 0.45).min(t.size.x * 2.0).max(2.0);
    (mid, mid - n * 20.0)
}

/// True when `p` is inside the tower (with a small margin so grazing a wall doesn't count).
pub fn inside(t: &Tower, p: Vec3) -> bool {
    let m = 0.4;
    let (mn, mx) = (t.min(), t.max());
    p.x > mn.x + m && p.x < mx.x - m && p.z > mn.z + m && p.z < mx.z - m && p.y < mx.y - m
}

/// A listed name found on a tower face.
#[derive(Clone, Copy, Debug)]
pub struct NameHit {
    pub line: usize,     // index into the tower's `lines`
    pub page: u32,       // which of the tower's pages it's on
    pub rect: [f32; 4],  // its box on the page, 0..1: u0, v0, u1, v1
}

/// Which listed name is at point `p` on the tower's surface.
/// Mirrors how tower.wgsl lays pages onto the faces.
pub fn line_at(t: &Tower, p: Vec3) -> Option<NameHit> {
    if t.page_hits.is_empty() {
        return None;
    }
    let s = t.size;
    let l = p - t.base;
    let (px, py, pz) = (l.x / s.x, l.y / s.y, l.z / s.z); // x,z in −0.5..0.5, y in 0..1
    // The face the point lies on: whichever boundary it's closest to.
    let (dx, dz, dy) = (0.5 - px.abs(), 0.5 - pz.abs(), 1.0 - py);
    // (face id as in the shader, u and v on the face in world units, face width, height)
    let (face, u, v, fw, fh) = if dy * s.y < dx * s.x && dy * s.y < dz * s.z {
        (4u32, (px + 0.5) * s.x, (pz + 0.5) * s.z, s.x, s.z)
    } else if dx * s.x < dz * s.z {
        if px > 0.0 {
            (3, (0.5 - pz) * s.z, (1.0 - py) * s.y, s.z, s.y)
        } else {
            (1, (pz + 0.5) * s.z, (1.0 - py) * s.y, s.z, s.y)
        }
    } else if pz > 0.0 {
        (0, (px + 0.5) * s.x, (1.0 - py) * s.y, s.x, s.y)
    } else {
        (2, (0.5 - px) * s.x, (1.0 - py) * s.y, s.x, s.y)
    };
    let page_h = fw * 2.0;
    let tv = v / page_h;
    let tile = tv.floor();
    let page = if face == 4 {
        0
    } else {
        let per_face = (fh / page_h - 0.001).ceil().max(1.0);
        let around = ((4 - face) % 4) as f32;
        ((around * per_face + tile) % t.pages.max(1) as f32) as usize
    };
    let x = (u / fw * TEX_W as f32) as i32;
    let y = ((tv - tile) * TEX_H as f32) as i32;
    let (tw, th) = (TEX_W as f32, TEX_H as f32);
    t.page_hits
        .get(page)?
        .iter()
        .find(|h| x >= h.x as i32 && x < (h.x + h.w) as i32 && y >= h.y as i32 && y < (h.y + h.h) as i32)
        .map(|h| NameHit {
            line: h.line as usize,
            page: page as u32,
            rect: [
                h.x as f32 / tw,
                h.y as f32 / th,
                (h.x + h.w) as f32 / tw,
                (h.y + h.h) as f32 / th,
            ],
        })
}
