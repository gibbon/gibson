// Handing files to the operating system: open with the default app, or show the
// file in Explorer / Finder / the file manager. Programs and scripts are never
// launched from here (a stray double-click mid-flight shouldn't run anything):
// they're revealed instead.

use std::path::Path;
use std::process::Command;

const PROGRAM_EXT: &[&str] = &[
    "exe", "com", "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "msi", "msp", "scr", "cpl",
    "hta", "jar", "reg", "lnk", "url", "appref-ms", "sh", "app", "command", "run", "bin", "appimage",
];

/// True for anything that would run code when "opened".
pub fn is_program(p: &Path) -> bool {
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    PROGRAM_EXT.contains(&ext.as_str())
}

/// Opens with the default app. Returns a short message for the HUD.
pub fn open(p: &Path) -> Result<String, String> {
    if is_program(p) {
        reveal(p)?;
        return Ok("PROGRAMS AREN'T RUN FROM GIBSON — SHOWN IN EXPLORER".into());
    }
    let r = if cfg!(windows) {
        Command::new("explorer").arg(p).spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(p).spawn()
    } else {
        Command::new("xdg-open").arg(p).spawn()
    };
    r.map(|_| format!("OPENING {}", name(p))).map_err(|e| format!("CAN'T OPEN: {e}").to_uppercase())
}

/// Shows the file selected in the system file manager.
pub fn reveal(p: &Path) -> Result<String, String> {
    let r = if cfg!(windows) {
        reveal_windows(p)
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg("-R").arg(p).spawn()
    } else {
        Command::new("xdg-open").arg(p.parent().unwrap_or(p)).spawn()
    };
    r.map(|_| format!("SHOWING {} IN EXPLORER", name(p))).map_err(|e| format!("CAN'T SHOW: {e}").to_uppercase())
}

#[cfg(windows)]
fn reveal_windows(p: &Path) -> std::io::Result<std::process::Child> {
    // explorer wants exactly: /select,"C:\path with spaces\file"
    use std::os::windows::process::CommandExt;
    Command::new("explorer").raw_arg(format!("/select,\"{}\"", p.display())).spawn()
}

#[cfg(not(windows))]
fn reveal_windows(_p: &Path) -> std::io::Result<std::process::Child> {
    unreachable!()
}

fn name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_uppercase()).unwrap_or_default()
}
