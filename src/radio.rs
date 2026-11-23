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

/// The video's id from the usual kinds of YouTube link, if it is one.
pub fn video_id(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.trim_start_matches("www.").trim_start_matches("m.").trim_start_matches("music.");
    let id = match host {
        "youtu.be" => path.split(['?', '&', '#', '/']).next()?.to_string(),
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
    let ok = (6..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    ok.then_some(id)
}

/// The page that holds the player (served to the web view by Gibson itself, so it
/// has a proper origin, which YouTube's embeds need).
fn page(id: &str, volume: u8) -> String {
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8">
<meta name="referrer" content="strict-origin-when-cross-origin">
<style>html,body{{margin:0;height:100%;background:#000;overflow:hidden}}#p{{width:100%;height:100%;border:0}}</style>
</head><body><div id="p"></div>
<script>
var p;
function send(m) {{ try {{ window.ipc.postMessage(m); }} catch (e) {{}} }}
function onYouTubeIframeAPIReady() {{
  p = new YT.Player('p', {{
    width: '100%', height: '100%', videoId: '{id}',
    playerVars: {{ autoplay: 1, loop: 1, playlist: '{id}', rel: 0, playsinline: 1, origin: location.origin }},
    events: {{
      onReady: function (e) {{ e.target.setVolume({volume}); e.target.playVideo(); }},
      onStateChange: function (e) {{
        if (e.data === 1) send('playing');
        if (e.data === 0) {{ e.target.seekTo(0); e.target.playVideo(); }}
      }},
      onError: function (e) {{ send('error:' + e.data); }}
    }}
  }});
}}
</script>
<script src="https://www.youtube.com/iframe_api" onerror="send('error:offline')"></script>
</body></html>"#
    )
}

/// The panel's place: the bottom-right corner, in physical pixels.
fn bounds(window: &Window) -> ((i32, i32), (u32, u32)) {
    let s = window.scale_factor();
    let size = window.inner_size();
    let (w, h) = ((W * s).round() as u32, (H * s).round() as u32);
    let m = (MARGIN * s).round() as i32;
    ((size.width as i32 - w as i32 - m, size.height as i32 - h as i32 - m), (w, h))
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
    pub fn new(window: &Window, id: &str, volume: u32) -> Result<Radio, String> {
        use std::borrow::Cow;
        use wry::{WebContext, WebViewBuilder, http};

        // The web view's own files (cookies, cache) under Gibson's settings folder,
        // not next to the exe.
        let mut context = WebContext::new(crate::settings::dir().map(|d| d.join("webview")));
        let html = page(id, volume.min(100) as u8);
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
    pub fn new(_window: &Window, _id: &str, _volume: u32) -> Result<Radio, String> {
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
        assert!(is_link("HTTPS://x") && !is_link(r"C:\music\song.mp3"));
    }
}
