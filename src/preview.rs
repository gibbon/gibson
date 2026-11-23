// Colour previews for image and video towers: a picture (or video frames) fitted
// into a PV×PV RGBA layer, top-left aligned, with a full mip chain. Videos are
// decoded by the FFmpeg command-line tool when it's installed.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

pub const PV: u32 = 512; // preview layer size
pub const PV_LAYERS: u32 = 64; // previews per city (~1.3 MB of GPU memory each)
pub const PV_MIPS: u32 = 10; // 512 → 1
pub const CLIP_FPS: f32 = 12.0;
const CLIP_SECONDS: f32 = 3.0; // short videos: loop this much from the start
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
/// Only the rows the picture uses are stored (at every mip level).
pub struct Preview {
    pub w: u32,
    pub h: u32,
    pub mips: Vec<u8>, // RGBA, level 0 first; level l is (PV>>l) wide, rows(h, l) tall
}

impl Preview {
    pub fn bytes(&self) -> usize {
        self.mips.len()
    }

    /// (byte offset, width, rows) of each mip level.
    pub fn levels(&self) -> impl Iterator<Item = (usize, u32, u32)> + '_ {
        let mut off = 0usize;
        (0..PV_MIPS).map(move |l| {
            let (w, r) = ((PV >> l).max(1), rows(self.h, l));
            let o = off;
            off += (w * r * 4) as usize;
            (o, w, r)
        })
    }
}

/// Rows a picture `h` tall fills at mip level `l`.
fn rows(h: u32, l: u32) -> u32 {
    (h.div_ceil(1 << l)).clamp(1, (PV >> l).max(1))
}

/// Builds a preview from `w`×`h` RGBA pixels (w, h ≤ PV).
fn from_rgba(w: u32, h: u32, px: &[u8]) -> Preview {
    let mut base = vec![0u8; (PV * h * 4) as usize];
    for y in 0..h as usize {
        let src = &px[y * w as usize * 4..(y + 1) * w as usize * 4];
        base[y * PV as usize * 4..y * PV as usize * 4 + src.len()].copy_from_slice(src);
    }
    let mut out = base.clone();
    let mut prev = base;
    let (mut pw, mut pr) = (PV as usize, h as usize);
    for l in 1..PV_MIPS {
        let (nw, nr) = ((PV >> l).max(1) as usize, rows(h, l) as usize);
        let mut next = vec![0u8; nw * nr * 4];
        for y in 0..nr {
            for x in 0..nw {
                for c in 0..4 {
                    let s = |dx: usize, dy: usize| {
                        prev[(((y * 2 + dy).min(pr - 1)) * pw + (x * 2 + dx).min(pw - 1)) * 4 + c] as u32
                    };
                    next[(y * nw + x) * 4 + c] = ((s(0, 0) + s(1, 0) + s(0, 1) + s(1, 1) + 2) / 4) as u8;
                }
            }
        }
        out.extend_from_slice(&next);
        prev = next;
        (pw, pr) = (nw, nr);
    }
    Preview { w, h, mips: out }
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
        let ok = tool("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
        log::info!("ffmpeg {}", if ok { "found: videos will play" } else { "not found: no video previews" });
        ok
    })
}

/// An `ffmpeg` command that doesn't flash a console window.
pub fn tool(name: &str) -> Command {
    debug_assert_eq!(name, "ffmpeg");
    #[allow(unused_mut)] // only changed on Windows
    let mut c = Command::new(ffmpeg_path());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// The video's length in seconds, if ffmpeg can tell.
fn duration(p: &Path) -> Option<f32> {
    probe(p).duration
}

static FFMPEG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Which ffmpeg to run (found once). First one bundled next to Gibson (the "full"
/// downloads), if it runs on this machine; then, on macOS, Homebrew's and MacPorts'
/// folders (apps started from Finder don't get the shell's PATH); then the PATH.
fn ffmpeg_path() -> PathBuf {
    FFMPEG_PATH
        .get_or_init(|| {
            let exe = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
            let mut candidates: Vec<PathBuf> = Vec::new();
            if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
                candidates.push(dir.join(exe));
            }
            if cfg!(target_os = "macos") {
                for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"] {
                    candidates.push(Path::new(dir).join(exe));
                }
            }
            for p in candidates.into_iter().filter(|p| p.is_file()) {
                if runs(&p) {
                    log::info!("ffmpeg: {}", p.display());
                    return p;
                }
                log::warn!("ffmpeg at {} doesn't run here: skipped", p.display());
            }
            PathBuf::from(exe)
        })
        .clone()
}

fn runs(p: &Path) -> bool {
    let mut c = Command::new(p);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c.arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// What ffmpeg says about a media file: picture size, length, and whether it has
/// sound. (`ffmpeg -i` with no output prints this and stops.)
#[derive(Default, Debug)]
pub struct Probe {
    pub size: Option<(u32, u32)>,
    pub duration: Option<f32>,
    pub audio: bool,
}

pub fn probe(p: &Path) -> Probe {
    let Ok(out) = tool("ffmpeg").args(["-hide_banner", "-nostdin", "-i"]).arg(p).stdout(Stdio::null()).output() else {
        return Probe::default();
    };
    parse_probe(&String::from_utf8_lossy(&out.stderr))
}

fn parse_probe(text: &str) -> Probe {
    let mut pr = Probe::default();
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("Duration:") {
            let hms = rest.split(',').next().unwrap_or("").trim();
            let parts: Vec<f32> = hms.split(':').filter_map(|v| v.parse().ok()).collect();
            if parts.len() == 3 {
                let d = parts[0] * 3600.0 + parts[1] * 60.0 + parts[2];
                pr.duration = (d.is_finite() && d > 0.0).then_some(d);
            }
        } else if line.starts_with("Stream #") {
            if line.contains(": Audio:") {
                pr.audio = true;
            } else if let Some(v) = line.split(": Video:").nth(1) {
                if pr.size.is_none() {
                    // e.g. "h264 (High) (avc1 / 0x31637661), yuv420p, 1920x1080 [SAR 1:1 DAR 16:9], ..."
                    pr.size = v.split([' ', ',']).find_map(|tok| {
                        let (w, h) = tok.split_once('x')?;
                        if w.len() < 2 || !w.bytes().all(|b| b.is_ascii_digit()) || !h.bytes().all(|b| b.is_ascii_digit()) {
                            return None;
                        }
                        Some((w.parse().ok()?, h.parse().ok()?)).filter(|&(w, h): &(u32, u32)| w >= 8 && h >= 8)
                    });
                }
            }
        }
    }
    pr
}

#[cfg(test)]
mod probe_tests {
    #[test]
    fn parses_ffmpeg_info() {
        let t = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'a.mp4':\n  Duration: 00:03:21.45, start: 0.000000, bitrate: 1200 kb/s\n  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, bt709, progressive), 1920x1080 [SAR 1:1 DAR 16:9], 1000 kb/s, 25 fps\n  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 48000 Hz, stereo, fltp, 128 kb/s\nAt least one output file must be specified\n";
        let p = super::parse_probe(t);
        assert_eq!(p.size, Some((1920, 1080)));
        assert!((p.duration.unwrap() - 201.45).abs() < 0.01);
        assert!(p.audio);
        let silent = super::parse_probe("  Duration: N/A, bitrate: N/A\n    Stream #0:0: Video: gif, bgra, 480x270, 10 fps\n");
        assert_eq!(silent.size, Some((480, 270)));
        assert_eq!(silent.duration, None);
        assert!(!silent.audio);
    }
}


/// Decodes `secs` of video from `seek` (or one frame if `secs` is None), scaled to
/// PV wide and at most PV tall, as raw RGBA.
fn decode(p: &Path, seek: f32, secs: Option<f32>) -> Option<Vec<u8>> {
    let fps = if secs.is_some() { format!("fps={CLIP_FPS},") } else { String::new() };
    let filter = format!("{fps}scale={PV}:-2,crop=iw:'min(ih,{PV})':0:0");
    let mut cmd = tool("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-ss", &format!("{seek:.2}"), "-i"]).arg(p);
    match secs {
        Some(t) => cmd.args(["-t", &format!("{t:.2}")]),
        None => cmd.args(["-frames:v", "1"]),
    };
    let mut child = cmd
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

/// Montage: short bits from across the video, cut together into one loop.
const MONTAGE_AT: [f32; 4] = [0.12, 0.32, 0.52, 0.72];
const MONTAGE_BIT: f32 = 0.75; // seconds per bit
const SHORT_VIDEO: f32 = 8.0; // shorter than this: just loop from the start

/// One frame for the still, from where the montage starts; and the video's length.
pub fn video_still(p: &Path) -> Option<(Preview, f32)> {
    if !have_ffmpeg() {
        return None;
    }
    let dur = duration(p).unwrap_or(0.0);
    let first = if dur > SHORT_VIDEO { dur * MONTAGE_AT[0] } else { 0.0 };
    for seek in [first, 0.0] {
        if let Some(px) = decode(p, seek, None) {
            let h = (px.len() / (PV as usize * 4)) as u32;
            if h > 0 {
                return Some((from_rgba(PV, h, &px[..(PV * h * 4) as usize]), dur));
            }
        }
    }
    None
}

/// A video's looping preview: its frames, and where each cut (a new bit) starts.
pub struct Clip {
    pub frames: Vec<Preview>,
    pub cuts: Vec<usize>,
}

/// Builds the montage for a video `dur` seconds long whose frames are `h` tall.
pub fn video_clip(p: &Path, dur: f32, h: u32) -> Option<Clip> {
    if !have_ffmpeg() || h == 0 {
        return None;
    }
    let frame = (PV * h * 4) as usize;
    let bits: Vec<(f32, f32)> = if dur > SHORT_VIDEO {
        MONTAGE_AT.iter().map(|f| (dur * f, MONTAGE_BIT)).collect()
    } else {
        vec![(0.0, CLIP_SECONDS.min(dur.max(1.0)))]
    };
    let mut clip = Clip { frames: Vec::new(), cuts: Vec::new() };
    for (seek, secs) in bits {
        let Some(px) = decode(p, seek, Some(secs)) else { continue };
        clip.cuts.push(clip.frames.len());
        clip.frames.extend(px.chunks_exact(frame).map(|f| from_rgba(PV, h, f)));
    }
    if clip.frames.is_empty() { None } else { Some(clip) }
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
