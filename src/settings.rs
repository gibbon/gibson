// Remembered choices, as `key=value` lines in a small settings file:
// %APPDATA%\Gibson\ on Windows, ~/Library/Application Support/Gibson/ on macOS,
// ~/.config/gibson/ elsewhere.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub tower_fx: usize,  // effect on tower previews
    pub viewer_fx: usize, // effect in the media viewer
    pub volume: u32,      // viewer sound, percent
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { tower_fx: 1, viewer_fx: 1, volume: 80 } // hologram
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
        let Ok(n) = v.trim().parse::<usize>() else { continue };
        match k.trim() {
            "tower_fx" => s.tower_fx = n % crate::viewer::EFFECTS.len(),
            "viewer_fx" => s.viewer_fx = n % crate::viewer::EFFECTS.len(),
            "volume" => s.volume = (n as u32).min(100),
            _ => {}
        }
    }
    s
}

pub fn save(s: &Settings) {
    let Some(f) = file() else { return };
    let text = format!("# Gibson settings\ntower_fx={}\nviewer_fx={}\nvolume={}\n", s.tower_fx, s.viewer_fx, s.volume);
    let r = f.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&f, text));
    if let Err(e) = r {
        log::warn!("can't save settings to {}: {e}", f.display());
    }
}
