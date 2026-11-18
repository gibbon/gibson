// Colour previews for image and video towers: a picture (or video frames) fitted
// into a PV×PV RGBA layer, top-left aligned, with a full mip chain. Videos are
// decoded by the FFmpeg command-line tool when it's installed.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

pub const PV: u32 = 512; // preview layer size
pub const PV_LAYERS: u32 = 64; // previews per city (~1.3 MB of GPU memory each)
pub const PV_MIPS: u32 = 10; // 512 → 1
pub const CLIP_FPS: f32 = 12.0;
const CLIP_SECONDS: f32 = 2.0;
const MAX_IMAGE_BYTES: u64 = 80 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PvKind {
    Image,
    Video,
}

pub fn kind(p: &Path) -> Option<PvKind> {
    let ext = p.extension()?.to_string_lossy().to_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" => Some(PvKind::Image),
        "mp4" | "mkv" | "mov" | "webm" | "avi" | "m4v" | "wmv" | "mpg" | "mpeg" | "ts" | "flv" => Some(PvKind::Video),
        _ => None,
    }
}

/// A picture ready for upload: `w`×`h` pixels at the top-left of a PV×PV layer.
pub struct Preview {
    pub w: u32,
    pub h: u32,
    pub mips: Vec<u8>, // RGBA, all levels of the PV×PV layer
}

impl Preview {
    pub fn bytes(&self) -> usize {
        self.mips.len()
    }
}

/// Builds a preview from `w`×`h` RGBA pixels (w, h ≤ PV).
fn from_rgba(w: u32, h: u32, px: &[u8]) -> Preview {
    let mut base = vec![0u8; (PV * PV * 4) as usize];
    for y in 0..h as usize {
        let src = &px[y * w as usize * 4..(y + 1) * w as usize * 4];
        base[y * PV as usize * 4..y * PV as usize * 4 + src.len()].copy_from_slice(src);
    }
    Preview { w, h, mips: mip_chain(&base) }
}

fn mip_chain(base: &[u8]) -> Vec<u8> {
    let mut out = base.to_vec();
    let mut prev = base.to_vec();
    let mut size = PV as usize;
    for _ in 1..PV_MIPS {
        let n = (size / 2).max(1);
        let mut next = vec![0u8; n * n * 4];
        for y in 0..n {
            for x in 0..n {
                for c in 0..4 {
                    let s = |dx: usize, dy: usize| prev[((y * 2 + dy) * size + x * 2 + dx) * 4 + c] as u32;
                    next[(y * n + x) * 4 + c] = ((s(0, 0) + s(1, 0) + s(0, 1) + s(1, 1) + 2) / 4) as u8;
                }
            }
        }
        out.extend_from_slice(&next);
        prev = next;
        size = n;
    }
    out
}

/// Decodes an image file into a preview.
pub fn image(p: &Path) -> Option<Preview> {
    if std::fs::metadata(p).ok()?.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let mut reader = image::ImageReader::open(p).ok()?.with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| log::debug!("preview {}: {e}", p.display())).ok()?;
    let thumb = img.thumbnail(PV, PV).to_rgba8();
    Some(from_rgba(thumb.width(), thumb.height(), thumb.as_raw()))
}

/// Non-blocking: true once the check has run and found FFmpeg.
pub fn ffmpeg_ready() -> bool {
    FFMPEG.get().copied().unwrap_or(false)
}

static FFMPEG: OnceLock<bool> = OnceLock::new();

/// Is the `ffmpeg` command available? (Checked once.)
pub fn have_ffmpeg() -> bool {
    *FFMPEG.get_or_init(|| {
        let ok = ffmpeg().arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
        log::info!("ffmpeg {}", if ok { "found: videos will play" } else { "not found: no video previews" });
        ok
    })
}

fn ffmpeg() -> Command {
    let mut c = Command::new("ffmpeg");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// Where in a video the still and the clip come from: a little way in (skips
/// black intros), or the start for short videos.
const SEEKS: [f32; 2] = [20.0, 0.0];

/// Runs FFmpeg to decode frames (scaled to PV wide, at most PV tall) as raw RGBA.
fn decode(p: &Path, seek: f32, args: &[&str]) -> Option<Vec<u8>> {
    let filter = format!("{}scale={PV}:-2,crop=iw:'min(ih,{PV})':0:0", args.first().copied().unwrap_or(""));
    let mut child = ffmpeg()
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-ss", &seek.to_string(), "-i"])
        .arg(p)
        .args(&args[1..])
        .args(["-an", "-vf", &filter, "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = Vec::new();
    child.stdout.take()?.read_to_end(&mut out).ok()?;
    let _ = child.wait();
    if out.is_empty() { None } else { Some(out) }
}

/// One frame from a video, and the seek it came from.
pub fn video_still(p: &Path) -> Option<(Preview, f32)> {
    if !have_ffmpeg() {
        return None;
    }
    for seek in SEEKS {
        if let Some(px) = decode(p, seek, &["", "-frames:v", "1"]) {
            let h = (px.len() / (PV as usize * 4)) as u32;
            if h > 0 {
                return Some((from_rgba(PV, h, &px[..(PV * h * 4) as usize]), seek));
            }
        }
    }
    None
}

/// A short looping clip: CLIP_SECONDS of frames at CLIP_FPS from `seek`.
pub fn video_clip(p: &Path, seek: f32, h: u32) -> Option<Vec<Preview>> {
    if !have_ffmpeg() || h == 0 {
        return None;
    }
    let fps = format!("fps={CLIP_FPS},");
    let t = CLIP_SECONDS.to_string();
    let px = decode(p, seek, &[&fps, "-t", &t])?;
    let frame = (PV * h * 4) as usize;
    let frames: Vec<Preview> = px.chunks_exact(frame).map(|f| from_rgba(PV, h, f)).collect();
    if frames.is_empty() { None } else { Some(frames) }
}

/// Where the picture goes on a tower's page, in page pixels (shared by the page
/// baker, which leaves the space, and the renderer, which draws into it).
pub fn page_rect(w: u32, h: u32) -> (f32, f32, f32, f32) {
    let (max_w, max_h, top) = (484.0, 560.0, 104.0);
    let ar = h.max(1) as f32 / w.max(1) as f32;
    let (mut ww, mut hh) = (max_w, max_w * ar);
    if hh > max_h {
        hh = max_h;
        ww = max_h / ar;
    }
    let x0 = 256.0 - ww / 2.0;
    (x0, top, ww, hh)
}
