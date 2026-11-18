// Keeps baked tower pages (compressed) so revisiting a folder is instant: going Back
// or Up shows the city at once instead of rebuilding every page.

use crate::fs_scan::Entry;
use crate::preview::Preview;
use crate::textgen::{LineHit, Page};
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

const CACHE_BYTES: usize = 384 * 1024 * 1024;

/// What's kept per tower: its packed pages, and its preview picture (plus, for
/// videos, where in the video the picture came from).
pub struct Cached {
    pub pages: Vec<PackedPage>,
    pub preview: Option<Arc<Preview>>,
    pub seek: f32,
}

impl Cached {
    fn bytes(&self) -> usize {
        self.pages.iter().map(PackedPage::bytes).sum::<usize>() + self.preview.as_ref().map_or(0, |p| p.bytes())
    }
}

/// A page, compressed: pages are mostly black, so zero runs are stored as (0, run).
pub struct PackedPage {
    data: Vec<u8>,
    hits: Vec<LineHit>,
}

impl PackedPage {
    pub fn pack(p: &Page) -> PackedPage {
        let mut data = Vec::with_capacity(p.mips.len() / 8);
        let mut i = 0;
        while i < p.mips.len() {
            let b = p.mips[i];
            if b == 0 {
                let mut n = 1;
                while n < 255 && i + n < p.mips.len() && p.mips[i + n] == 0 {
                    n += 1;
                }
                data.push(0);
                data.push(n as u8);
                i += n;
            } else {
                data.push(b);
                i += 1;
            }
        }
        PackedPage { data, hits: p.hits.clone() }
    }

    pub fn unpack(&self, len: usize) -> Page {
        let mut mips = Vec::with_capacity(len);
        let mut i = 0;
        while i < self.data.len() {
            let b = self.data[i];
            if b == 0 {
                mips.resize(mips.len() + self.data[i + 1] as usize, 0);
                i += 2;
            } else {
                mips.push(b);
                i += 1;
            }
        }
        Page { mips, hits: self.hits.clone() }
    }

    fn bytes(&self) -> usize {
        self.data.len() + self.hits.len() * std::mem::size_of::<LineHit>()
    }
}

/// What a tower's pages depend on: if any of this changes, they're baked again.
pub fn key(e: &Entry, seed: f32, max_pages: u32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    e.path.hash(&mut h);
    e.size.hash(&mut h);
    e.modified.hash(&mut h);
    e.child_count.hash(&mut h);
    e.lines.hash(&mut h);
    e.head.len().hash(&mut h);
    seed.to_bits().hash(&mut h);
    max_pages.hash(&mut h);
    h.finish()
}

#[derive(Default)]
struct Inner {
    map: HashMap<u64, Arc<Cached>>,
    order: VecDeque<u64>, // oldest first
    bytes: usize,
}

/// Shared between the bake threads.
#[derive(Clone, Default)]
pub struct PageCache(Arc<Mutex<Inner>>);

impl PageCache {
    pub fn get(&self, key: u64) -> Option<Arc<Cached>> {
        let mut c = self.0.lock().unwrap();
        let hit = c.map.get(&key).cloned();
        if hit.is_some() {
            // Most recently used goes to the back.
            if let Some(pos) = c.order.iter().position(|k| *k == key) {
                c.order.remove(pos);
            }
            c.order.push_back(key);
        }
        hit
    }

    pub fn put(&self, key: u64, entry: Cached) {
        let size = entry.bytes();
        let mut c = self.0.lock().unwrap();
        if c.map.insert(key, Arc::new(entry)).is_none() {
            c.order.push_back(key);
            c.bytes += size;
        }
        while c.bytes > CACHE_BYTES {
            let Some(old) = c.order.pop_front() else { break };
            if let Some(p) = c.map.remove(&old) {
                c.bytes -= p.bytes();
            }
        }
    }

    pub fn megabytes(&self) -> f32 {
        self.0.lock().unwrap().bytes as f32 / 1_048_576.0
    }
}
