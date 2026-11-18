// Reads one folder level, plus a peek inside each subfolder, for the city builder.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const MAX_ENTRIES: usize = 240; // towers per city (also the texture-array size)
const MAX_CHILD_PEEK: usize = 4000; // entries counted inside each subfolder
const MAX_NAMES: usize = 2000; // child names kept for a folder tower's pages
const MAX_TEXT_LINES: usize = 1500; // text-file lines kept
const TEXT_BYTES: usize = 48 * 1024; // bytes read from text files
const HEAD_BYTES: usize = 8 * 1024; // bytes read from other files for hex dumps

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,          // file bytes; folder = bytes of its immediate files
    pub child_count: usize, // folders: immediate entries
    pub capped: bool,       // more children than we counted
    pub modified: Option<SystemTime>,
    pub lines: Vec<(String, bool)>, // (text, is_folder) — child names, or text-file lines
    pub head: Vec<u8>,              // files: first bytes
    pub types: Vec<TypeStat>,       // folders: file types inside, biggest first
}

/// Files of one type inside a folder (`label` is the upper-case extension, or OTHER).
#[derive(Clone, Debug)]
pub struct TypeStat {
    pub label: String,
    pub count: usize,
    pub bytes: u64,
}

const MAX_TYPES: usize = 6; // shown separately; the rest are folded into OTHER

const TEXT_EXT: &[&str] = &[
    "txt", "md", "cs", "cpp", "c", "h", "hpp", "js", "ts", "tsx", "jsx", "py", "rs", "go", "java", "json",
    "xml", "yml", "yaml", "toml", "ini", "cfg", "log", "csv", "html", "css", "hlsl", "wgsl", "glsl", "sh",
    "ps1", "bat", "cmd", "sql", "csproj", "sln", "lock",
];

pub fn scan(dir: &Path) -> Vec<Entry> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            log::warn!("scan {}: {e}", dir.display());
            return Vec::new();
        }
    };
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for item in rd.flatten().take(5000) {
        let Ok(ft) = item.file_type() else { continue };
        if ft.is_dir() {
            dirs.push(item.path());
        } else if ft.is_file() {
            let len = item.metadata().map(|m| m.len()).unwrap_or(0);
            files.push((item.path(), len));
        }
    }
    // Folders first (they're what you navigate by), then the biggest files.
    dirs.sort_by_key(|p| name_of(p).to_lowercase());
    files.sort_by(|a, b| b.1.cmp(&a.1));

    let mut out = Vec::new();
    for p in dirs.into_iter().take(MAX_ENTRIES) {
        out.push(read_dir_entry(&p));
    }
    for (p, len) in files.into_iter().take(MAX_ENTRIES.saturating_sub(out.len())) {
        out.push(read_file_entry(&p, len));
    }
    log::info!("scanned {}: {} entries", dir.display(), out.len());
    out
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

fn read_dir_entry(p: &Path) -> Entry {
    let mut e = Entry {
        name: name_of(p),
        path: p.to_path_buf(),
        is_dir: true,
        size: 0,
        child_count: 0,
        capped: false,
        modified: fs::metadata(p).and_then(|m| m.modified()).ok(),
        lines: Vec::new(),
        head: Vec::new(),
        types: Vec::new(),
    };
    let mut by_ext: std::collections::HashMap<String, (usize, u64)> = Default::default();
    if let Ok(rd) = fs::read_dir(p) {
        let mut sub_dirs = Vec::new();
        let mut sub_files = Vec::new();
        for item in rd.flatten() {
            if e.child_count >= MAX_CHILD_PEEK {
                e.capped = true;
                break;
            }
            e.child_count += 1;
            let is_dir = item.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if !is_dir {
                let len = item.metadata().map(|m| m.len()).unwrap_or(0);
                e.size += len;
                let ext = Path::new(&item.file_name())
                    .extension()
                    .map(|x| x.to_string_lossy().to_uppercase())
                    .filter(|x| x.len() <= 6)
                    .unwrap_or_else(|| "—".into());
                let s = by_ext.entry(ext).or_default();
                s.0 += 1;
                s.1 += len;
            }
            if sub_dirs.len() + sub_files.len() < MAX_NAMES {
                let n = item.file_name().to_string_lossy().into_owned();
                if is_dir { sub_dirs.push(n) } else { sub_files.push(n) }
            }
        }
        sub_dirs.sort_by_key(|n| n.to_lowercase());
        sub_files.sort_by_key(|n| n.to_lowercase());
        e.lines.extend(sub_dirs.into_iter().map(|n| (n, true)));
        e.lines.extend(sub_files.into_iter().map(|n| (n, false)));
    }
    e.types = type_stats(by_ext);
    e
}

/// Biggest types first (by bytes, then count); the tail folded into OTHER.
fn type_stats(by_ext: std::collections::HashMap<String, (usize, u64)>) -> Vec<TypeStat> {
    let mut v: Vec<TypeStat> =
        by_ext.into_iter().map(|(label, (count, bytes))| TypeStat { label, count, bytes }).collect();
    v.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(b.count.cmp(&a.count)).then(a.label.cmp(&b.label)));
    if v.len() > MAX_TYPES {
        let rest = v.split_off(MAX_TYPES - 1);
        v.push(TypeStat {
            label: "OTHER".into(),
            count: rest.iter().map(|t| t.count).sum(),
            bytes: rest.iter().map(|t| t.bytes).sum(),
        });
    }
    v
}

fn read_file_entry(p: &Path, len: u64) -> Entry {
    let mut e = Entry {
        name: name_of(p),
        path: p.to_path_buf(),
        is_dir: false,
        size: len,
        child_count: 0,
        capped: false,
        modified: fs::metadata(p).and_then(|m| m.modified()).ok(),
        lines: Vec::new(),
        head: Vec::new(),
        types: Vec::new(),
    };
    let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
    let is_text = TEXT_EXT.contains(&ext.as_str());
    if let Ok(f) = fs::File::open(p) {
        let want = if is_text { TEXT_BYTES } else { HEAD_BYTES }.min(len as usize);
        let mut buf = Vec::with_capacity(want);
        if f.take(want as u64).read_to_end(&mut buf).is_ok() {
            e.head = buf;
        }
    }
    if is_text && !e.head.is_empty() {
        let text = String::from_utf8_lossy(&e.head);
        for raw in text.lines().take(MAX_TEXT_LINES) {
            let line: String = raw.replace('\t', "  ").chars().filter(|c| !c.is_control()).collect();
            let line = line.trim_end();
            if !line.is_empty() {
                e.lines.push((line.to_string(), false));
            }
        }
    }
    e
}

pub fn format_size(b: u64) -> String {
    let f = b as f64;
    match b {
        0..1024 => format!("{b} B"),
        1024..1_048_576 => format!("{:.1} KB", f / 1024.0),
        1_048_576..1_073_741_824 => format!("{:.1} MB", f / 1_048_576.0),
        _ => format!("{:.2} GB", f / 1_073_741_824.0),
    }
}

pub fn format_date(t: Option<SystemTime>) -> String {
    let Some(t) = t else { return "----------".into() };
    let secs = t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    // Civil-from-days (Howard Hinnant), UTC.
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}
