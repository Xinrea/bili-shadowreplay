#!/usr/bin/env bash
set -euo pipefail

prefix=${1:?Usage: build.sh INSTALL_PREFIX}
source_dir=$(mktemp -d)
trap 'rm -rf "$source_dir"' EXIT
minimum=$(node -p "JSON.parse(require('fs').readFileSync('src-tauri/tauri.macos.conf.json', 'utf8')).bundle.macOS.minimumSystemVersion")
codec_cflags=""
codec_ldflags=""
codec_pkg_config=""
for formula in lame opus x264 x265; do
  dependency=$(brew --prefix "$formula")
  codec_cflags="$codec_cflags -I$dependency/include"
  codec_ldflags="$codec_ldflags -L$dependency/lib"
  codec_pkg_config="$dependency/lib/pkgconfig:$codec_pkg_config"
done
export PKG_CONFIG_PATH="$codec_pkg_config${PKG_CONFIG_PATH:-}"

archive="$source_dir/ffmpeg.tar.gz"
curl --fail --location --retry 3 --connect-timeout 20 --max-time 180 \
  https://codeload.github.com/FFmpeg/FFmpeg/tar.gz/refs/tags/n9.0.2 \
  --output "$archive"
echo "6e374ed621e48faa40639307dff48ba6fe574a509977956d2cce9669b7cc27e9  $archive" | shasum -a 256 --check
tar -xzf "$archive" -C "$source_dir" --strip-components=1
cd "$source_dir"

# Avoid ffmpeg-full's unrelated OCR/ML dependencies, whose current bottles can
# require a newer macOS patch release. Keep all built-in codecs and filters,
# the application's external codecs, native TLS and Vulkan/VideoToolbox.
export MACOSX_DEPLOYMENT_TARGET="$minimum"
./configure \
  --prefix="$prefix" \
  --cc=clang --cxx=clang++ \
  --disable-autodetect --disable-debug --disable-doc --disable-ffplay \
  --disable-static --enable-shared \
  --enable-gpl --enable-version3 \
  --enable-libmp3lame --enable-libopus --enable-libx264 --enable-libx265 \
  --enable-securetransport --enable-zlib \
  --enable-vulkan --enable-videotoolbox --enable-audiotoolbox \
  --extra-cflags="-mmacosx-version-min=$minimum -I$(brew --prefix vulkan-headers)/include$codec_cflags" \
  --extra-ldflags="-mmacosx-version-min=$minimum$codec_ldflags"
make -j"$(sysctl -n hw.logicalcpu)"
make install
