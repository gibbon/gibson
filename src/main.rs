// Gibson — fly through your file system as a city of glass towers.

mod camera;
mod city;
mod fs_scan;
mod hud;
mod nav;
mod render;
mod textgen;

use camera::{Camera, FlyInput};
use city::City;
use glam::Vec3;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowId};

/// Work coming back from the loader threads.
enum Msg {
    Scanned { generation: u64, path: PathBuf, result: Result<Vec<fs_scan::Entry>, String> },
    Baked { generation: u64, index: usize, pages: Vec<textgen::Page> },
    Drives(Vec<PathBuf>),
}

const PAGE_UPLOADS_PER_FRAME: usize = 12;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const CLICK_SLOP: f64 = 5.0; // pixels the mouse may move and still count as a click
const HUD_MIN_INTERVAL: Duration = Duration::from_millis(33); // HUD redraws at most ~30/s
const HISTORY_MAX: usize = 100;

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
    pending: Vec<(usize, Vec<textgen::Page>)>, // baked pages, waiting for upload
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
    cursor_icon: CursorIcon,
    hud_drawn: Instant,
    start: Instant,
    last_frame: Instant,
    fps_frames: u32,
    fps_time: Instant,
    fps: u32,
}

struct App {
    start_path: PathBuf,
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
                Msg::Baked { generation, index, pages } if generation == self.generation => {
                    self.pending.push((index, pages));
                }
                Msg::Drives(d) => self.drives = d,
                _ => {} // stale: from a folder we've already left
            }
        }
        // Upload a few pages per frame so loading never stutters.
        if let Some(city) = &mut self.city {
            let mut uploaded = 0;
            while uploaded < PAGE_UPLOADS_PER_FRAME {
                let Some((index, pages)) = self.pending.pop() else { break };
                let t = &mut city.towers[index];
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
                let eye = Vec3::new(city.origin.x - city::CELL * 0.5, 9.0, city.half_extent.y + 45.0);
                self.camera.place(eye, eye + Vec3::new(0.0, -0.1, -1.0));
            }
        }

        // Bake every face in parallel, off the UI thread.
        let generation = self.generation;
        let jobs: Vec<(usize, fs_scan::Entry, f32, u32)> =
            city.towers.iter().enumerate().map(|(i, t)| (i, t.entry.clone(), t.seed, t.max_pages)).collect();
        // On a pool that leaves cores free for flying, and skipping work as soon as
        // we've moved on to another folder.
        let (tx, fonts, live) = (self.tx.clone(), self.fonts.clone(), self.live_gen.clone());
        self.bake_pool.spawn(move || {
            jobs.into_par_iter().for_each(|(index, entry, seed, max_pages)| {
                if live.load(Ordering::Relaxed) != generation {
                    return;
                }
                let pages = textgen::bake(&entry, seed, &fonts, max_pages);
                let _ = tx.send(Msg::Baked { generation, index, pages });
            });
        });
        log::info!("city: {} towers in {}x{}, {} face pages", city.towers.len(), city.cols, city.rows, city.layers());
        self.city = Some(city);
        self.path = path;
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
            }
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
        if *is_dir {
            let path = e.path.join(name);
            self.load(path, false);
        } else {
            self.say("FILES OPEN IN THE NEXT MILESTONE", false);
        }
    }

    fn fly_to(&mut self, i: usize) {
        let Some(city) = &self.city else { return };
        let (eye, look) = nav::view_point(&city.towers[i], self.camera.pos);
        self.camera.fly_to(eye, look);
        self.entering = None;
    }

    /// Fly into a folder tower; arriving inside opens it.
    fn enter(&mut self, i: usize) {
        let Some(city) = &self.city else { return };
        let t = &city.towers[i];
        if !t.entry.is_dir {
            self.say("FILES OPEN IN THE NEXT MILESTONE", false);
            self.fly_to(i);
            return;
        }
        let (eye, look) = nav::enter_point(t, self.camera.pos);
        self.camera.fly_to(eye, look);
        self.entering = Some(i);
    }

    /// Opens a folder once the camera is inside its tower (flown in, or by autopilot).
    fn check_entered(&mut self) {
        if self.loading.is_some() {
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
        self.camera.update(&input, dt);
        self.check_entered();

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
            }
        }
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

        let view_proj = self.camera.proj(w as f32 / h.max(1) as f32) * self.camera.view();
        let t1 = Instant::now();
        self.update_hud(w, h);
        let t_hud = t1.elapsed();
        self.renderer.render(view_proj, self.camera.pos, time, self.city.as_ref(), hl);
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

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title("GIBSON").with_inner_size(PhysicalSize::new(1600, 900));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let renderer = pollster::block_on(render::Renderer::new(window.clone()));
        let (tx, rx) = channel();
        {
            // Drive letters can be slow to answer (network, card readers): ask off-thread.
            let tx = tx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Drives(fs_scan::drives()));
            });
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
            cursor_icon: CursorIcon::Default,
            hud_drawn: now,
            start: now,
            last_frame: now,
            fps_frames: 0,
            fps_time: now,
            fps: 0,
        };
        s.load(self.start_path.clone(), false);
        self.state = Some(s);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(s) = &mut self.state else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => s.renderer.resize(size.width, size.height),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    match event.state {
                        ElementState::Pressed => {
                            match code {
                                KeyCode::Escape => event_loop.exit(),
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
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 60.0,
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
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"))
        .init();
    let start_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    log::info!("starting in {}", start_path.display());

    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App { start_path, state: None };
    event_loop.run_app(&mut app).expect("run");
}
