// Facts about a file or folder for its tower: what its bytes say it is, exact size,
// times, attributes, entropy, lines, pixels... as (KEY, VALUE) rows.

use std::fs::Metadata;
use std::path::Path;
use std::time::SystemTime;

/// Rows for a file. `head` is its first bytes, `lines` how many text lines were read.
pub fn file(p: &Path, md: Option<&Metadata>, head: &[u8], lines: usize, is_text: bool) -> Vec<(String, String)> {
    let mut m: Vec<(String, String)> = Vec::new();
    let mut add = |k: &str, v: String| m.push((k.to_string(), v));
    add("TYPE", signature(head, is_text));
    if let Some(md) = md {
        add("BYTES", grouped(md.len()));
        add("MODIFIED", datetime(md.modified().ok()));
        add("CREATED", datetime(md.created().ok()));
        add("ACCESSED", datetime(md.accessed().ok()));
        add("ATTRIB", attributes(md));
    }
    if !head.is_empty() {
        let e = entropy(head);
        let note = if e > 7.5 { "  COMPRESSED / ENCRYPTED" } else if e < 5.0 { "  STRUCTURED" } else { "" };
        add("ENTROPY", format!("{e:.2} BITS/BYTE{note}"));
    }
    if is_text {
        add("LINES", format!("{lines}{}", if head.len() >= 48 * 1024 { "+" } else { "" }));
        add("ENCODING", encoding(head));
    }
    if crate::preview::kind(p) == Some(crate::preview::PvKind::Image) {
        if let Ok((w, h)) = image::image_dimensions(p) {
            add("PIXELS", format!("{w} × {h}  ({:.1} MP)", w as f64 * h as f64 / 1e6));
        }
    }
    if !head.is_empty() {
        let first: Vec<String> = head.iter().take(8).map(|b| format!("{b:02X}")).collect();
        add("HEAD", first.join(" "));
    }
    m
}

/// Rows for a folder.
pub fn folder(md: Option<&Metadata>, files: usize, dirs: usize) -> Vec<(String, String)> {
    let mut m = vec![("CONTAINS".to_string(), format!("{files} FILES, {dirs} FOLDERS"))];
    if let Some(md) = md {
        m.push(("MODIFIED".into(), datetime(md.modified().ok())));
        m.push(("CREATED".into(), datetime(md.created().ok())));
        m.push(("ATTRIB".into(), attributes(md)));
    }
    m
}

/// What the first bytes say the file is (magic numbers), whatever its extension.
fn signature(h: &[u8], is_text: bool) -> String {
    let starts = |s: &[u8]| h.starts_with(s);
    let at = |o: usize, s: &[u8]| h.len() >= o + s.len() && &h[o..o + s.len()] == s;
    let t = if starts(b"\x89PNG") {
        "PNG IMAGE"
    } else if starts(b"\xFF\xD8\xFF") {
        "JPEG IMAGE"
    } else if starts(b"GIF8") {
        "GIF IMAGE"
    } else if starts(b"RIFF") && at(8, b"WEBP") {
        "WEBP IMAGE"
    } else if starts(b"RIFF") && at(8, b"WAVE") {
        "WAV AUDIO"
    } else if starts(b"RIFF") && at(8, b"AVI ") {
        "AVI VIDEO"
    } else if starts(b"BM") && h.len() > 14 {
        "BMP IMAGE"
    } else if starts(b"%PDF") {
        "PDF DOCUMENT"
    } else if starts(b"PK\x03\x04") {
        "ZIP CONTAINER"
    } else if starts(b"7z\xBC\xAF") {
        "7-ZIP ARCHIVE"
    } else if starts(b"Rar!") {
        "RAR ARCHIVE"
    } else if starts(b"\x1F\x8B") {
        "GZIP STREAM"
    } else if starts(b"MZ") {
        "WINDOWS EXECUTABLE (PE)"
    } else if starts(b"\x7FELF") {
        "ELF EXECUTABLE"
    } else if starts(b"\xCF\xFA\xED\xFE") || starts(b"\xCA\xFE\xBA\xBE") {
        "MACH-O EXECUTABLE"
    } else if starts(b"\x1A\x45\xDF\xA3") {
        "MATROSKA / WEBM VIDEO"
    } else if at(4, b"ftyp") {
        "MPEG-4 / ISO MEDIA"
    } else if starts(b"ID3") || starts(b"\xFF\xFB") || starts(b"\xFF\xF3") {
        "MP3 AUDIO"
    } else if starts(b"fLaC") {
        "FLAC AUDIO"
    } else if starts(b"OggS") {
        "OGG MEDIA"
    } else if starts(b"SQLite format 3") {
        "SQLITE DATABASE"
    } else if starts(b"\xD0\xCF\x11\xE0") {
        "OLE2 (OLD OFFICE)"
    } else if starts(b"{\\rtf") {
        "RTF DOCUMENT"
    } else if starts(b"\xEF\xBB\xBF") {
        "UTF-8 TEXT (BOM)"
    } else if h.is_empty() {
        "EMPTY"
    } else if is_text || looks_like_text(h) {
        "TEXT"
    } else {
        "BINARY DATA"
    };
    t.to_string()
}

fn looks_like_text(h: &[u8]) -> bool {
    let sample = &h[..h.len().min(1024)];
    let odd = sample.iter().filter(|&&b| b < 0x09 || (b > 0x0D && b < 0x20)).count();
    odd * 50 < sample.len()
}

fn encoding(h: &[u8]) -> String {
    if h.starts_with(b"\xEF\xBB\xBF") {
        "UTF-8 WITH BOM".into()
    } else if h.starts_with(b"\xFF\xFE") || h.starts_with(b"\xFE\xFF") {
        "UTF-16".into()
    } else if h.is_ascii() {
        "ASCII".into()
    } else if std::str::from_utf8(h).is_ok() || std::str::from_utf8(&h[..h.len().saturating_sub(4)]).is_ok() {
        "UTF-8".into()
    } else {
        "8-BIT / MIXED".into()
    }
}

/// Shannon entropy of the bytes, 0 (all the same) .. 8 (random).
fn entropy(h: &[u8]) -> f32 {
    let mut n = [0u32; 256];
    for &b in h {
        n[b as usize] += 1;
    }
    let len = h.len() as f32;
    n.iter().filter(|&&c| c > 0).map(|&c| {
        let p = c as f32 / len;
        -p * p.log2()
    }).sum()
}

/// How often each byte value occurs, in 64 bins (for the byte map), scaled 0..1.
pub fn byte_map(h: &[u8]) -> Vec<f32> {
    let mut bins = vec![0u32; 64];
    for &b in h {
        bins[b as usize / 4] += 1;
    }
    let max = bins.iter().copied().max().unwrap_or(1).max(1) as f32;
    // square root so rare values still show
    bins.iter().map(|&c| (c as f32 / max).sqrt()).collect()
}

fn grouped(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn datetime(t: Option<SystemTime>) -> String {
    let Some(t) = t else { return "-".into() };
    let secs = t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!(
        "{} {:02}:{:02}:{:02} UTC",
        crate::fs_scan::format_date(Some(t)),
        secs / 3600 % 24,
        secs / 60 % 60,
        secs % 60
    )
}

#[cfg(windows)]
fn attributes(md: &Metadata) -> String {
    use std::os::windows::fs::MetadataExt;
    let a = md.file_attributes();
    let flags = [
        (0x1, "READ-ONLY"),
        (0x2, "HIDDEN"),
        (0x4, "SYSTEM"),
        (0x20, "ARCHIVE"),
        (0x200, "SPARSE"),
        (0x400, "LINK"),
        (0x800, "COMPRESSED"),
        (0x1000, "OFFLINE"),
        (0x4000, "ENCRYPTED"),
        (0x40_0000, "CLOUD (NOT ON DISK)"),
    ];
    let on: Vec<&str> = flags.iter().filter(|(bit, _)| a & bit != 0).map(|(_, n)| *n).collect();
    if on.is_empty() { "NORMAL".into() } else { on.join(", ") }
}

#[cfg(not(windows))]
fn attributes(md: &Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    let mode = md.permissions().mode();
    let rwx = |s: u32| {
        format!("{}{}{}", if s & 4 != 0 { 'r' } else { '-' }, if s & 2 != 0 { 'w' } else { '-' }, if s & 1 != 0 { 'x' } else { '-' })
    };
    format!("{:o}  {}{}{}", mode & 0o7777, rwx(mode >> 6 & 7), rwx(mode >> 3 & 7), rwx(mode & 7)).to_uppercase()
}
