// Autofly: the camera flies itself down the streets between the towers, like the
// film's flights. Waypoints are street crossings (cell corners); at each one it
// mostly carries straight on, sometimes turns. It cruises along the rooftops and
// now and then dives down into the streets for a stretch, then climbs back up. A Catmull-Rom curve through the waypoints keeps
// it smooth, and the camera looks a little way ahead along it.

use crate::city::{City, Rng, CELL};
use glam::Vec3;
use std::collections::VecDeque;

pub struct AutoFly {
    pts: VecDeque<Vec3>, // p0, p1, p2, p3, p4: flying p1 → p2
    s: f32,              // 0..1 along p1 → p2
    at: (i32, i32),      // crossing of the last waypoint
    dir: (i32, i32),     // street direction
    dive: u32,           // waypoints left down in the streets (0 = up on the roofs)
    rng: Rng,
}

/// A street crossing's position (cell corners: between the towers).
fn crossing(city: &City, i: i32, j: i32, height: f32) -> Vec3 {
    Vec3::new(city.origin.x + (i as f32 - 0.5) * CELL, height, city.origin.y + (j as f32 - 0.5) * CELL)
}

/// The streets to fly: the inner ones (towers on both sides), unless the city is tiny.
fn inner(city: &City) -> (i32, i32, i32) {
    let (cols, rows) = (city.cols as i32, city.rows as i32);
    if cols >= 3 && rows >= 3 { (1, cols - 1, rows - 1) } else { (0, cols, rows) }
}

/// Roof level: the height most towers stay under (the 80th percentile).
fn roof(city: &City) -> f32 {
    let mut h: Vec<f32> = city.towers.iter().filter(|t| !t.secret).map(|t| t.size.y).collect();
    if h.is_empty() {
        return 12.0;
    }
    h.sort_by(f32::total_cmp);
    h[(h.len() * 4 / 5).min(h.len() - 1)].max(8.0)
}

fn catmull(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Vec3 {
    let (t2, t3) = (t * t, t * t * t);
    0.5 * ((2.0 * p1) + (p2 - p0) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3)
}

impl AutoFly {
    /// Starts from the camera: the nearest crossing, heading the way it faces.
    pub fn new(city: &City, from: Vec3, facing: Vec3, seed: u64) -> AutoFly {
        let (lo, hi_i, hi_j) = inner(city);
        let i = (((from.x - city.origin.x) / CELL) + 0.5).round().clamp(lo as f32, hi_i as f32) as i32;
        let j = (((from.z - city.origin.y) / CELL) + 0.5).round().clamp(lo as f32, hi_j as f32) as i32;
        let dir = if facing.x.abs() > facing.z.abs() {
            (facing.x.signum() as i32, 0)
        } else {
            (0, if facing.z < 0.0 { -1 } else { 1 })
        };
        let mut a = AutoFly { pts: VecDeque::new(), s: 0.0, at: (i, j), dir, dive: 0, rng: Rng(seed) };
        let first = crossing(city, i, j, from.y.clamp(6.0, 60.0));
        let back = from - (first - from).normalize_or(Vec3::Z) * 10.0;
        a.pts.extend([back, from, first]);
        while a.pts.len() < 5 {
            let p = a.next_waypoint(city);
            a.pts.push_back(p);
        }
        a
    }

    fn next_waypoint(&mut self, city: &City) -> Vec3 {
        let (lo, hi_i, hi_j) = inner(city);
        let inside = |(i, j): (i32, i32)| i >= lo && j >= lo && i <= hi_i && j <= hi_j;
        let (dx, dz) = self.dir;
        let straight = (dx, dz);
        let left = (dz, -dx);
        let right = (-dz, dx);
        let step = |d: (i32, i32), at: (i32, i32)| (at.0 + d.0, at.1 + d.1);
        let r = self.rng.f32();
        let mut order = if r < 0.62 { [straight, left, right] } else if r < 0.81 { [left, straight, right] } else { [right, straight, left] };
        if self.rng.f32() < 0.5 {
            order.swap(1, 2);
        }
        let d = order
            .into_iter()
            .find(|&d| inside(step(d, self.at)))
            .unwrap_or((-dx, -dz)); // a dead end (tiny city): turn back
        self.dir = d;
        self.at = step(d, self.at);
        // Up at roof level (just over most towers), diving now and then.
        if self.dive > 0 {
            self.dive -= 1;
        } else if self.rng.f32() < 0.14 {
            self.dive = 1 + (self.rng.f32() * 2.5) as u32; // 1-3 crossings down low
        }
        let h = if self.dive > 0 { 5.0 + self.rng.f32() * 7.0 } else { roof(city) * (0.95 + self.rng.f32() * 0.2) + 3.0 };
        crossing(city, self.at.0, self.at.1, h)
    }

    /// Advances `speed` units along the route; returns where to be and where to look.
    pub fn step(&mut self, city: &City, dt: f32, speed: f32) -> (Vec3, Vec3) {
        let len = self.pts[1].distance(self.pts[2]).max(1.0);
        self.s += speed * dt / len;
        while self.s >= 1.0 {
            self.s -= 1.0;
            self.pts.pop_front();
            let p = self.next_waypoint(city);
            self.pts.push_back(p);
        }
        let p = |k: usize| self.pts[k];
        let pos = catmull(p(0), p(1), p(2), p(3), self.s);
        // Look ahead along the curve (into the next stretch if need be).
        let ahead = self.s + 0.45;
        let look = if ahead < 1.0 { catmull(p(0), p(1), p(2), p(3), ahead) } else { catmull(p(1), p(2), p(3), p(4), ahead - 1.0) };
        // Eyes level or down, never up at the empty sky (even while climbing).
        // ...and not steeply down either (about 30° at most), even in a dive.
        let flat = Vec3::new(look.x - pos.x, 0.0, look.z - pos.z).length().max(1.0);
        let look = Vec3::new(look.x, look.y.min(pos.y - flat * 0.1).max(pos.y - flat * 0.55) - 1.5, look.z);
        (pos.max(Vec3::new(f32::MIN, 1.5, f32::MIN)), look)
    }
}
