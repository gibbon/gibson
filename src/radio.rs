// The soundtrack from a YouTube link (`music=https://www.youtube.com/watch?v=...`):
// YouTube's own embedded player, in a small panel in the corner of the window
// (YouTube wants its player visible, 200x200 or more, so it's never hidden). On
// Windows (WebView2) and macOS (WebKit). Where it can't play here (Linux, no
// WebView2, a video that doesn't allow embedding) Gibson opens the link in the
// browser instead.
#![cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code, unused_imports))]
use std::sync::mpsc::{Receiver, channel};
use winit::window::Window;

/// Whether this build can show the player in the window.
pub const IN_WINDOW: bool = cfg!(any(windows, target_os = "macos"));

/// The panel's size in logical pixels (16:9, and YouTube's 200 px minimum).
const W: f64 = 356.0;
const H: f64 = 200.0;
const MARGIN: f64 = 18.0;

/// What the player says back.
pub enum Event {
    Playing,
    Error(String), // YouTube's error code: 101 / 150 = embedding not allowed, etc.
}

/// Is this setting a web link rather than a file?
pub fn is_link(s: &str) -> bool {
    let s = s.trim().to_ascii_lowercase();
    s.starts_with("https://") || s.starts_with("http://")
}

/// What a YouTube link points at: a video, a playlist, or both.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub video: Option<String>,
    pub list: Option<String>, // a playlist: shuffled, starting on a random track
}

/// A YouTube link's video and / or playlist, if it is one.
pub fn source(url: &str) -> Option<Source> {
    let list = playlist_id(url);
    let video = video_id(url);
    (list.is_some() || video.is_some()).then_some(Source { video, list })
}

fn safe_id(id: &str, len: std::ops::RangeInclusive<usize>) -> bool {
    len.contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The `list=` in a YouTube link (watch?v=..&list=.., or playlist?list=..).
fn playlist_id(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.trim_start_matches("www.").trim_start_matches("m.").trim_start_matches("music.");
    if !["youtube.com", "youtube-nocookie.com", "youtu.be"].contains(&host) {
        return None;
    }
    let (_, q) = path.split_once('?')?;
    let id = q.split(['&', '#']).find_map(|kv| kv.strip_prefix("list="))?;
    safe_id(id, 10..=64).then(|| id.to_string())
}

/// The video's id from the usual kinds of YouTube link, if it is one.
fn video_id(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.trim_start_matches("www.").trim_start_matches("m.").trim_start_matches("music.");
    let id = match host {
        "youtu.be" => path.split(['?', '&', '#', '/']).next()?.to_string(),
        "youtube.com" | "youtube-nocookie.com" if path.starts_with("playlist") => return None,
        "youtube.com" | "youtube-nocookie.com" => {
            if let Some(q) = path.strip_prefix("watch?").or_else(|| path.split_once("watch?").map(|(_, q)| q)) {
                q.split(['&', '#']).find_map(|kv| kv.strip_prefix("v="))?.to_string()
            } else {
                let (kind, id) = path.split_once('/')?;
                if !["shorts", "embed", "live", "v"].contains(&kind) {
                    return None;
                }
                id.split(['?', '&', '#', '/']).next()?.to_string()
            }
        }
        _ => return None,
    };
    safe_id(&id, 6..=20).then_some(id)
}

/// The page that holds the player (served to the web view by Gibson itself, so it
/// has a proper origin, which YouTube's embeds need).
/// A playlist plays shuffled from a random track, looping; a single video loops.
/// A track in a playlist that won't play (removed, not embeddable) is skipped.
fn page(src: &Source, volume: u8) -> String {
    let video = src.video.as_deref().unwrap_or("");
    let list = src.list.as_deref().unwrap_or("");
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8">
<meta name="referrer" content="strict-origin-when-cross-origin">
<style>html,body{{margin:0;height:100%;background:#000;overflow:hidden}}#p{{width:100%;height:100%;border:0}}</style>
</head><body><div id="p"></div>
<script>
var p, VIDEO = '{video}', LIST = '{list}', tries = 0, errors = 0;
function send(m) {{ try {{ window.ipc.postMessage(m); }} catch (e) {{}} }}
// A random track, once the playlist has loaded into the player.
function pick() {{
  var ids = p.getPlaylist();
  if (ids && ids.length) p.playVideoAt(Math.floor(Math.random() * ids.length));
  else if (tries++ < 40) setTimeout(pick, 250);
  else p.playVideo();
}}
function onYouTubeIframeAPIReady() {{
  var vars = {{ autoplay: 1, rel: 0, playsinline: 1, origin: location.origin }};
  if (LIST) {{ vars.listType = 'playlist'; vars.list = LIST; }}
  else {{ vars.loop = 1; vars.playlist = VIDEO; }}
  var cfg = {{
    width: '100%', height: '100%', playerVars: vars,
    events: {{
      onReady: function (e) {{
        e.target.setVolume({volume});
        if (LIST) {{ e.target.setShuffle(true); e.target.setLoop(true); pick(); }}
        else e.target.playVideo();
      }},
      onStateChange: function (e) {{
        if (e.data === 1) {{
          errors = 0;
          var d = p.getVideoData ? p.getVideoData() : null, i = p.getPlaylistIndex ? p.getPlaylistIndex() : -1;
          send('playing:' + (d ? d.video_id : '') + ' #' + i);
        }}
        if (e.data === 0 && !LIST) {{ e.target.seekTo(0); e.target.playVideo(); }}
      }},
      onError: function (e) {{
        // In a playlist, skip a track that won't play; give up after a run of them.
        if (LIST && ++errors < 6) {{ setTimeout(function () {{ p.nextVideo(); }}, 300); return; }}
        send('error:' + e.data);
      }}
    }}
  }};
  if (!LIST) cfg.videoId = VIDEO;
  p = new YT.Player('p', cfg);
}}
</script>
<script src="https://www.youtube.com/iframe_api" onerror="send('error:offline')"></script>
</body></html>"#
    )
}

/// The panel's place in a window of this size: the bottom-right corner, in
/// physical pixels (x, y, w, h). The HUD draws its bezel around the same box.
pub fn panel(win_w: u32, win_h: u32, scale: f64) -> (i32, i32, u32, u32) {
    let (w, h) = ((W * scale).round() as u32, (H * scale).round() as u32);
    let m = (MARGIN * scale).round() as i32;
    (win_w as i32 - w as i32 - m, win_h as i32 - h as i32 - m, w, h)
}

fn bounds(window: &Window) -> ((i32, i32), (u32, u32)) {
    let size = window.inner_size();
    let (x, y, w, h) = panel(size.width, size.height, window.scale_factor());
    ((x, y), (w, h))
}

#[cfg(any(windows, target_os = "macos"))]
pub struct Radio {
    view: wry::WebView,
    _context: wry::WebContext,
    events: Receiver<Event>,
    paused: bool,
}

#[cfg(any(windows, target_os = "macos"))]
impl Radio {
    pub fn new(window: &Window, src: &Source, volume: u32) -> Result<Radio, String> {
        use std::borrow::Cow;
        use wry::{WebContext, WebViewBuilder, http};

        // The web view's own files (cookies, cache) under Gibson's settings folder,
        // not next to the exe.
        let mut context = WebContext::new(crate::settings::dir().map(|d| d.join("webview")));
        let html = page(src, volume.min(100) as u8);
        let (tx, events) = channel();
        let home = if cfg!(windows) { "http://gibson.localhost/" } else { "gibson://localhost/" };
        let ((x, y), (w, h)) = bounds(window);
        let builder = WebViewBuilder::new_with_web_context(&mut context)
            .with_custom_protocol("gibson".into(), move |_, _req| {
                http::Response::builder()
                    .header("Content-Type", "text/html; charset=utf-8")
                    .body(Cow::Owned(html.clone().into_bytes()))
                    .unwrap()
            })
            .with_url(format!("{home}radio.html"))
            .with_bounds(rect(x, y, w, h))
            .with_autoplay(true)
            .with_focused(false)
            .with_background_color((0, 0, 0, 255))
            .with_ipc_handler(move |req| {
                let m = req.body();
                log::info!("youtube player: {m}");
                let _ = tx.send(match m.strip_prefix("error:") {
                    Some(code) => Event::Error(code.to_string()),
                    None => Event::Playing,
                });
            })
            // Links out of the player (the YouTube logo, "watch on YouTube") open in
            // the browser, never in the panel.
            .with_navigation_handler(move |url| {
                let ours = url.starts_with(home);
                if !ours {
                    let _ = crate::files::open_url(&url);
                }
                ours
            })
            .with_new_window_req_handler(|url, _| {
                let _ = crate::files::open_url(&url);
                wry::NewWindowResponse::Deny
            });
        let view = builder.build_as_child(window).map_err(|e| e.to_string())?;
        Ok(Radio { view, _context: context, events, paused: false })
    }

    /// Back in the corner after the window changes size.
    pub fn place(&self, window: &Window) {
        let ((x, y), (w, h)) = bounds(window);
        let _ = self.view.set_bounds(rect(x, y, w, h));
    }

    pub fn set_paused(&mut self, paused: bool) {
        if paused != self.paused {
            self.paused = paused;
            let js = if paused { "p && p.pauseVideo()" } else { "p && p.playVideo()" };
            let _ = self.view.evaluate_script(js);
        }
    }

    /// Hands the keyboard back to Gibson (after a click on the player).
    pub fn give_back_focus(&self) {
        let _ = self.view.focus_parent();
    }

    pub fn poll(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn rect(x: i32, y: i32, w: u32, h: u32) -> wry::Rect {
    use wry::dpi::{PhysicalPosition, PhysicalSize};
    wry::Rect { position: PhysicalPosition::new(x, y).into(), size: PhysicalSize::new(w, h).into() }
}

/// Elsewhere there's no in-window player: Gibson uses the browser.
#[cfg(not(any(windows, target_os = "macos")))]
pub struct Radio {
    _never: std::convert::Infallible,
}

#[cfg(not(any(windows, target_os = "macos")))]
impl Radio {
    pub fn new(_window: &Window, _src: &Source, _volume: u32) -> Result<Radio, String> {
        Err("no in-window player on this platform".into())
    }
    pub fn place(&self, _window: &Window) {}
    pub fn set_paused(&mut self, _paused: bool) {}
    pub fn give_back_focus(&self) {}
    pub fn poll(&self) -> Option<Event> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        let id = Some("abcDEF12345".to_string());
        assert_eq!(video_id("https://www.youtube.com/watch?v=abcDEF12345"), id);
        assert_eq!(video_id("https://youtube.com/watch?feature=x&v=abcDEF12345&t=10"), id);
        assert_eq!(video_id("https://youtu.be/abcDEF12345?t=3"), id);
        assert_eq!(video_id("https://music.youtube.com/watch?v=abcDEF12345"), id);
        assert_eq!(video_id("https://m.youtube.com/shorts/abcDEF12345"), id);
        assert_eq!(video_id("https://www.youtube.com/embed/abcDEF12345"), id);
        assert_eq!(video_id("https://example.com/watch?v=abcDEF12345"), None);
        assert_eq!(video_id("https://www.youtube.com/watch?v=<script>"), None);
        let s = source("https://www.youtube.com/watch?v=k07cflKCl-Y&list=PL6E1AC915A0204BC0").unwrap();
        assert_eq!(s.list.as_deref(), Some("PL6E1AC915A0204BC0"));
        assert_eq!(s.video.as_deref(), Some("k07cflKCl-Y"));
        let s = source("https://www.youtube.com/playlist?list=PL6E1AC915A0204BC0").unwrap();
        assert_eq!((s.video, s.list.as_deref()), (None, Some("PL6E1AC915A0204BC0")));
        assert_eq!(source("https://www.youtube.com/watch?v=abcDEF12345&list=x'y"), Some(Source { video: id.clone(), list: None }));
        assert!(is_link("HTTPS://x") && !is_link(r"C:\music\song.mp3"));
    }
}
