// The media viewer: an image or video full-screen over the city, with switchable
// effects. Images are decoded at full size; videos stream through FFmpeg (no sound)
// at the size they'll be shown, and seeking restarts the stream at the new time.

use crate::preview::PvKind;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, TryRecvError};
use std::time::Instant;

pub const EFFECTS: [&str; 6] = ["CLEAN", "HOLOGRAM", "CRT", "THERMAL", "DATA MOSAIC", "EDGE GLOW"];
const VIDEO_FPS: f32 = 30.0;
const MAX_IMAGE: u32 = 4096;

/// A decoded picture, RGBA.
pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

pub struct Viewer {
    pub items: Vec<PathBuf>, // the images / videos you can step through
    pub index: usize,
    pub effect: usize,
    pub opened: Instant,
    pub video: Option<Video>,
    image_rx: Option<Receiver<Option<Frame>>>,
    pub frame: Option<Frame>, // newest picture, waiting for upload
    pub size: Option<(u32, u32)>, // size of what's on screen
    pub error: Option<String>,
}

pub struct Video {
    pub duration: f32,
    pub w: u32,
    pub h: u32,
    pub paused: bool,
    seek: f32,     // where the current stream started
    played: f32,   // seconds played since `seek`, up to `since`
    since: Instant,
    child: Option<Child>,
    rx: Option<Receiver<Vec<u8>>>,
    shown: u64, // frames taken from the current stream
}

impl Drop for Video {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Video {
    pub fn time(&self) -> f32 {
        let t = self.seek + self.played + if self.paused { 0.0 } else { self.since.elapsed().as_secs_f32() };
        if self.duration > 0.0 { t.min(self.duration) } else { t }
    }

    fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.rx = None;
    }

    fn start(&mut self, path: &Path, at: f32) {
        self.stop();
        self.seek = at.clamp(0.0, (self.duration - 0.1).max(0.0));
        self.played = 0.0;
        self.since = Instant::now();
        self.shown = 0;
        let filter = format!("fps={VIDEO_FPS},scale={}:{}", self.w, self.h);
        let child = tool("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-ss", &format!("{:.2}", self.seek), "-i"])
            .arg(path)
            .args(["-an", "-sn", "-vf", &filter, "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else { return };
        let Some(mut out) = child.stdout.take() else { return };
        let bytes = (self.w * self.h * 4) as usize;
        let (tx, rx) = sync_channel(6); // a few frames ahead; the decoder waits when full
        std::thread::spawn(move || loop {
            let mut buf = vec![0u8; bytes];
            if out.read_exact(&mut buf).is_err() || tx.send(buf).is_err() {
                break;
            }
        });
        self.child = Some(child);
        self.rx = Some(rx);
    }
}

fn tool(name: &str) -> Command {
    let mut c = Command::new(name);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// Width, height and length of a video (ffprobe).
fn probe(p: &Path) -> Option<(u32, u32, f32)> {
    let out = tool("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height:format=duration", "-of", "default=nw=1"])
        .arg(p)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)).map(str::trim);
    let w = field("width=")?.parse().ok()?;
    let h = field("height=")?.parse().ok()?;
    let d = field("duration=").and_then(|d| d.parse().ok()).unwrap_or(0.0);
    Some((w, h, d))
}

impl Viewer {
    pub fn open(items: Vec<PathBuf>, index: usize, effect: usize, screen: (u32, u32)) -> Viewer {
        let mut v = Viewer {
            items,
            index,
            effect,
            opened: Instant::now(),
            video: None,
            image_rx: None,
            frame: None,
            size: None,
            error: None,
        };
        v.load(screen);
        v
    }

    pub fn path(&self) -> &Path {
        &self.items[self.index]
    }

    /// Steps to the next / previous item (wrapping).
    pub fn step(&mut self, by: i32, screen: (u32, u32)) {
        let n = self.items.len() as i32;
        self.index = ((self.index as i32 + by).rem_euclid(n)) as usize;
        self.opened = Instant::now();
        self.load(screen);
    }

    fn load(&mut self, screen: (u32, u32)) {
        self.video = None;
        self.image_rx = None;
        self.frame = None;
        self.size = None;
        self.error = None;
        let path = self.path().to_path_buf();
        match crate::preview::kind(&path) {
            Some(PvKind::Video) => {
                let Some((vw, vh, duration)) = probe(&path) else {
                    self.error = Some("CAN'T READ THIS VIDEO".into());
                    return;
                };
                // Decode at the size it's shown (fits the window, even numbers).
                let k = (screen.0 as f32 / vw as f32).min(screen.1 as f32 / vh as f32).min(1.0);
                let w = ((vw as f32 * k) as u32 & !1).max(2);
                let h = ((vh as f32 * k) as u32 & !1).max(2);
                let mut v = Video {
                    duration,
                    w,
                    h,
                    paused: false,
                    seek: 0.0,
                    played: 0.0,
                    since: Instant::now(),
                    child: None,
                    rx: None,
                    shown: 0,
                };
                let start = if duration > 60.0 { duration * 0.1 } else { 0.0 };
                v.start(&path, start);
                self.size = Some((w, h));
                self.video = Some(v);
            }
            _ => {
                let (tx, rx) = sync_channel(1);
                std::thread::spawn(move || {
                    let _ = tx.send(decode_image(&path));
                });
                self.image_rx = Some(rx);
            }
        }
    }

    /// Call every frame: picks up a decoded image, or the video frame due now.
    pub fn update(&mut self) {
        if let Some(rx) = &self.image_rx {
            match rx.try_recv() {
                Ok(Some(f)) => {
                    self.size = Some((f.w, f.h));
                    self.frame = Some(f);
                    self.image_rx = None;
                }
                Ok(None) | Err(TryRecvError::Disconnected) => {
                    self.error = Some("CAN'T READ THIS IMAGE".into());
                    self.image_rx = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        let path = self.items[self.index].clone();
        let Some(v) = &mut self.video else { return };
        if v.paused {
            return;
        }
        // Take frames until we're at the one due now; keep the newest.
        let due = ((v.time() - v.seek) * VIDEO_FPS) as u64 + 1;
        let mut ended = false;
        while v.shown < due {
            let Some(rx) = &v.rx else { break };
            match rx.try_recv() {
                Ok(px) => {
                    v.shown += 1;
                    self.frame = Some(Frame { w: v.w, h: v.h, px });
                }
                Err(TryRecvError::Empty) => break, // decoder is behind: show what we have
                Err(TryRecvError::Disconnected) => {
                    ended = true;
                    break;
                }
            }
        }
        if ended || (v.duration > 0.0 && v.time() >= v.duration - 0.05) {
            v.start(&path, 0.0); // loop
        }
    }

    pub fn toggle_pause(&mut self) {
        if let Some(v) = &mut self.video {
            if v.paused {
                v.since = Instant::now();
            } else {
                v.played += v.since.elapsed().as_secs_f32();
            }
            v.paused = !v.paused;
        }
    }

    pub fn seek(&mut self, to: f32) {
        let path = self.items[self.index].clone();
        if let Some(v) = &mut self.video {
            let paused = v.paused;
            v.start(&path, to);
            v.paused = paused;
            if paused {
                // Show the frame at the new spot even while paused.
                v.paused = false;
                v.shown = 0;
                if let Some(rx) = &v.rx {
                    if let Ok(px) = rx.recv_timeout(std::time::Duration::from_millis(1500)) {
                        v.shown = 1;
                        self.frame = Some(Frame { w: v.w, h: v.h, px });
                    }
                }
                v.paused = true;
            }
        }
    }

    pub fn seek_by(&mut self, secs: f32) {
        if let Some(v) = &self.video {
            let t = v.time() + secs;
            self.seek(t);
        }
    }

    /// 0 → 1 over the opening transition.
    pub fn open_amount(&self) -> f32 {
        (self.opened.elapsed().as_secs_f32() / 0.35).min(1.0)
    }
}

fn decode_image(p: &Path) -> Option<Frame> {
    let mut reader = image::ImageReader::open(p).ok()?.with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(30_000);
    limits.max_image_height = Some(30_000);
    limits.max_alloc = Some(1024 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    let img = if img.width() > MAX_IMAGE || img.height() > MAX_IMAGE {
        img.resize(MAX_IMAGE, MAX_IMAGE, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Some(Frame { w: rgba.width(), h: rgba.height(), px: rgba.into_raw() })
}

pub fn clock(t: f32) -> String {
    let s = t.max(0.0) as u32;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}
