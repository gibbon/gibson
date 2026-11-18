// File categories for the top-bar filter.

use crate::fs_scan::Entry;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Cat {
    Folder,
    Image,
    Video,
    Audio,
    Document,
    Code,
    Archive,
    Program,
    Other,
}

impl Cat {
    pub const ALL: [Cat; 9] =
        [Cat::Folder, Cat::Image, Cat::Video, Cat::Audio, Cat::Document, Cat::Code, Cat::Archive, Cat::Program, Cat::Other];

    pub fn label(self) -> &'static str {
        match self {
            Cat::Folder => "FOLDERS",
            Cat::Image => "IMAGES",
            Cat::Video => "VIDEO",
            Cat::Audio => "AUDIO",
            Cat::Document => "DOCS",
            Cat::Code => "CODE",
            Cat::Archive => "ARCHIVES",
            Cat::Program => "PROGRAMS",
            Cat::Other => "OTHER",
        }
    }
}

pub fn of(e: &Entry) -> Cat {
    if e.is_dir {
        return Cat::Folder;
    }
    let ext = e.path.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
    let is = |list: &[&str]| list.contains(&ext.as_str());
    if is(&["png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "heic", "svg", "ico", "raw", "cr2", "nef", "psd"]) {
        Cat::Image
    } else if is(&["mp4", "mkv", "mov", "webm", "avi", "m4v", "wmv", "mpg", "mpeg", "ts", "flv"]) {
        Cat::Video
    } else if is(&["mp3", "wav", "flac", "m4a", "m4b", "aac", "ogg", "opus", "wma", "aiff", "mid", "midi"]) {
        Cat::Audio
    } else if is(&["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt", "md", "epub", "csv"]) {
        Cat::Document
    } else if is(&["exe", "msi", "dll", "com", "bat", "cmd", "ps1", "sh", "appimage", "app", "jar", "apk", "sys", "scr", "lnk"]) {
        Cat::Program
    } else if is(&["zip", "7z", "rar", "tar", "gz", "bz2", "xz", "tgz", "iso", "dmg", "cab", "zst"]) {
        Cat::Archive
    } else if is(&[
        "rs", "py", "js", "ts", "tsx", "jsx", "c", "cpp", "h", "hpp", "cs", "java", "go", "rb", "php", "html", "css",
        "json", "xml", "yml", "yaml", "toml", "ini", "cfg", "wgsl", "glsl", "hlsl", "sql", "lua", "kt", "swift", "lock",
        "log", "csproj", "sln",
    ]) {
        Cat::Code
    } else {
        Cat::Other
    }
}
