// Gibson — fly through your file system as a city of glass towers.

// Release builds on Windows are a normal windowed app (no console); see the log file.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod cache;
mod audio;
mod autofly;
mod camera;
mod city;
mod files;
mod filetype;
mod fs_scan;
mod hud;
mod nav;
mod preview;
mod render;
mod settings;
mod textgen;
mod viewer;

use camera::{Camera, FlyInput};
use city::City;
use glam::Vec3;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowId};

/// Work coming back from the loader threads.
enum Msg {
    Scanned { generation: u64, path: PathBuf, result: Result<Vec<fs_scan::Entry>, String> },
    Baked {
        generation: u64,
        index: usize,
        pages: Vec<textgen::Page>,
        cached: bool,
        preview: Option<Arc<preview::Preview>>,
        duration: f32,
    },
    Clip { generation: u64, index: usize, clip: Option<preview::Clip> },
    Drives(Vec<PathBuf>),
}

const PAGE_UPLOADS_PER_FRAME: usize = 8; // ~6 MB per frame: big folders load without hitches
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const CLICK_SLOP: f64 = 5.0; // pixels the mouse may move and still count as a click
const HUD_MIN_INTERVAL: Duration = Duration::from_millis(33); // HUD redraws at most ~30/s
const HISTORY_MAX: usize = 100;
const MAX_PLAYING: usize = 4; // videos playing at once (the nearest)
const PLAY_DISTANCE: f32 = 170.0;

struct Running {
    window: Arc<Window>,
    renderer: render::Renderer,
    camera: Camera,
    city: Option<City>,
    path: PathBuf,
    loading: Option<PathBuf>,   // folder being scanned (the old city stays up meanwhile)
    loading_back: bool,         // that load is a "Back" (pops history instead of pushing)
    came_from: Option<PathBuf>, // folder we went up out of: start in front of its tower
    history: Vec<PathBuf>,      // folders visited before this one, for Back
    generation: u64,
    live_gen: Arc<AtomicU64>, // bakers skip work for folders we've left
    bake_pool: Arc<rayon::ThreadPool>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    fonts: Arc<textgen::Fonts>,
    pending: Vec<Baked>, // baked towers, waiting for upload
    clips: HashMap<usize, (preview::Clip, usize)>, // tower → video montage, frame shown
    clip_loading: HashSet<usize>,
    clip_failed: HashSet<usize>,
    cache: cache::PageCache,
    hud: hud::Hud,
    drives: Vec<PathBuf>,
    message: Option<(String, bool, Instant)>, // (text, is_error, since)
    keys: HashSet<KeyCode>,
    dragging: bool,
    press: Option<(f64, f64)>, // where the left button went down
    last_cursor: Option<(f64, f64)>,
    cursor: (f64, f64),
    look: (f32, f32),
    hovered: Option<usize>,
    hovered_name: Option<nav::NameHit>, // listed name under the cursor (on a folder tower)
    selected: Option<usize>,     // tower last clicked: tinted, and shown in the path bar
    hot: Option<usize>,          // HUD button under the cursor
    last_click: Option<(Instant, usize, Option<usize>)>, // when, tower, name
    entering: Option<usize>, // tower we're flying into
    help: bool,              // hotkey panel open
    filter: Vec<filetype::Cat>, // file types shown (empty = all)
    search_filter: bool,     // search hides non-matches (live) instead of dimming
    viewer: Option<viewer::Viewer>,
    effect: usize,           // viewer effect, remembered between pictures
    settings: settings::Settings,
    scrub_drag: Option<f32>, // dragging the viewer's timeline: where (0..1)
    view_on_load: Option<PathBuf>, // started with a file: view it once its folder is up
    autofly: Option<autofly::AutoFly>, // flying itself down the streets (P)
    search: Option<String>,  // search box open, with its text
    matches: Vec<f32>,       // per tower: 1 name matches, 0.5 something inside it does
    match_order: Vec<usize>, // matching towers, best and nearest first (Enter walks it)
    match_next: usize,
    cursor_icon: CursorIcon,
    hud_drawn: Instant,
    start: Instant,
    last_frame: Instant,
    fps_frames: u32,
    fps_time: Instant,
    fps: u32,
    city_since: Instant,   // when the current city went up
    stuck_logged: bool,    // reported towers that never finished building
}

struct App {
    start_path: PathBuf,
    start_file: Option<PathBuf>, // given a file: open its folder and view it
    state: Option<Running>,
}

impl Running {
    /// Scans a folder on a worker thread. The current city stays until it's ready.
    /// If `path` is above the current folder, the camera starts in front of the
    /// folder we came out of.
    fn load(&mut self, path: PathBuf, back: bool) {
        if self.loading.as_ref() == Some(&path) {
            return;
        }
        log::info!("open {}", path.display());
        self.generation += 1;
        self.live_gen.store(self.generation, Ordering::Relaxed);
        self.loading = Some(path.clone());
        self.loading_back = back;
        self.came_from = self.path.ancestors().find(|a| a.parent() == Some(path.as_path())).map(Path::to_path_buf);
        self.entering = None;
        let (tx, generation) = (self.tx.clone(), self.generation);
        std::thread::spawn(move || {
            let result = fs_scan::scan(&path);
            let _ = tx.send(Msg::Scanned { generation, path, result });
        });
    }

    fn go_up(&mut self) {
        match self.path.parent() {
            Some(parent) => self.load(parent.to_path_buf(), false),
            None => self.say("TOP OF THE DRIVE", false),
        }
    }

    /// Back to the folder before this one, like a browser.
    fn go_back(&mut self) {
        match self.history.last().cloned() {
            Some(prev) => self.load(prev, true),
            None => self.say("NOTHING TO GO BACK TO", false),
        }
    }

    fn say(&mut self, text: &str, error: bool) {
        self.message = Some((text.to_string(), error, Instant::now()));
    }

    fn handle_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Scanned { generation, path, result } if generation == self.generation => {
                    self.loading = None;
                    match result {
                        Ok(entries) => self.show(path, entries),
                        Err(e) => {
                            log::warn!("can't open {}: {e}", path.display());
                            let name = path.file_name().map(|n| n.to_string_lossy().to_uppercase()).unwrap_or_default();
                            self.say(&format!("{e}: {name}"), true);
                        }
                    }
                }
                Msg::Baked { generation, index, pages, cached, preview, duration } if generation == self.generation => {
                    self.pending.push(Baked { index, pages, cached, preview, duration });
                }
                Msg::Clip { generation, index, clip } if generation == self.generation => {
                    self.clip_loading.remove(&index);
                    match clip {
                        Some(f) => {
                            self.clips.insert(index, (f, usize::MAX));
                        }
                        None => {
                            self.clip_failed.insert(index);
                        }
                    }
                }
                Msg::Drives(d) => self.drives = d,
                _ => {} // stale: from a folder we've already left
            }
        }
        // Towers still unbuilt long after the city went up: say which (a file that
        // hangs its decoder, say), once.
        if let Some(city) = &self.city {
            if !self.stuck_logged && self.pending.is_empty() && self.city_since.elapsed() > Duration::from_secs(15) {
                let stuck: Vec<&str> =
                    city.towers.iter().filter(|t| t.layer.is_none()).map(|t| t.entry.name.as_str()).take(8).collect();
                if !stuck.is_empty() {
                    log::warn!("towers still unbuilt after 15 s: {stuck:?}");
                }
                self.stuck_logged = true;
            }
        }
        // Upload a few pages per frame so loading never stutters.
        if let Some(city) = &mut self.city {
            let mut uploaded = 0;
            while uploaded < PAGE_UPLOADS_PER_FRAME {
                let Some(Baked { index, pages, cached, preview, duration }) = self.pending.pop() else { break };
                let t = &mut city.towers[index];
                if let (Some(layer), Some(p)) = (t.pv_layer, preview) {
                    self.renderer.upload_preview(layer, &p);
                    t.pv_size = Some((p.w, p.h));
                    t.pv_duration = duration;
                    t.pv_still = Some(p);
                    uploaded += 1;
                }
                if cached {
                    t.appear = 1.0; // seen before: no need to rise again
                }
                let pages: Vec<textgen::Page> = pages.into_iter().take(t.max_pages as usize).collect();
                for (p, page) in pages.iter().enumerate() {
                    self.renderer.upload_face(t.first_layer + p as u32, &page.mips);
                }
                t.pages = pages.len() as u32;
                uploaded += pages.len();
                t.page_hits = pages.into_iter().map(|p| p.hits).collect();
                t.layer = Some(t.first_layer);
            }
        }
    }

    /// A folder's scan is in: build its city and put the camera somewhere sensible.
    fn show(&mut self, path: PathBuf, entries: Vec<fs_scan::Entry>) {
        if self.city.is_some() && path != self.path {
            if self.loading_back {
                self.history.pop();
            } else {
                self.history.push(self.path.clone());
                if self.history.len() > HISTORY_MAX {
                    self.history.remove(0);
                }
            }
        }
        let city = City::build(entries, self.renderer.max_face_layers);
        self.renderer.reserve_face_layers(city.layers());
        self.pending.clear();
        self.clips.clear();
        self.clip_loading.clear();
        self.clip_failed.clear();
        self.hovered = None;
        self.hovered_name = None;
        self.selected = None;
        self.last_click = None;

        // Coming back up: start in front of the folder we left. Otherwise: at the
        // front edge of the city, low, looking down the main lane.
        let speed = self.camera.speed;
        self.camera = Camera::new();
        self.camera.speed = speed;
        let back_to = self.came_from.take().and_then(|from| city.towers.iter().find(|t| t.entry.path == from));
        match back_to {
            Some(t) => {
                let (eye, look) = nav::view_point(t, t.base + Vec3::Z * 1000.0);
                let back = (eye - look).with_y(0.0).normalize_or_zero() * 14.0;
                self.camera.place(eye + back + Vec3::Y * 2.0, look);
            }
            None => {
                // Raised, just in front of the city, looking down over the towers.
                let tallest = city.towers.iter().map(|t| t.size.y).fold(10.0f32, f32::max);
                let h = (tallest * 0.75).clamp(22.0, 70.0);
                let eye = Vec3::new(0.0, h, city.half_extent.y + 10.0 + h * 0.45);
                let look = Vec3::new(0.0, tallest * 0.2, -city.half_extent.y * 0.25);
                self.camera.place(eye, look);
            }
        }
        if self.autofly.is_some() {
            // Keep flying, through the new city.
            self.autofly = Some(autofly::AutoFly::new(&city, self.camera.pos, self.camera.forward(), self.generation));
        }

        // Bake every face in parallel, off the UI thread.
        let generation = self.generation;
        let jobs: Vec<(usize, fs_scan::Entry, f32, u32, Option<preview::PvKind>)> = city
            .towers
            .iter()
            .enumerate()
            .map(|(i, t)| (i, t.entry.clone(), t.seed, t.max_pages, t.pv_layer.and(t.pv_kind)))
            .collect();
        // On a pool that leaves cores free for flying, and skipping work as soon as
        // we've moved on to another folder.
        // Pages baked before (and unchanged) come straight from the cache.
        let (tx, fonts, live, cache) = (self.tx.clone(), self.fonts.clone(), self.live_gen.clone(), self.cache.clone());
        self.bake_pool.spawn(move || {
            jobs.into_par_iter().for_each(|(index, entry, seed, max_pages, pv_kind)| {
                if live.load(Ordering::Relaxed) != generation {
                    return;
                }
                let key = cache::key(&entry, seed, max_pages);
                let (pages, cached, preview, duration) = match cache.get(key) {
                    Some(c) => (
                        c.pages.iter().map(|p| p.unpack(textgen::mips_len())).collect(),
                        true,
                        c.preview.clone(),
                        c.duration,
                    ),
                    None => {
                        // The picture first: the front page leaves room for it.
                        let (preview, duration) = match pv_kind {
                            Some(preview::PvKind::Image) => (preview::image(&entry.path).map(Arc::new), 0.0),
                            Some(preview::PvKind::Video) => match preview::video_still(&entry.path) {
                                Some((p, d)) => (Some(Arc::new(p)), d),
                                None => (None, 0.0),
                            },
                            None => (None, 0.0),
                        };
                        let picture = preview.as_ref().map(|p| (p.w, p.h));
                        let pages = textgen::bake(&entry, seed, &fonts, max_pages, picture);
                        let packed = pages.iter().map(cache::PackedPage::pack).collect();
                        cache.put(key, cache::Cached { pages: packed, preview: preview.clone(), duration });
                        (pages, false, preview, duration)
                    }
                };
                let _ = tx.send(Msg::Baked { generation, index, pages, cached, preview, duration });
            });
        });
        log::info!(
            "city: {} towers in {}x{}, {} face pages (page cache {:.0} MB)",
            city.towers.len(),
            city.cols,
            city.rows,
            city.layers(),
            self.cache.megabytes()
        );
        self.city = Some(city);
        self.path = path;
        self.city_since = Instant::now();
        self.stuck_logged = false;
        self.filter.clear();
        self.update_matches();
        if let Some(file) = self.view_on_load.take() {
            let i = self.city.as_ref().and_then(|c| c.towers.iter().position(|t| t.entry.path == file));
            match i {
                Some(i) => self.view_tower(i),
                None => self.open_viewer(vec![file], 0),
            }
        }
    }

    /// Plays the nearest few videos (looping clips decoded by FFmpeg); the others show
    /// their still frame.
    fn play_videos(&mut self, time: f32) {
        if !preview::ffmpeg_ready() {
            return;
        }
        let Some(city) = &self.city else { return };
        let cam = self.camera.pos;
        let mut near: Vec<(f32, usize)> = city
            .towers
            .iter()
            .enumerate()
            .filter(|(_, t)| t.pv_video && t.pv_size.is_some() && t.appear > 0.99)
            .map(|(i, t)| ((t.base + Vec3::Y * t.size.y.min(t.size.x * 2.0) * 0.5).distance(cam), i))
            .filter(|&(d, i)| d < PLAY_DISTANCE && !self.clip_failed.contains(&i))
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        let wanted: Vec<usize> = near.iter().take(MAX_PLAYING).map(|&(_, i)| i).collect();

        // Out of range: back to the still frame, and free the frames.
        let gone: Vec<usize> = self.clips.keys().copied().filter(|i| !wanted.contains(i)).collect();
        for i in gone {
            self.clips.remove(&i);
            let t = &city.towers[i];
            if let (Some(layer), Some(still)) = (t.pv_layer, &t.pv_still) {
                self.renderer.upload_preview(layer, still);
            }
        }
        let mut cuts: Vec<(usize, f32)> = Vec::new();
        for &i in &wanted {
            let t = &city.towers[i];
            let Some(layer) = t.pv_layer else { continue };
            match self.clips.get_mut(&i) {
                Some((clip, shown)) => {
                    let f = ((time * preview::CLIP_FPS) as usize + i * 7) % clip.frames.len();
                    if f != *shown {
                        self.renderer.upload_preview(layer, &clip.frames[f]);
                        *shown = f;
                    }
                    // The first two frames after a cut glitch hard.
                    let near_cut = clip.cuts.len() > 1 && clip.cuts.iter().any(|&c| f >= c && f < c + 2);
                    cuts.push((i, if near_cut { 1.0 } else { 0.0 }));
                }
                None if !self.clip_loading.contains(&i) => {
                    self.clip_loading.insert(i);
                    let (path, dur, h) = (t.entry.path.clone(), t.pv_duration, t.pv_size.map_or(0, |s| s.1));
                    let (tx, generation) = (self.tx.clone(), self.generation);
                    std::thread::spawn(move || {
                        let clip = preview::video_clip(&path, dur, h);
                        let _ = tx.send(Msg::Clip { generation, index: i, clip });
                    });
                }
                None => {}
            }
        }
        if let Some(city) = &mut self.city {
            for t in &mut city.towers {
                t.pv_cut = 0.0;
            }
            for (i, c) in cuts {
                city.towers[i].pv_cut = c;
            }
        }
    }

    /// Left click (press + release without dragging).
    fn click(&mut self) {
        // HUD first: drives and path segments.
        let (cx, cy) = self.cursor;
        if let Some(action) = self.hud.hits.iter().find(|(r, _)| r.contains(cx, cy)).map(|h| h.1.clone()) {
            match action {
                hud::HudAction::Go(path) => self.load(path, false),
                hud::HudAction::Back => self.go_back(),
                hud::HudAction::Up => self.go_up(),
                hud::HudAction::Help => self.help = !self.help,
                hud::HudAction::Filter(None) => self.filter.clear(),
                hud::HudAction::Filter(Some(c)) => {
                    match self.filter.iter().position(|&f| f == c) {
                        Some(k) => {
                            self.filter.remove(k);
                        }
                        None => self.filter.push(c),
                    }
                }
                hud::HudAction::SearchFilter => self.search_filter = !self.search_filter,
                hud::HudAction::Effect(e) => self.set_effect(e),
                hud::HudAction::PlayPause => {
                    if let Some(v) = &mut self.viewer {
                        v.toggle_pause();
                    }
                }
                hud::HudAction::CloseViewer => self.viewer = None,
                hud::HudAction::TowerFx => self.step_tower_fx(1),
                hud::HudAction::AutoFly => self.toggle_autofly(),
            }
            return;
        }
        if self.help {
            self.help = false; // a click outside the panel closes it
            return;
        }
        if self.viewer.is_some() {
            return;
        }
        let Some(i) = self.hovered else { return };
        let now = Instant::now();
        // The first click starts a flight, so by the second click the cursor may be
        // over a different name: a double click acts on what the first click hit.
        if let Some((t, j, line)) = self.last_click {
            if j == i && now - t < DOUBLE_CLICK {
                self.last_click = None;
                match line {
                    Some(l) => self.open_child(i, l),
                    None => self.enter(i),
                }
                return;
            }
        }
        self.last_click = Some((now, i, self.hovered_name.map(|n| n.line)));
        self.selected = Some(i);
        self.fly_to(i);
    }

    /// Opens a name listed on a folder tower's face (a subfolder of that folder).
    fn open_child(&mut self, i: usize, line: usize) {
        let Some(city) = &self.city else { return };
        let e = &city.towers[i].entry;
        let Some((name, is_dir)) = e.lines.get(line) else { return };
        let path = e.path.join(name);
        if *is_dir {
            self.load(path, false);
        } else if preview::kind(&path).is_some() {
            // Step through the pictures and videos listed on this tower.
            let items: Vec<PathBuf> = e
                .lines
                .iter()
                .filter(|(n, d)| !*d && preview::kind(Path::new(n)).is_some())
                .map(|(n, _)| e.path.join(n))
                .collect();
            let at = items.iter().position(|p| *p == path).unwrap_or(0);
            self.open_viewer(items, at);
        } else {
            self.report(files::open(&path));
        }
    }

    /// Opens the media viewer on `items[at]`.
    fn open_viewer(&mut self, items: Vec<PathBuf>, at: usize) {
        if items.is_empty() {
            return;
        }
        if preview::kind(&items[at]) == Some(preview::PvKind::Video) && !preview::have_ffmpeg() {
            self.say("VIDEOS NEED FFMPEG INSTALLED", true);
            return;
        }
        self.keys.clear();
        self.search = None;
        self.help = false;
        self.camera.autopilot = None;
        let volume = self.settings.volume as f32 / 100.0;
        self.viewer = Some(viewer::Viewer::open(items, at, self.effect, self.renderer.size(), volume));
    }

    /// Opens the viewer on tower `i`, stepping through the city's pictures and videos.
    fn view_tower(&mut self, i: usize) {
        let Some(city) = &self.city else { return };
        let media: Vec<usize> = (0..city.towers.len()).filter(|&k| city.towers[k].pv_kind.is_some()).collect();
        let items: Vec<PathBuf> = media.iter().map(|&k| city.towers[k].entry.path.clone()).collect();
        let at = media.iter().position(|&k| k == i).unwrap_or(0);
        self.open_viewer(items, at);
    }

    /// V: view what's under the cursor (a tower, or a name on one) or the selection.
    fn view_target(&mut self) {
        let Some(city) = &self.city else { return };
        if let (Some(i), Some(n)) = (self.hovered, self.hovered_name) {
            if let Some((name, false)) = city.towers[i].entry.lines.get(n.line) {
                if preview::kind(Path::new(name)).is_some() {
                    self.open_child(i, n.line);
                    return;
                }
            }
        }
        match self.hovered.or(self.selected) {
            Some(i) if city.towers[i].pv_kind.is_some() => self.view_tower(i),
            _ => self.say("POINT AT AN IMAGE OR VIDEO TO VIEW IT", false),
        }
    }

    fn set_effect(&mut self, e: usize) {
        self.effect = e % viewer::EFFECTS.len();
        if let Some(v) = &mut self.viewer {
            v.effect = self.effect;
        }
        self.settings.viewer_fx = self.effect;
        settings::save(&self.settings);
    }

    /// The effect on every tower's picture / video preview (T, Shift+T, or the strip button).
    fn step_tower_fx(&mut self, by: i32) {
        let n = viewer::EFFECTS.len() as i32;
        let fx = (self.settings.tower_fx as i32 + by).rem_euclid(n) as usize;
        self.settings.tower_fx = fx;
        self.renderer.tower_fx = fx as u32;
        settings::save(&self.settings);
        self.say(&format!("TOWER PREVIEWS: {}", viewer::EFFECTS[fx]), false);
    }

    /// Keys while the viewer is open. All are consumed (no flying).
    fn viewer_key(&mut self, ev: &KeyEvent) {
        let PhysicalKey::Code(code) = ev.physical_key else { return };
        let shift = self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight);
        let screen = self.renderer.size();
        let n = viewer::EFFECTS.len();
        match code {
            KeyCode::Escape if self.help => self.help = false,
            KeyCode::Escape => self.viewer = None,
            KeyCode::F1 => self.help = !self.help,
            KeyCode::Space if !ev.repeat => {
                if let Some(v) = &mut self.viewer {
                    v.toggle_pause();
                }
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let by = if shift { 30.0 } else { 5.0 } * if code == KeyCode::ArrowLeft { -1.0 } else { 1.0 };
                if let Some(v) = &mut self.viewer {
                    v.seek_by(by);
                }
            }
            KeyCode::BracketLeft | KeyCode::PageUp | KeyCode::BracketRight | KeyCode::PageDown if !ev.repeat => {
                let by = if matches!(code, KeyCode::BracketLeft | KeyCode::PageUp) { -1 } else { 1 };
                if let Some(v) = &mut self.viewer {
                    v.step(by, screen);
                }
            }
            KeyCode::KeyE if !ev.repeat => self.set_effect(if shift { self.effect + n - 1 } else { self.effect + 1 }),
            KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4 | KeyCode::Digit5 | KeyCode::Digit6 => {
                let d = match code {
                    KeyCode::Digit1 => 0,
                    KeyCode::Digit2 => 1,
                    KeyCode::Digit3 => 2,
                    KeyCode::Digit4 => 3,
                    KeyCode::Digit5 => 4,
                    _ => 5,
                };
                self.set_effect(d);
            }
            KeyCode::KeyM if !ev.repeat => {
                if let Some(v) = &mut self.viewer {
                    v.toggle_mute();
                }
            }
            KeyCode::Equal | KeyCode::NumpadAdd | KeyCode::Minus | KeyCode::NumpadSubtract => {
                let up = matches!(code, KeyCode::Equal | KeyCode::NumpadAdd);
                if let Some(v) = &mut self.viewer {
                    v.change_volume(if up { 0.1 } else { -0.1 });
                    self.settings.volume = (v.volume * 100.0).round() as u32;
                    settings::save(&self.settings);
                }
            }
            KeyCode::KeyO if !ev.repeat => {
                if let Some(v) = &self.viewer {
                    let p = v.path().to_path_buf();
                    self.report(files::open(&p));
                }
            }
            _ => {}
        }
    }

    /// Where the viewer's picture goes: fitted between the top overlay and the timeline.
    fn viewer_rect(&self, w: u32, h: u32) -> [f32; 4] {
        let s = self.window.scale_factor() as f32;
        let Some(v) = &self.viewer else { return [0.0; 4] };
        let (pw, ph) = v.size.unwrap_or((16, 9));
        let top = self.hud.bar_h as f32 + 64.0 * s;
        let bottom = h as f32 - 116.0 * s;
        let (aw, ah) = (w as f32 - 48.0 * s, (bottom - top).max(10.0));
        let k = (aw / pw as f32).min(ah / ph as f32);
        let (dw, dh) = (pw as f32 * k, ph as f32 * k);
        let x0 = (w as f32 - dw) / 2.0;
        let y0 = top + (ah - dh) / 2.0;
        [x0, y0, x0 + dw, y0 + dh]
    }

    fn report(&mut self, r: Result<String, String>) {
        match r {
            Ok(m) => self.say(&m, false),
            Err(m) => self.say(&m, true),
        }
    }

    /// The file or folder the user means: the name under the cursor, else the tower
    /// under the cursor, else the selected tower.
    fn target_path(&self) -> Option<PathBuf> {
        let city = self.city.as_ref()?;
        if let (Some(i), Some(n)) = (self.hovered, self.hovered_name) {
            let e = &city.towers[i].entry;
            return e.lines.get(n.line).map(|(name, _)| e.path.join(name));
        }
        self.hovered.or(self.selected).map(|i| city.towers[i].entry.path.clone())
    }

    fn reveal(&mut self) {
        match self.target_path() {
            Some(p) => self.report(files::reveal(&p)),
            None => self.say("POINT AT SOMETHING TO SHOW IT IN EXPLORER", false),
        }
    }

    // ── Search: type to light up matching towers ───────────────────────────
    fn open_search(&mut self) {
        self.search = Some(String::new());
        self.keys.clear(); // typing mustn't fly
        self.update_matches();
    }

    fn close_search(&mut self) {
        self.search = None;
        self.update_matches();
    }

    /// Ctrl (or Cmd on a Mac) is down.
    fn command_held(&self) -> bool {
        [KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::SuperLeft, KeyCode::SuperRight]
            .iter()
            .any(|k| self.keys.contains(k))
    }

    /// P / the AUTOFLY chip: fly itself down the streets, or stop.
    fn toggle_autofly(&mut self) {
        if self.autofly.take().is_some() {
            self.say("AUTOFLY OFF", false);
            return;
        }
        let Some(city) = &self.city else { return };
        if city.towers.is_empty() {
            return;
        }
        let seed = self.start.elapsed().as_nanos() as u64 ^ 0x5DEE_CE66;
        self.autofly = Some(autofly::AutoFly::new(city, self.camera.pos, self.camera.forward(), seed));
        self.selected = None;
        self.entering = None;
        self.say("AUTOFLY  -  ANY KEY, DRAG OR CLICK TAKES OVER", false);
    }

    /// Where the cursor is along the viewer's timeline, 0..1.
    fn scrub_fraction(&self) -> f32 {
        match self.hud.scrub {
            Some(r) => ((self.cursor.0 as f32 - r.x as f32) / r.w.max(1) as f32).clamp(0.0, 1.0),
            None => 0.0,
        }
    }

    /// Keys while the search box is open. All are consumed.
    fn search_key(&mut self, ev: &KeyEvent) {
        match ev.physical_key {
            PhysicalKey::Code(KeyCode::Escape) => self.close_search(),
            PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter) => self.next_match(),
            PhysicalKey::Code(KeyCode::Tab) => self.search_filter = !self.search_filter,
            PhysicalKey::Code(KeyCode::Backspace) => {
                if let Some(q) = &mut self.search {
                    q.pop();
                }
                self.update_matches();
            }
            _ => {
                let add: String = ev.text.as_deref().unwrap_or("").chars().filter(|c| !c.is_control()).collect();
                if let (Some(q), false) = (&mut self.search, add.is_empty()) {
                    q.push_str(&add);
                    self.update_matches();
                }
            }
        }
    }

    fn update_matches(&mut self) {
        self.match_next = 0;
        self.match_order.clear();
        let q = self.search.as_deref().unwrap_or("").to_lowercase();
        let Some(city) = &self.city else {
            self.matches.clear();
            return;
        };
        self.matches = city
            .towers
            .iter()
            .map(|t| {
                if q.is_empty() {
                    0.0
                } else if t.entry.name.to_lowercase().contains(&q) {
                    1.0
                } else if t.entry.is_dir && t.entry.lines.iter().any(|(n, _)| n.to_lowercase().contains(&q)) {
                    0.5
                } else {
                    0.0
                }
            })
            .collect();
        let pos = self.camera.pos;
        let dist = |i: usize| city.towers[i].base.distance_squared(pos);
        let m = &self.matches;
        let mut order: Vec<usize> = (0..m.len()).filter(|&i| m[i] > 0.0).collect();
        order.sort_by(|&a, &b| m[b].total_cmp(&m[a]).then(dist(a).total_cmp(&dist(b))));
        self.match_order = order;
    }

    /// Enter in the search box: fly to the next match.
    fn next_match(&mut self) {
        if self.match_order.is_empty() {
            return;
        }
        let i = self.match_order[self.match_next % self.match_order.len()];
        self.match_next += 1;
        self.selected = Some(i);
        self.fly_to(i);
    }

    fn fly_to(&mut self, i: usize) {
        self.autofly = None;
        let Some(city) = &self.city else { return };
        let (eye, look) = nav::view_point(&city.towers[i], self.camera.pos);
        self.camera.fly_to(eye, look);
        self.entering = None;
    }

    /// Fly into a folder tower; arriving inside opens it.
    fn enter(&mut self, i: usize) {
        self.autofly = None;
        let Some(city) = &self.city else { return };
        let t = &city.towers[i];
        if !t.entry.is_dir {
            let path = t.entry.path.clone();
            if t.pv_kind.is_some() {
                self.view_tower(i);
                return;
            }
            self.fly_to(i);
            self.report(files::open(&path));
            return;
        }
        let (eye, look) = nav::enter_point(t, self.camera.pos);
        self.camera.fly_to(eye, look);
        self.entering = Some(i);
    }

    /// Opens a folder once the camera is inside its tower (flown in, or by autopilot).
    fn check_entered(&mut self) {
        if self.loading.is_some() || self.viewer.is_some() {
            return;
        }
        let Some(city) = &self.city else { return };
        let pos = self.camera.pos;
        // On autopilot only the tower being entered counts (flights pass through
        // others on the way); flying by hand, any folder you fly into opens.
        let can_open = |i: usize| match (self.entering, self.camera.autopilot.is_some()) {
            (Some(e), _) => e == i,
            (None, autopilot) => !autopilot,
        };
        let inside = city
            .towers
            .iter()
            .enumerate()
            .position(|(i, t)| can_open(i) && t.entry.is_dir && t.layer.is_some() && nav::inside(t, pos));
        if let Some(i) = inside {
            let path = city.towers[i].entry.path.clone();
            self.load(path, false);
        } else if self.camera.autopilot.is_none() {
            self.entering = None;
        }
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let t0 = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let time = (now - self.start).as_secs_f32();

        self.handle_messages();
        let t_msgs = t0.elapsed();

        // Fly
        let k = |c: KeyCode| self.keys.contains(&c);
        let axis = |pos: bool, neg: bool| (pos as i32 - neg as i32) as f32;
        let input = FlyInput {
            forward: axis(k(KeyCode::KeyW) || k(KeyCode::ArrowUp), k(KeyCode::KeyS) || k(KeyCode::ArrowDown)),
            right: axis(k(KeyCode::KeyD) || k(KeyCode::ArrowRight), k(KeyCode::KeyA) || k(KeyCode::ArrowLeft)),
            up: axis(k(KeyCode::KeyE) || k(KeyCode::Space), k(KeyCode::KeyQ) || k(KeyCode::ControlLeft)),
            boost: k(KeyCode::ShiftLeft) || k(KeyCode::ShiftRight),
            look_dx: std::mem::take(&mut self.look.0),
            look_dy: std::mem::take(&mut self.look.1),
        };
        let manual = input.forward != 0.0 || input.right != 0.0 || input.up != 0.0 || input.look_dx != 0.0 || input.look_dy != 0.0;
        if manual && self.autofly.is_some() {
            self.autofly = None; // you've got it
        }
        match (&mut self.autofly, &self.city) {
            (Some(af), Some(city)) => {
                let speed = (self.camera.speed * 0.75).max(10.0); // the wheel still sets the pace
                let (pos, look) = af.step(city, dt, speed);
                self.camera.steer(pos, look, dt);
            }
            _ => {
                self.camera.update(&input, dt);
                self.check_entered();
            }
        }

        // Hover (not through the HUD) + tower animation
        let (w, h) = self.renderer.size();
        let (cx, cy) = self.cursor;
        self.hot = self.hud.hits.iter().position(|(r, _)| r.contains(cx, cy));
        let over_hud = self.hud.covers(cx, cy);
        if let Some(city) = &mut self.city {
            let (o, d) = self.camera.ray(cx as f32, cy as f32, w as f32, h as f32);
            if !self.dragging {
                let hit = if over_hud { None } else { city.pick(o, d) };
                self.hovered = hit.map(|h| h.0);
                self.hovered_name = hit.and_then(|(i, p)| {
                    let t = &city.towers[i];
                    if t.entry.is_dir { nav::line_at(t, p) } else { None }
                });
            }
            for (i, t) in city.towers.iter_mut().enumerate() {
                if t.layer.is_some() {
                    t.appear = (t.appear + dt * 1.5).min(1.0);
                }
                let target = if Some(i) == self.hovered || Some(i) == self.entering { 1.0 } else { 0.0 };
                t.hover += (target - t.hover) * (1.0 - (-dt * 10.0).exp());
                let sel = if Some(i) == self.selected { 1.0 } else { 0.0 };
                t.select += (sel - t.select) * (1.0 - (-dt * 6.0).exp());
                let m = self.matches.get(i).copied().unwrap_or(0.0);
                let searching = self.search.as_ref().is_some_and(|q| !q.is_empty());
                // Filters: by file type, and (live) by search when its FILTER is on.
                let by_type = self.filter.is_empty() || self.filter.contains(&t.cat);
                let by_search = !(self.search_filter && searching && m == 0.0);
                let shown = if by_type && by_search { 1.0 } else { 0.0 };
                t.shown += (shown - t.shown) * (1.0 - (-dt * 7.0).exp());
                let dim = if searching && m == 0.0 && !self.search_filter { 1.0 } else { 0.0 };
                t.found += (m - t.found) * (1.0 - (-dt * 8.0).exp());
                t.dim += (dim - t.dim) * (1.0 - (-dt * 8.0).exp());
            }
        }
        self.play_videos(time);

        let icon = if self.hot.is_some() || self.hovered.is_some() { CursorIcon::Pointer } else { CursorIcon::Default };
        if icon != self.cursor_icon {
            self.cursor_icon = icon;
            self.window.set_cursor(icon);
        }

        // The hovered name gets lit up on its tower face.
        let hl = match (&self.city, self.hovered, self.hovered_name) {
            (Some(c), Some(i), Some(n)) => {
                Some(render::Highlight { layer: c.towers[i].first_layer + n.page, rect: n.rect })
            }
            _ => None,
        };

        // Viewer: take the picture / video frame due now, and where to draw it.
        let mut vd = None;
        if let Some(v) = &mut self.viewer {
            v.update();
            if let Some(f) = v.frame.take() {
                self.renderer.viewer_image(f.w, f.h, &f.px);
            }
        }
        if let Some(v) = &self.viewer {
            vd = Some(render::ViewerDraw { effect: v.effect as u32, time, open: v.open_amount(), rect: self.viewer_rect(w, h) });
        }

        let view_proj = self.camera.proj(w as f32 / h.max(1) as f32) * self.camera.view();
        let t1 = Instant::now();
        self.update_hud(w, h);
        let t_hud = t1.elapsed();
        self.renderer.render(view_proj, self.camera.pos, time, self.city.as_ref(), hl, vd);
        let total = t0.elapsed();
        if total > Duration::from_millis(40) {
            log::warn!(
                "slow frame {:.1} ms (messages/uploads {:.1}, hud {:.1}, render {:.1})",
                total.as_secs_f64() * 1e3,
                t_msgs.as_secs_f64() * 1e3,
                t_hud.as_secs_f64() * 1e3,
                (total - t_hud - t_msgs).as_secs_f64() * 1e3
            );
        }

        self.fps_frames += 1;
        if (now - self.fps_time).as_secs_f32() >= 0.5 {
            self.fps = (self.fps_frames as f32 / (now - self.fps_time).as_secs_f32()).round() as u32;
            self.fps_frames = 0;
            self.fps_time = now;
            self.window.set_title(&format!("GIBSON — {}  — {} fps", self.path.display(), self.fps));
        }
    }

    fn update_hud(&mut self, w: u32, h: u32) {
        if self.message.as_ref().is_some_and(|m| m.2.elapsed() > Duration::from_secs(4)) {
            self.message = None;
        }
        let (status, warn) = if let Some((m, err, _)) = &self.message {
            (m.clone(), *err)
        } else if self.loading.is_some() {
            ("SCANNING…".to_string(), false)
        } else {
            match &self.city {
                None => ("SCANNING…".to_string(), false),
                Some(c) if c.towers.is_empty() => ("EMPTY FOLDER".to_string(), false),
                Some(c) => {
                    let done = c.towers.iter().filter(|t| t.layer.is_some()).count();
                    let s = if done < c.towers.len() { format!("BUILDING {done}/{}", c.towers.len()) } else { String::new() };
                    (s, false)
                }
            }
        };
        let hover = match (&self.city, self.hovered) {
            (Some(c), Some(i)) => Some(match self.hovered_name.and_then(|n| c.towers[i].entry.lines.get(n.line)) {
                Some((name, is_dir)) => {
                    let what = if *is_dir { "FOLDER" } else { "FILE" };
                    let how = if *is_dir { "  ·  DOUBLE-CLICK TO OPEN" } else { "" };
                    hud::HoverInfo {
                        name: name.clone(),
                        detail: format!("{what} IN {}{how}", c.towers[i].entry.name.to_uppercase()),
                    }
                }
                None => hover_info(&c.towers[i].entry),
            }),
            _ => None,
        };
        let selected = match (&self.city, self.selected) {
            (Some(c), Some(i)) if self.loading.is_none() => {
                let e = &c.towers[i].entry;
                Some((e.name.clone(), e.path.clone(), e.is_dir))
            }
            _ => None,
        };
        let st = hud::HudState {
            path: self.loading.clone().unwrap_or_else(|| self.path.clone()),
            drives: self.drives.clone(),
            can_back: !self.history.is_empty(),
            can_up: self.path.parent().is_some(),
            fps: self.fps,
            search: self.search.clone().map(|q| (q, self.match_order.len())),
            help: self.help,
            search_filter: self.search_filter,
            cats: self.city.as_ref().map(|c| c.cats.clone()).unwrap_or_default(),
            filter: self.filter.clone(),
            tower_fx: self.settings.tower_fx,
            autofly: self.autofly.is_some(),
            viewer: self.viewer.as_ref().map(|v| hud::ViewerHud {
                name: v.path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                index: v.index,
                count: v.items.len(),
                size: v.size,
                effect: v.effect,
                video: v.video.as_ref().map(|vi| {
                    let t = match self.scrub_drag {
                        Some(f) => f * vi.duration,
                        None => vi.time(),
                    };
                    (t, vi.duration, vi.paused)
                }),
                sound: v
                    .video
                    .as_ref()
                    .filter(|vi| vi.audio.is_some())
                    .map(|_| ((v.volume * 100.0).round() as u32, v.muted)),
                error: v.error.clone(),
            }),
            hover,
            selected,
            status,
            warn,
            hot: self.hot,
        };
        // Hover changes every frame while flying past names: cap the redraw rate.
        if self.hud_drawn.elapsed() < HUD_MIN_INTERVAL && self.hud.w == w && self.hud.h == h {
            return;
        }
        let scale = self.window.scale_factor() as f32;
        if self.hud.update(&st, w, h, scale, &self.fonts) {
            self.hud_drawn = Instant::now();
            self.renderer.upload_hud(w, h, &self.hud.px);
        }
    }
}

fn hover_info(e: &fs_scan::Entry) -> hud::HoverInfo {
    let date = fs_scan::format_date(e.modified);
    let detail = if e.is_dir {
        let more = if e.capped { "+" } else { "" };
        format!("FOLDER  ·  {}{more} ITEMS  ·  {}  ·  {date}", e.child_count, fs_scan::format_size(e.size))
    } else {
        let ext = Path::new(&e.name).extension().map(|x| x.to_string_lossy().to_uppercase() + " ").unwrap_or_default();
        format!("{ext}FILE  ·  {}  ·  {date}", fs_scan::format_size(e.size))
    };
    hud::HoverInfo { name: e.name.clone(), detail }
}

/// A tower's baked pages (and preview), waiting to be uploaded.
struct Baked {
    index: usize,
    pages: Vec<textgen::Page>,
    cached: bool,
    preview: Option<Arc<preview::Preview>>,
    duration: f32,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        // 1600×900 points, or less to fit the screen (logical size: right on Retina too).
        let (mut lw, mut lh) = (1600.0, 900.0);
        if let Some(m) = event_loop.primary_monitor() {
            let s = m.size().to_logical::<f64>(m.scale_factor());
            let k = (s.width * 0.85 / lw).min(s.height * 0.85 / lh).min(1.0);
            (lw, lh) = (lw * k, lh * k);
        }
        let attrs = Window::default_attributes().with_title("GIBSON").with_inner_size(LogicalSize::new(lw, lh));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let renderer = pollster::block_on(render::Renderer::new(window.clone()));
        let (tx, rx) = channel();
        {
            // Drive letters can be slow to answer (network, card readers): ask off-thread.
            let tx = tx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Drives(fs_scan::drives()));
            });
            std::thread::spawn(preview::have_ffmpeg);
        }
        let now = Instant::now();
        let mut s = Running {
            window,
            renderer,
            camera: Camera::new(),
            city: None,
            path: self.start_path.clone(),
            loading: None,
            loading_back: false,
            came_from: None,
            history: Vec::new(),
            generation: 0,
            live_gen: Arc::new(AtomicU64::new(0)),
            bake_pool: Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(2).max(1)))
                    .thread_name(|i| format!("bake-{i}"))
                    .build()
                    .expect("bake pool"),
            ),
            tx,
            rx,
            fonts: Arc::new(textgen::Fonts::load()),
            pending: Vec::new(),
            clips: HashMap::new(),
            clip_loading: HashSet::new(),
            clip_failed: HashSet::new(),
            cache: cache::PageCache::default(),
            hud: hud::Hud::new(),
            drives: Vec::new(),
            message: None,
            keys: HashSet::new(),
            dragging: false,
            press: None,
            last_cursor: None,
            cursor: (0.0, 0.0),
            look: (0.0, 0.0),
            hovered: None,
            hovered_name: None,
            selected: None,
            hot: None,
            last_click: None,
            entering: None,
            help: false,
            filter: Vec::new(),
            search_filter: false,
            viewer: None,
            effect: 1,
            settings: settings::Settings::default(),
            scrub_drag: None,
            view_on_load: None,
            autofly: None,
            search: None,
            matches: Vec::new(),
            match_order: Vec::new(),
            match_next: 0,
            cursor_icon: CursorIcon::Default,
            hud_drawn: now,
            start: now,
            last_frame: now,
            fps_frames: 0,
            fps_time: now,
            fps: 0,
            city_since: now,
            stuck_logged: false,
        };
        s.settings = settings::load();
        s.effect = s.settings.viewer_fx;
        s.renderer.tower_fx = s.settings.tower_fx as u32;
        let (gw, gh, gpx) = textgen::glyph_strip(&s.fonts);
        s.renderer.set_glyph_atlas(gw, gh, &gpx);
        s.view_on_load = self.start_file.take();
        s.load(self.start_path.clone(), false);
        self.state = Some(s);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(s) = &mut self.state else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => s.renderer.resize(size.width, size.height),
            WindowEvent::KeyboardInput { event, .. } => {
                if s.viewer.is_some() {
                    if let PhysicalKey::Code(code) = event.physical_key {
                        match event.state {
                            ElementState::Pressed => {
                                s.viewer_key(&event);
                                s.keys.insert(code);
                            }
                            ElementState::Released => {
                                s.keys.remove(&code);
                            }
                        }
                    }
                    return;
                }
                if s.search.is_some() && event.state == ElementState::Pressed {
                    s.search_key(&event);
                    return;
                }
                if let PhysicalKey::Code(code) = event.physical_key {
                    match event.state {
                        ElementState::Pressed => {
                            match code {
                                KeyCode::Escape if s.help => s.help = false,
                                KeyCode::Escape => {
                                    s.selected = None;
                                    s.autofly = None;
                                }
                                KeyCode::F1 if !event.repeat => s.help = !s.help,
                                KeyCode::Slash
                                    if !event.repeat
                                        && (s.keys.contains(&KeyCode::ShiftLeft) || s.keys.contains(&KeyCode::ShiftRight)) =>
                                {
                                    s.help = !s.help
                                }
                                KeyCode::Slash if !event.repeat => s.open_search(),
                                KeyCode::KeyF if !event.repeat && s.command_held() => s.open_search(),
                                // macOS: Cmd+[ / Cmd+Left back, Cmd+Up up (as in Finder)
                                KeyCode::BracketLeft if !event.repeat && s.command_held() => s.go_back(),
                                KeyCode::ArrowLeft
                                    if !event.repeat
                                        && (s.keys.contains(&KeyCode::SuperLeft) || s.keys.contains(&KeyCode::SuperRight)) =>
                                {
                                    s.go_back()
                                }
                                KeyCode::ArrowUp
                                    if !event.repeat
                                        && (s.keys.contains(&KeyCode::SuperLeft) || s.keys.contains(&KeyCode::SuperRight)) =>
                                {
                                    s.go_up()
                                }
                                KeyCode::KeyR if !event.repeat => s.reveal(),
                                KeyCode::KeyV if !event.repeat => s.view_target(),
                                KeyCode::KeyP if !event.repeat => s.toggle_autofly(),
                                KeyCode::KeyT if !event.repeat => {
                                    let back = s.keys.contains(&KeyCode::ShiftLeft) || s.keys.contains(&KeyCode::ShiftRight);
                                    s.step_tower_fx(if back { -1 } else { 1 });
                                }
                                KeyCode::Backspace if !event.repeat => s.go_up(),
                                KeyCode::BrowserBack if !event.repeat => s.go_back(),
                                KeyCode::ArrowLeft
                                    if !event.repeat
                                        && (s.keys.contains(&KeyCode::AltLeft) || s.keys.contains(&KeyCode::AltRight)) =>
                                {
                                    s.go_back()
                                }
                                KeyCode::Enter | KeyCode::NumpadEnter if !event.repeat => {
                                    if let Some(i) = s.hovered.or(s.selected) {
                                        s.enter(i);
                                    }
                                }
                                _ => {}
                            }
                            s.keys.insert(code);
                        }
                        ElementState::Released => {
                            s.keys.remove(&code);
                        }
                    }
                }
            }
            WindowEvent::Focused(false) => {
                s.keys.clear();
                s.dragging = false;
                s.press = None;
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Back, .. } => s.go_back(),
            WindowEvent::MouseInput { state, button, .. } => {
                if s.viewer.is_some() && button == MouseButton::Left {
                    let down = state == ElementState::Pressed;
                    let on_bar = s.hud.scrub.is_some_and(|r| r.contains(s.cursor.0, s.cursor.1));
                    if down && on_bar {
                        s.scrub_drag = Some(s.scrub_fraction());
                    } else if !down && s.scrub_drag.is_some() {
                        let f = s.scrub_fraction();
                        s.scrub_drag = None;
                        if let Some(v) = &mut s.viewer {
                            if let Some(d) = v.video.as_ref().map(|vi| vi.duration) {
                                v.seek(f * d);
                            }
                        }
                    } else if !down {
                        s.click();
                    }
                    return;
                }
                if s.viewer.is_some() {
                    return;
                }
                if matches!(button, MouseButton::Left | MouseButton::Right) {
                    let down = state == ElementState::Pressed;
                    s.dragging = down;
                    s.last_cursor = None;
                    if button == MouseButton::Left {
                        if down {
                            s.press = Some(s.cursor);
                        } else if let Some((px, py)) = s.press.take() {
                            if (s.cursor.0 - px).hypot(s.cursor.1 - py) < CLICK_SLOP {
                                s.click();
                            }
                        }
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                s.cursor = (position.x, position.y);
                if s.scrub_drag.is_some() {
                    s.scrub_drag = Some(s.scrub_fraction());
                }
                if s.dragging {
                    if let Some((lx, ly)) = s.last_cursor {
                        s.look.0 += (position.x - lx) as f32;
                        s.look.1 += (position.y - ly) as f32;
                    }
                    s.last_cursor = Some((position.x, position.y));
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => (p.y as f32 / 240.0).clamp(-1.0, 1.0), // trackpads
                };
                s.camera.speed = (s.camera.speed * 1.15f32.powf(y)).clamp(3.0, 400.0);
            }
            WindowEvent::RedrawRequested => {
                if s.renderer.size().0 > 0 {
                    s.frame();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(s) = &self.state {
            s.window.request_redraw();
        }
    }
}

fn main() {
    // Log to stderr when there's a terminal (debug builds), and always to
    // gibson.log next to the settings, so a double-clicked app leaves a trace.
    let mut logger =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"));
    if !cfg!(debug_assertions) {
        if let Some(dir) = settings::dir() {
            let _ = std::fs::create_dir_all(&dir);
            if let Ok(f) = std::fs::File::create(dir.join("gibson.log")) {
                logger.target(env_logger::Target::Pipe(Box::new(f)));
            }
        }
    }
    logger.init();
    let start_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    log::info!("starting in {}", start_path.display());

    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    // Given a file: open its folder, and the viewer on it (for pictures and videos).
    let (start_path, start_file) = if start_path.is_file() {
        let folder = start_path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        let view = preview::kind(&start_path).is_some().then_some(start_path);
        (folder, view)
    } else {
        (start_path, None)
    };
    let mut app = App { start_path, start_file, state: None };
    event_loop.run_app(&mut app).expect("run");
}
