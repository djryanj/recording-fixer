#!/usr/bin/env bash
# Builds the ffmpeg that ships in the macOS app: LGPL only (no --enable-gpl, no --enable-nonfree), with
# LAME for MP3 and Apple's AudioToolbox for AAC (so "aac_at", Apple's encoder, works with no extra install).
#   bash scripts/build-ffmpeg-macos.sh arm64|x86_64|universal
# Output: src-tauri/binaries/ffmpeg-{aarch64,x86_64,universal}-apple-darwin. Takes about 10 minutes.
#
# Sources are pinned by SHA-256, taken from the publishers' https sites:
#   FFmpeg 7.1.1   https://ffmpeg.org/releases/ffmpeg-7.1.1.tar.xz  (signed; signature at the same URL + .asc)
#   LAME 3.100     https://sourceforge.net/projects/lame/files/lame/3.100/
# Those are the sources to offer if anyone asks for the corresponding source of the bundled ffmpeg.
set -euo pipefail

FFMPEG_VERSION=7.1.1
FFMPEG_SHA256=733984395e0dbbe5c046abda2dc49a5544e7e0e1e2366bba849222ae9e3a03b1
LAME_VERSION=3.100
LAME_SHA256=ddfe36cab873794038ae2c1210557ad34857a4b6bdc515785d1da9e175b1da1e
MIN_MACOS=11.0

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/src-tauri/binaries"
work="${FFMPEG_WORK:-$root/target/ffmpeg-build}"
jobs="$(sysctl -n hw.ncpu)"
mkdir -p "$out" "$work"

fetch() { # url file sha256
  [ -f "$2" ] || curl -fsSL "$1" -o "$2"
  echo "$3  $2" | shasum -a 256 -c - >/dev/null || { echo "checksum mismatch for $2" >&2; exit 1; }
}

fetch "https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.xz" "$work/ffmpeg-$FFMPEG_VERSION.tar.xz" "$FFMPEG_SHA256"
fetch "https://downloads.sourceforge.net/project/lame/lame/$LAME_VERSION/lame-$LAME_VERSION.tar.gz" "$work/lame-$LAME_VERSION.tar.gz" "$LAME_SHA256"

build_arch() { # arm64 | x86_64
  local arch="$1" triple host_arch dir prefix cflags
  case "$arch" in
    arm64) triple=aarch64-apple-darwin ;;
    x86_64) triple=x86_64-apple-darwin ;;
    *) echo "unknown arch $arch" >&2; exit 1 ;;
  esac
  dir="$work/$arch"; prefix="$dir/prefix"
  rm -rf "$dir"; mkdir -p "$dir"
  cflags="-arch $arch -mmacosx-version-min=$MIN_MACOS -O2"

  echo "== LAME ($arch)"
  tar -xzf "$work/lame-$LAME_VERSION.tar.gz" -C "$dir"
  ( cd "$dir/lame-$LAME_VERSION" &&
    ./configure --prefix="$prefix" --host="$triple" --disable-shared --enable-static --disable-frontend --disable-decoder \
      CC=clang CFLAGS="$cflags" LDFLAGS="-arch $arch" >/dev/null &&
    make -j"$jobs" >/dev/null && make install >/dev/null )

  echo "== FFmpeg ($arch)"
  tar -xJf "$work/ffmpeg-$FFMPEG_VERSION.tar.xz" -C "$dir"
  host_arch="$(uname -m)"
  local cross=()
  [ "$host_arch" = "$arch" ] || cross=(--enable-cross-compile --arch="$arch" --target-os=darwin)
  ( cd "$dir/ffmpeg-$FFMPEG_VERSION" &&
    ./configure --prefix="$prefix" "${cross[@]}" --cc="clang -arch $arch" \
      --extra-cflags="-I$prefix/include -mmacosx-version-min=$MIN_MACOS" \
      --extra-ldflags="-L$prefix/lib -arch $arch -mmacosx-version-min=$MIN_MACOS" \
      --disable-autodetect --disable-doc --disable-debug --disable-ffplay --disable-ffprobe \
      --enable-libmp3lame --enable-audiotoolbox >/dev/null &&
    make -j"$jobs" >/dev/null )
  cp "$dir/ffmpeg-$FFMPEG_VERSION/ffmpeg" "$out/ffmpeg-$triple"
  strip -x "$out/ffmpeg-$triple"

  # Never ship a build that is not LGPL.
  if strings "$out/ffmpeg-$triple" | grep -qE -- '--enable-(gpl|nonfree|version3)'; then
    echo "the build configuration enables GPL or non-free parts" >&2; exit 1
  fi
  # Native binaries can be run to check them; the cross-built one is checked by its file type.
  if [ "$host_arch" = "$arch" ]; then
    "$out/ffmpeg-$triple" -hide_banner -encoders | grep -q libmp3lame || { echo "no MP3 encoder" >&2; exit 1; }
    "$out/ffmpeg-$triple" -hide_banner -encoders | grep -q aac_at || { echo "no Apple AAC encoder" >&2; exit 1; }
  else
    file "$out/ffmpeg-$triple" | grep -q "$arch" || { echo "wrong architecture" >&2; exit 1; }
  fi
}

want="${1:-universal}"
case "$want" in
  arm64) build_arch arm64 ;;
  x86_64) build_arch x86_64 ;;
  universal)
    build_arch arm64
    build_arch x86_64
    lipo -create "$out/ffmpeg-aarch64-apple-darwin" "$out/ffmpeg-x86_64-apple-darwin" -output "$out/ffmpeg-universal-apple-darwin"
    ;;
  *) echo "usage: $0 arm64|x86_64|universal" >&2; exit 1 ;;
esac

cat > "$out/FFMPEG-BUILD.txt" <<EOF
ffmpeg for macOS, built from source for this release, LGPL only (no --enable-gpl, no --enable-nonfree).
FFmpeg $FFMPEG_VERSION  sha256 $FFMPEG_SHA256  https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.xz
LAME $LAME_VERSION  sha256 $LAME_SHA256  https://sourceforge.net/projects/lame/files/lame/$LAME_VERSION/
Configured with: --disable-autodetect --disable-doc --disable-debug --disable-ffplay --disable-ffprobe --enable-libmp3lame --enable-audiotoolbox
Build script: scripts/build-ffmpeg-macos.sh in https://github.com/djryanj/recording-fixer
EOF
echo "Done: $out"
