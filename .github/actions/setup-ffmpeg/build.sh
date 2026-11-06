#!/usr/bin/env bash
set -euo pipefail

prefix=${1:?Usage: build.sh INSTALL_PREFIX}
source_dir=$(mktemp -d)
trap 'rm -rf "$source_dir"' EXIT

archive="$source_dir/ffmpeg.tar.gz"
curl --fail --location --retry 3 --connect-timeout 20 --max-time 180 \
  https://codeload.github.com/FFmpeg/FFmpeg/tar.gz/refs/tags/n9.0.2 \
  --output "$archive"
echo "6e374ed621e48faa40639307dff48ba6fe574a509977956d2cce9669b7cc27e9  $archive" | sha256sum --check
tar -xzf "$archive" -C "$source_dir" --strip-components=1
cd "$source_dir"

# Keep optional libraries explicit so the SDK does not depend on which extra
# development packages happen to be installed in the Tests or Lint runner.
# This CPU test SDK includes all built-in codecs, filters and formats.
./configure \
  --prefix="$prefix" \
  --cc=clang --cxx=clang++ \
  --disable-autodetect --disable-debug --disable-doc --disable-ffplay \
  --disable-static --enable-shared \
  --enable-gpl --enable-version3 \
  --enable-libass --enable-libfontconfig --enable-libfreetype --enable-libharfbuzz \
  --enable-libmp3lame --enable-libopus --enable-libx264 --enable-libx265 \
  --enable-openssl --enable-zlib
make -j"$(nproc)"
make install
