#!/usr/bin/env bash
# Packages the Linux build: dist/Gibson-linux-x86_64.tar.gz, or with FFMPEG=<dir>
# (holding a slim ffmpeg from packaging/ffmpeg/build.sh) the -full one, carrying
# ffmpeg next to gibson. Run from the repo root after `cargo build --release`.
set -euo pipefail

name=Gibson-linux-x86_64
[ -n "${FFMPEG:-}" ] && name=$name-full
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
dir=$stage/gibson
mkdir -p "$dir" dist

cp target/release/gibson README.md LICENSE "$dir/"
cp assets/icon/icon_256.png "$dir/gibson.png"
cp packaging/linux/gibson.desktop packaging/linux/install.sh "$dir/"
chmod +x "$dir/gibson" "$dir/install.sh"
if [ -n "${FFMPEG:-}" ]; then
    cp "$FFMPEG/ffmpeg" "$dir/"
    chmod +x "$dir/ffmpeg"
    cp packaging/ffmpeg/NOTICE.txt "$dir/FFMPEG-NOTICE.txt"
    cp "$FFMPEG/COPYING.LGPLv2.1" "$dir/FFMPEG-COPYING.LGPLv2.1.txt"
fi

tar -C "$stage" -czf "dist/$name.tar.gz" gibson
echo "made dist/$name.tar.gz"
