#!/usr/bin/env bash
# Optional: puts Gibson in your app menu, for you only (no sudo).
#   ./install.sh            install to ~/.local
#   ./install.sh --remove   take it out again
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
app=~/.local/share/gibson
bin=~/.local/bin
share=~/.local/share

if [ "${1:-}" = --remove ]; then
    rm -rf "$app" "$bin/gibson" "$share/applications/gibson.desktop" "$share/icons/hicolor/256x256/apps/gibson.png"
    echo "Gibson removed."
    exit 0
fi

# The whole folder goes in (so a bundled ffmpeg stays next to gibson), with a
# link on the PATH.
mkdir -p "$app" "$bin" "$share/applications" "$share/icons/hicolor/256x256/apps"
cp -r "$here"/. "$app/"
ln -sf "$app/gibson" "$bin/gibson"
sed "s|^Exec=gibson|Exec=$app/gibson|" "$here/gibson.desktop" > "$share/applications/gibson.desktop"
cp "$here/gibson.png" "$share/icons/hicolor/256x256/apps/gibson.png"
command -v update-desktop-database >/dev/null && update-desktop-database "$share/applications" || true
echo "Gibson installed: find it in your app menu, or run 'gibson' (needs ~/.local/bin on your PATH)."
