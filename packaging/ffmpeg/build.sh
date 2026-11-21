#!/usr/bin/env bash
# Builds the slim ffmpeg that the "full" downloads carry: decode-only (every
# built-in decoder and demuxer, but no encoders except raw output, no network,
# no external libraries), LGPL v2.1+, statically linked. Gibson only ever turns
# media into raw pixels and samples, so this is all it needs.
#
#   packaging/ffmpeg/build.sh <target> <outdir>
#   target: windows-x86_64 (on Linux, with mingw-w64 + nasm)
#           macos-arm64 | macos-x86_64 (on a Mac, with nasm)
#
# Writes <outdir>/ffmpeg[.exe], the licence text, and the exact source tarball
# (published alongside the release, as the LGPL asks).
set -euo pipefail

VERSION=9.1
SHA256=1a4d394ae63d190a526cd052eb4415c156b5c5e2df37efab41a7de0b8fde2433

target=$1
out=$(mkdir -p "$2" && cd "$2" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

tarball="ffmpeg-$VERSION.tar.xz"
curl -sSfL -o "$work/$tarball" "https://ffmpeg.org/releases/$tarball"
(cd "$work" && echo "$SHA256  $tarball" | shasum -a 256 -c -)
tar -xJf "$work/$tarball" -C "$work"
cd "$work/ffmpeg-$VERSION"

common=(
    --disable-debug --disable-doc --disable-ffplay --disable-ffprobe
    --enable-static --disable-shared
    --disable-autodetect --disable-network --disable-devices --disable-hwaccels
    --disable-encoders --enable-encoder=rawvideo,pcm_f32le
    --disable-muxers --enable-muxer=rawvideo,pcm_f32le
    --disable-protocols --enable-protocol=file,pipe
    --disable-filters --enable-filter=fps,scale,crop,format,aformat,aresample,null,anull,setpts,trim
)

case "$target" in
windows-x86_64)
    exe=ffmpeg.exe
    ./configure "${common[@]}" \
        --arch=x86_64 --target-os=mingw32 --cross-prefix=x86_64-w64-mingw32- \
        --enable-w32threads --extra-ldflags=-static
    jobs=$(nproc)
    ;;
macos-arm64 | macos-x86_64)
    exe=ffmpeg
    arch=${target#macos-}
    min="-mmacosx-version-min=11.0"
    cross=()
    if [ "$arch" != "$(uname -m)" ]; then
        cross=(--enable-cross-compile --target-os=darwin)
    fi
    ./configure "${common[@]}" "${cross[@]}" \
        --arch="$arch" --cc="clang -arch $arch" \
        --extra-cflags="$min" --extra-ldflags="$min" \
        --enable-pthreads --enable-zlib
    jobs=$(sysctl -n hw.ncpu)
    ;;
*)
    echo "unknown target: $target" >&2
    exit 1
    ;;
esac

make -j"$jobs" ffmpeg >/dev/null
cp "$exe" "$out/$exe"
cp COPYING.LGPLv2.1 "$out/"
cp "$work/$tarball" "$out/"
echo "built $out/$exe ($(du -h "$out/$exe" | cut -f1))"
