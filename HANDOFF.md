# Gibson — handoff notes

A 3D file browser inspired by the "Gibson" in *Hackers* (1995): your real folders
as a city of translucent glass towers on a glowing circuit-board floor, which you
fly through. Started in a WSL session; continuing natively on Windows.

## Decisions so far
- **Main goal:** a usable file browser (fly into folders, open files). Maybe a
  screensaver/autopilot mode later. The 3D window compositor (`..\3d`, C#) can
  show it as a window, so no code sharing is needed.
- **Stack:** Rust + wgpu 30 + winit 0.30 (cross-platform: Windows, macOS, Linux,
  later web). Fonts bundled in `assets/fonts` (OFL): Michroma (wide headings),
  Share Tech Mono (data text).
- **Original implementation.** Two GPL-3.0 projects exist
  (github.com/paulkiernan/gibson-webgl and gibson-screensaver). The user is fine
  with GPL but does **not** want a port/clone. Don't copy their code. Their
  `docs/film-reference.md` (observations of the film's look) is used as a visual
  reference only.
- **What makes ours different:** real data on the towers (folder contents, real
  text-file lines, hex dumps of real file bytes, later image thumbnails), height =
  how much is inside, colour = file type, live updates later, navigation.

## Visual target (from film observations)
Square glass prisms ~3–5:1, deep blue body ~35% opacity, text visible through
the glass from behind (double-sided), cyan HDR text that blooms, mosaic of data
blocks (hex dumps, numeric columns, bar glyphs, framed / inverse-video blocks),
big right-aligned uppercase folder names with ▸ markers, tower tops teal from
above, black floor with violet Manhattan PCB traces + pads, white-cyan pulses
along lanes, blue haze → black with distance, strong bloom, slight grain,
banking flights down lanes.

## Milestones (stop after each one for the user to test and confirm)
1. **The city** — one real folder as glass towers with real names/data on the
   faces, circuit floor, bloom, free fly (WASD/QE/Space, Shift boost, drag to
   look, wheel = speed). ✅ **done, confirmed by user 2026-11-18** (camera has
   eased look, ~1.5 s glide, banking into turns; tuning consts top of camera.rs)
2. **Navigation** ✅ **done, confirmed 2026-11-18** — click a tower to fly to it, fly into a folder to enter it,
   Backspace to go up, drives list at the top, HUD with path/hover/sizes.
3. **Files** ← **in progress** — open file / reveal in Explorer, previews on
   faces (text, hex, image thumbnails), type-to-search lights up matching
   towers. User chose: videos actually *play* (a few seconds looping, via the
   installed FFmpeg CLI) with glitch shader effects (tearing, RGB split,
   flicker) on image/video previews. Programs/scripts are never run from
   Gibson: revealed in Explorer instead.
   Done so far (awaiting user test): open/reveal (R), search (/ or Ctrl+F),
   image previews and looping video clips (nearest 4 within 170 units, 2 s at
   12 fps, decoded by `ffmpeg` on PATH) with the glitch shader. Screenshots:
   use PrintWindow on the Gibson window only (never full-screen captures —
   the user's other windows are private), and don't send synthetic input
   while the user is at the machine. To test UI without touching the user's
   desktop: post keys/clicks to the Gibson HWND only (PostMessage WM_KEYDOWN /
   WM_MOUSEMOVE+WM_LBUTTONDOWN), and `gibson <file>` opens the viewer on it.
   Also done: ? / F1 help panel; file-type filter strip; search FILTER (Tab)
   hides non-matches live; media viewer with sound (cpal + an ffmpeg audio
   stream; the played-sample count is the clock the video follows; M mute,
   +/- volume; tower previews stay silent): images full size, videos streamed via ffmpeg with
   timeline scrubbing, 6 effects (clean, hologram, CRT, thermal, data
   mosaic, edge glow), [ ] steps through the folder's media.
4. **Alive** — live updates (notify crate), autopilot flythrough/screensaver,
   colour themes (e.g. the film's orange/magenta "under attack" palette).
5. **Cross-platform** — verify macOS/Linux, then a web build.

## Code layout (`src/`)
- `main.rs` — winit app, input, background scan + parallel face baking
  (rayon), uploads a few textures per frame, title-bar HUD.
- `fs_scan.rs` — reads one folder level + peeks inside subfolders; file heads
  for hex dumps and text lines. Caps at 240 entries (= texture-array layers).
- `city.rs` — grid layout (cell 26 units, streets between), tower sizes/kinds,
  ray picking, small deterministic RNG.
- `textgen.rs` — bakes each tower's 512×1024 R8 face texture (fontdue) with a
  full CPU mip chain.
- `camera.rs` — fly camera with inertia.
- `render.rs` — wgpu: HDR target, floor pass, towers drawn back-to-front
  (back faces then front faces, premultiplied alpha), bloom chain, composite.
- `shaders/floor.wgsl`, `tower.wgsl`, `post.wgsl`.

## Build
Needs the MSVC C++ build tools + Windows SDK (VS 2022 Build Tools, "Desktop
development with C++"), which were being installed. Then:

    cargo run --release            # starts in your user folder
    cargo run --release -- D:\some\folder

A `stable-x86_64-pc-windows-gnu` toolchain was installed while trying to work
around the missing tools. It's not needed; remove it with
`rustup toolchain uninstall stable-x86_64-pc-windows-gnu`.

Builds cleanly on Windows (MSVC, RTX 4070 / Vulkan, 120 fps). Screenshot the
window via PowerShell CopyFromScreen to check the look.
Milestone 1 committed (968ee10). Git identity is set globally.

## Working style
- Stop after each milestone and wait for the user to confirm it works.
- Keep files small; log what happens; never leave the machine in a broken
  state. Don't block the UI thread on file I/O.
- Commit after each working step (end commit messages with the user's usual
  Co-Authored-By line).

## Releases
- Public repo github.com/gibbon/gibson (MIT). GitHub Actions
  (.github/workflows/build.yml) builds Gibson-windows.zip (windowed exe with
  icon), Gibson-macos.zip (universal, ad-hoc signed .app) and
  Gibson-linux-x86_64.tar.gz (built on Ubuntu 22.04: glibc 2.35+) on every
  push; a `v*` tag publishes a Release. v0.1.0 released 2026-11-19; v0.2.0
  2026-11-21; v0.2.2 2026-11-23 (Linux, YouTube soundtrack player).
- Music (N): `music=` is a file or a YouTube link; empty plays DEFAULT_MUSIC, five tracks of the Hackers soundtrack playlist (DEFAULT_TRACKS), a random one each time
  (src/main.rs). Links play in YouTube's own embedded player in a child web view
  (wry; src/radio.rs) in the bottom-right corner, with a HUD bezel around it.
  YouTube's terms: the player stays visible (>= 200x200), unmodified, never
  rendered into the 3D scene. Linux, missing WebView2, embed errors or 20 s of
  silence fall back to opening the link in the browser.
- Since v0.2.0 each platform also has a `-full` zip carrying a slim FFmpeg 9.1
  (packaging/ffmpeg/build.sh: LGPL, static, decode-only, no external libs;
  ~21 MB exe on Windows, universal on macOS), built in CI and cached by the
  script's hash; the release attaches ffmpeg-9.1.tar.xz for the LGPL. Gibson
  runs an ffmpeg next to its own exe first, then Homebrew's, then PATH; it no
  longer uses ffprobe. Known gap: no AV1 software decoder (would need dav1d).
- macOS build compiles in CI but has not yet been run on a real Mac: the
  user will try it and report (Retina, trackpad, Cmd keys, sound, ffmpeg
  found via /opt/homebrew/bin).
- Also since milestone 3: colour themes (C: neon, glass, attack, matrix),
  autofly (P), tower preview effects (T), settings + log in the app's
  settings folder.

