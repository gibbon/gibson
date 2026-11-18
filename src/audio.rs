// Sound for the media viewer: FFmpeg decodes a video's audio to stereo f32 at the
// output device's rate; a small queue feeds the speakers (cpal). The samples that
// have actually been played are the clock the video follows, so picture and sound
// stay in step (and if the audio runs dry, the picture waits for it).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use std::collections::VecDeque;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const AHEAD_SECS: f32 = 0.5; // how far the decoder may run ahead of the speakers

struct Shared {
    queue: Mutex<VecDeque<f32>>, // interleaved stereo
    played: AtomicU64,           // stereo frames sent to the speakers since the last start
    paused: AtomicBool,
    gain: AtomicU32, // f32 bits: volume, 0 when muted
}

pub struct Audio {
    _stream: cpal::Stream,
    shared: Arc<Shared>,
    rate: u32,
    feeder: Option<(Arc<AtomicBool>, Child)>,
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.stop_feeder();
    }
}

/// Does this file have a sound track? (ffprobe)
pub fn has_audio(p: &Path) -> bool {
    tool("ffprobe")
        .args(["-v", "error", "-select_streams", "a:0", "-show_entries", "stream=codec_type", "-of", "csv=p=0"])
        .arg(p)
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("audio"))
}

fn tool(name: &str) -> Command {
    crate::preview::tool(name)
}

impl Audio {
    /// Opens the default output device. None if there isn't one.
    pub fn new(gain: f32) -> Option<Audio> {
        let device = cpal::default_host().default_output_device()?;
        let supported = device.default_output_config().map_err(|e| log::warn!("audio: {e}")).ok()?;
        let config = supported.config();
        let shared = Arc::new(Shared {
            queue: Mutex::new(VecDeque::new()),
            played: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            gain: AtomicU32::new(gain.to_bits()),
        });
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, shared.clone()),
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, shared.clone()),
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, shared.clone()),
            cpal::SampleFormat::I32 => build::<i32>(&device, &config, shared.clone()),
            other => {
                log::warn!("audio: unsupported sample format {other:?}");
                return None;
            }
        }?;
        stream.play().map_err(|e| log::warn!("audio: {e}")).ok()?;
        log::info!("audio: {} Hz, {} channels", config.sample_rate, config.channels);
        Some(Audio { _stream: stream, shared, rate: config.sample_rate, feeder: None })
    }

    /// Plays `path`'s sound from `at` seconds (replacing whatever was playing).
    pub fn start(&mut self, path: &Path, at: f32) {
        self.stop_feeder();
        self.shared.queue.lock().unwrap().clear();
        self.shared.played.store(0, Ordering::Relaxed);
        let child = tool("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-ss", &format!("{at:.2}"), "-i"])
            .arg(path)
            .args(["-vn", "-sn", "-ac", "2", "-ar", &self.rate.to_string(), "-f", "f32le", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else { return };
        let Some(mut out) = child.stdout.take() else { return };
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, stop2) = (self.shared.clone(), stop.clone());
        let limit = (self.rate as f32 * 2.0 * AHEAD_SECS) as usize;
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 16 * 1024];
            let mut carry: Vec<u8> = Vec::new();
            while !stop2.load(Ordering::Relaxed) {
                // Stay only a little ahead of the speakers.
                if shared.queue.lock().unwrap().len() > limit {
                    std::thread::sleep(std::time::Duration::from_millis(8));
                    continue;
                }
                let n = match out.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                carry.extend_from_slice(&buf[..n]);
                let whole = carry.len() / 4 * 4;
                let mut q = shared.queue.lock().unwrap();
                q.extend(carry[..whole].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
                drop(q);
                carry.drain(..whole);
            }
        });
        self.feeder = Some((stop, child));
    }

    fn stop_feeder(&mut self) {
        if let Some((stop, mut child)) = self.feeder.take() {
            stop.store(true, Ordering::Relaxed);
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Seconds of sound played since the last `start`.
    pub fn clock(&self) -> f32 {
        self.shared.played.load(Ordering::Relaxed) as f32 / self.rate as f32
    }

    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Relaxed);
    }

    pub fn set_gain(&self, gain: f32) {
        self.shared.gain.store(gain.to_bits(), Ordering::Relaxed);
    }
}

fn build<T>(device: &cpal::Device, config: &cpal::StreamConfig, shared: Arc<Shared>) -> Option<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let stream = device.build_output_stream::<T, _, _>(
        config.clone(),
        move |data: &mut [T], _| {
            let gain = f32::from_bits(shared.gain.load(Ordering::Relaxed));
            if shared.paused.load(Ordering::Relaxed) {
                data.iter_mut().for_each(|s| *s = T::from_sample(0.0));
                return;
            }
            let mut q = shared.queue.lock().unwrap();
            let mut frames = 0u64;
            for frame in data.chunks_mut(channels) {
                // Out of sound: silence, and the clock (so the picture) waits.
                let (l, r) = if q.len() >= 2 {
                    frames += 1;
                    (q.pop_front().unwrap_or(0.0) * gain, q.pop_front().unwrap_or(0.0) * gain)
                } else {
                    (0.0, 0.0)
                };
                for (c, s) in frame.iter_mut().enumerate() {
                    let v = match (channels, c) {
                        (1, _) => (l + r) * 0.5,
                        (_, 0) => l,
                        (_, 1) => r,
                        _ => 0.0,
                    };
                    *s = T::from_sample(v);
                }
            }
            drop(q);
            shared.played.fetch_add(frames, Ordering::Relaxed);
        },
        |e| log::warn!("audio stream: {e}"),
        None,
    );
    stream.map_err(|e| log::warn!("audio: {e}")).ok()
}
