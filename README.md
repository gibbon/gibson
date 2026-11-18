<p align="center"><img src="assets/icon/icon_256.png" width="128" alt="Gibson icon"></p>

# Gibson

A 3D file browser: your folders become a city of glowing glass towers on a
circuit-board floor, and you fly through it. Inspired by the look of the
"Gibson" in *Hackers* (1995). This is an original program, not affiliated with
the film.

- **Real data on the glass**: every tower shows its folder's contents, a file's
  text, or a hex dump of its bytes; folders show a breakdown of their file types.
- **Height** is how much is inside, **colour** is the file type.
- **Pictures and videos** play on their towers as glitching holograms, and open
  in a full-screen player with sound and switchable effects (hologram, CRT,
  thermal, data mosaic, edge glow).
- **Navigate**: click a tower to fly to it, fly into a folder to enter it,
  search, filter by file type, go back and up.

Press **?** (or F1) in the app for every key.

## Download

Get the latest build from the [Releases](../../releases) page.

### Windows

1. Download `Gibson-windows.zip`, unzip it, and run `gibson.exe`.
2. The app isn't code-signed, so Windows SmartScreen may warn the first time:
   click **More info → Run anyway**.

### macOS (Apple Silicon and Intel, macOS 11+)

1. Download `Gibson-macos.zip`, unzip it, and drag **Gibson.app** to Applications.
2. The app isn't notarised by Apple ("unidentified developer"), so the first time:
   - macOS 15 and later: open it once (it will be blocked), then go to
     **System Settings → Privacy & Security** and click **Open Anyway**.
   - macOS 14 and earlier: **right-click** (or Control-click) Gibson.app → **Open** → **Open**.
   - Or, in Terminal: `xattr -dr com.apple.quarantine /Applications/Gibson.app`

### Video previews need FFmpeg

Images work out of the box. For video towers and the video player, install FFmpeg:

- Windows: `winget install Gyan.FFmpeg` (then restart Gibson)
- macOS: `brew install ffmpeg` (Homebrew)

Without it, Gibson still runs; videos just show as ordinary file towers.

## Start somewhere else

```
gibson D:\Photos              # open a folder
gibson ~/Movies/clip.mp4      # open a file's folder, with the player on it
```

(On macOS the binary is `Gibson.app/Contents/MacOS/gibson`.)

Settings and a log file live in `%APPDATA%\Gibson` (Windows) or
`~/Library/Application Support/Gibson` (macOS).

## Build from source

Needs [Rust](https://rustup.rs) (stable). On Windows, the MSVC build tools; on
macOS, the Xcode command-line tools.

```
cargo run --release
```

Pushing a tag like `v0.1.0` makes GitHub Actions build the Windows `.exe` and
the universal macOS `.app` and attach them to a release (see
`.github/workflows/build.yml` and `packaging/macos/make_app.sh`).

## License

MIT (see `LICENSE`). The fonts in `assets/fonts` are under the SIL Open Font License.

## Credits

- Built with [wgpu](https://wgpu.rs), [winit](https://github.com/rust-windowing/winit),
  [cpal](https://github.com/RustAudio/cpal), [fontdue](https://github.com/mooman219/fontdue)
  and [image](https://github.com/image-rs/image); video via the FFmpeg command-line tools.
- Fonts: [Michroma](https://fonts.google.com/specimen/Michroma) and
  [Share Tech Mono](https://fonts.google.com/specimen/Share+Tech+Mono), both under the
  SIL Open Font License (see `assets/fonts`).
