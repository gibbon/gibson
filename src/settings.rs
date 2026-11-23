// Remembered choices, as `key=value` lines in a small settings file:
// %APPDATA%\Gibson\ on Windows, ~/Library/Application Support/Gibson/ on macOS,
// ~/.config/gibson/ elsewhere.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub tower_fx: usize,  // effect on tower previews
    pub viewer_fx: usize, // effect in the media viewer
    pub volume: u32,      // viewer sound, percent
    pub theme: usize,     // colour scheme
    pub music: Option<PathBuf>, // soundtrack (N): any audio file of yours
    pub window: Option<(u32, u32)>, // window size in pixels (e.g. 1920x1080 for recording)
    pub look: [u32; LOOK_N], // the LOOK panel (L): see LOOK, 0..100 each
}

/// The LOOK sliders: settings-file key, label, default.
pub const LOOK_N: usize = 9;
pub const LOOK: [(&str, &str, u32); LOOK_N] = [
    ("smoke", "SMOKE", 50),
    ("smoke_colour", "SMOKE COLOUR", 0), // 0 = grey, then round the colour wheel
    ("smoke_opacity", "SMOKE OPACITY", 80),
    ("vignette", "VIGNETTE", 70),
    ("darkness", "DARKNESS", 60),
    ("grain", "FILM GRAIN", 50),
    ("crt", "CRT", 35),
    ("glitch", "GLITCH", 25),
    ("softness", "SOFTNESS", 40),
];

pub fn look_defaults() -> [u32; LOOK_N] {
    LOOK.map(|l| l.2)
}

impl Default for Settings {
    fn default() -> Settings {
        // hologram, neon
        Settings { tower_fx: 1, viewer_fx: 1, volume: 80, theme: 0, music: None, window: None, look: look_defaults() }
    }
}

/// Where Gibson keeps its settings and log.
pub fn dir() -> Option<PathBuf> {
    Some(if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?).join("Gibson")
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Gibson")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?
            .join("gibson")
    })
}

fn file() -> Option<PathBuf> {
    Some(dir()?.join("settings.txt"))
}

pub fn load() -> Settings {
    let mut s = Settings::default();
    let Some(text) = file().and_then(|f| std::fs::read_to_string(f).ok()) else { return s };
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        if k.trim() == "window" {
            s.window = parse_size(v);
            continue;
        }
        if k.trim() == "music" {
            let v = v.trim();
            s.music = (!v.is_empty()).then(|| PathBuf::from(v));
            continue;
        }
        let Ok(n) = v.trim().parse::<usize>() else { continue };
        match k.trim() {
            "tower_fx" => s.tower_fx = n % crate::viewer::EFFECTS.len(),
            "viewer_fx" => s.viewer_fx = n % crate::viewer::EFFECTS.len(),
            "volume" => s.volume = (n as u32).min(100),
            "theme" => s.theme = n % crate::theme::THEMES.len(),
            key => {
                if let Some(k) = LOOK.iter().position(|l| l.0 == key) {
                    s.look[k] = (n as u32).min(100);
                }
            }
        }
    }
    s
}

/// "1920x1080" (or 1920×1080) -> (1920, 1080), within sensible limits.
pub fn parse_size(v: &str) -> Option<(u32, u32)> {
    let (w, h) = v.trim().split_once(['x', 'X', '×'])?;
    let (w, h) = (w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?);
    ((320..=7680).contains(&w) && (240..=4320).contains(&h)).then_some((w, h))
}

pub fn save(s: &Settings) {
    let Some(f) = file() else { return };
    let text = format!(
        "# Gibson settings\ntower_fx={}\nviewer_fx={}\nvolume={}\ntheme={}\n\
         # music: an audio file of yours, or a YouTube link, to play with N (and when hidden things surface); empty: the default track\nmusic={}\n\
         # window: size in pixels, e.g. 1920x1080 (empty: fit the screen)\nwindow={}\n",
        s.tower_fx,
        s.viewer_fx,
        s.volume,
        s.theme,
        s.music.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
        s.window.map(|(w, h)| format!("{w}x{h}")).unwrap_or_default()
    );
    let mut text = text;
    text += "# look (L in the app): 0..100
";
    for (k, (key, _, _)) in LOOK.iter().enumerate() {
        text += &format!("{key}={}
", s.look[k]);
    }
    let r = f.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&f, text));
    if let Err(e) = r {
        log::warn!("can't save settings to {}: {e}", f.display());
    }
}
