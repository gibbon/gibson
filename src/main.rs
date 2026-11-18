// Gibson — fly through your file system as a city of glass towers.

mod camera;
mod city;
mod fs_scan;
mod render;
mod textgen;

use camera::{Camera, FlyInput};
use city::City;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// Work coming back from the loader threads.
enum Msg {
    Scanned { generation: u64, entries: Vec<fs_scan::Entry> },
    Baked { generation: u64, index: usize, pages: Vec<Vec<u8>> }, // one mip chain per page
}

const PAGE_UPLOADS_PER_FRAME: usize = 12;

struct Running {
    window: Arc<Window>,
    renderer: render::Renderer,
    camera: Camera,
    city: Option<City>,
    path: PathBuf,
    generation: u64,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    fonts: Arc<textgen::Fonts>,
    pending: Vec<(usize, Vec<Vec<u8>>)>, // baked pages, waiting for upload
    keys: HashSet<KeyCode>,
    dragging: bool,
    last_cursor: Option<(f64, f64)>,
    cursor: (f64, f64),
    look: (f32, f32),
    hovered: Option<usize>,
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
    /// Scans a folder on a worker thread; towers appear as their faces finish baking.
    fn load(&mut self, path: PathBuf) {
        self.generation += 1;
        self.path = path.clone();
        self.city = None;
        self.pending.clear();
        self.hovered = None;
        let (tx, generation) = (self.tx.clone(), self.generation);
        std::thread::spawn(move || {
            let entries = fs_scan::scan(&path);
            let _ = tx.send(Msg::Scanned { generation, entries });
        });
    }

    fn handle_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Scanned { generation, entries } if generation == self.generation => {
                    let city = City::build(entries, self.renderer.max_face_layers);
                    self.renderer.reserve_face_layers(city.layers());
                    // Start at the front edge of the city, low, looking down the main lane.
                    self.camera = Camera::new();
                    self.camera.pos = glam::Vec3::new(city.origin.x - city::CELL * 0.5, 9.0, city.half_extent.y + 45.0);
                    // Bake every face in parallel, off the UI thread.
                    let jobs: Vec<(usize, fs_scan::Entry, f32, u32)> = city
                        .towers
                        .iter()
                        .enumerate()
                        .map(|(i, t)| (i, t.entry.clone(), t.seed, t.max_pages))
                        .collect();
                    let (tx, fonts) = (self.tx.clone(), self.fonts.clone());
                    std::thread::spawn(move || {
                        jobs.into_par_iter().for_each(|(index, entry, seed, max_pages)| {
                            let pages = textgen::bake(&entry, seed, &fonts, max_pages);
                            let _ = tx.send(Msg::Baked { generation, index, pages });
                        });
                    });
                    log::info!(
                        "city: {} towers in {}x{}, {} face pages reserved",
                        city.towers.len(),
                        city.cols,
                        city.rows,
                        city.layers()
                    );
                    self.city = Some(city);
                }
                Msg::Baked { generation, index, pages } if generation == self.generation => {
                    self.pending.push((index, pages));
                }
                _ => {} // stale: from a folder we've already left
            }
        }
        // Upload a few per frame so loading never stutters.
        if let Some(city) = &mut self.city {
            let mut uploaded = 0;
            while uploaded < PAGE_UPLOADS_PER_FRAME {
                let Some((index, pages)) = self.pending.pop() else { break };
                let t = &mut city.towers[index];
                for (p, mips) in pages.iter().take(t.max_pages as usize).enumerate() {
                    self.renderer.upload_face(t.first_layer + p as u32, mips);
                }
                t.pages = (pages.len() as u32).min(t.max_pages);
                t.layer = Some(t.first_layer);
                uploaded += pages.len();
            }
        }
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let time = (now - self.start).as_secs_f32();

        self.handle_messages();

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

        // Hover + tower animation
        let (w, h) = self.renderer.size();
        if let Some(city) = &mut self.city {
            let (o, d) = self.camera.ray(self.cursor.0 as f32, self.cursor.1 as f32, w as f32, h as f32);
            self.hovered = if self.dragging { self.hovered } else { city.pick(o, d) };
            for (i, t) in city.towers.iter_mut().enumerate() {
                if t.layer.is_some() {
                    t.appear = (t.appear + dt * 1.5).min(1.0);
                }
                let target = if Some(i) == self.hovered { 1.0 } else { 0.0 };
                t.hover += (target - t.hover) * (1.0 - (-dt * 10.0).exp());
            }
        }

        let view_proj = self.camera.proj(w as f32 / h.max(1) as f32) * self.camera.view();
        self.renderer.render(view_proj, self.camera.pos, time, self.city.as_ref());

        // Title: where we are, what's under the cursor, fps.
        self.fps_frames += 1;
        if (now - self.fps_time).as_secs_f32() >= 0.5 {
            self.fps = (self.fps_frames as f32 / (now - self.fps_time).as_secs_f32()).round() as u32;
            self.fps_frames = 0;
            self.fps_time = now;
            let hover = match (&self.city, self.hovered) {
                (Some(c), Some(i)) => {
                    let e = &c.towers[i].entry;
                    if e.is_dir {
                        format!("  ▸ {}  ({} items)", e.name, e.child_count)
                    } else {
                        format!("  ▸ {}  ({})", e.name, fs_scan::format_size(e.size))
                    }
                }
                _ => String::new(),
            };
            let loading = match &self.city {
                None => "  [scanning…]".to_string(),
                Some(c) => {
                    let done = c.towers.iter().filter(|t| t.layer.is_some()).count();
                    if done < c.towers.len() { format!("  [building {done}/{}]", c.towers.len()) } else { String::new() }
                }
            };
            self.window.set_title(&format!("GIBSON — {}{loading}{hover}  — {} fps", self.path.display(), self.fps));
        }
    }
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
        let now = Instant::now();
        let mut s = Running {
            window,
            renderer,
            camera: Camera::new(),
            city: None,
            path: PathBuf::new(),
            generation: 0,
            tx,
            rx,
            fonts: Arc::new(textgen::Fonts::load()),
            pending: Vec::new(),
            keys: HashSet::new(),
            dragging: false,
            last_cursor: None,
            cursor: (0.0, 0.0),
            look: (0.0, 0.0),
            hovered: None,
            start: now,
            last_frame: now,
            fps_frames: 0,
            fps_time: now,
            fps: 0,
        };
        s.load(self.start_path.clone());
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
                            if code == KeyCode::Escape {
                                event_loop.exit();
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
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if matches!(button, MouseButton::Left | MouseButton::Right) {
                    s.dragging = state == ElementState::Pressed;
                    s.last_cursor = None;
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
