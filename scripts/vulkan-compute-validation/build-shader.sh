#!/bin/sh
# Developer-only compiler; the application embeds the validated SPIR-V.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
shader="$root/src-tauri/crates/ffmpeg_utils/src/vulkan_video/shaders/composite"
: "${GLSLANG:=glslang}"
: "${SPIRV_VAL:=spirv-val}"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
"$GLSLANG" -V --target-env vulkan1.2 -o "$tmp/composite.spv" "$shader.comp"
"$SPIRV_VAL" --target-env vulkan1.2 "$tmp/composite.spv"
case "${1:---check}" in
  --check) cmp "$tmp/composite.spv" "$shader.spv" ;;
  --write) cp "$tmp/composite.spv" "$shader.spv" ;;
  *) echo 'usage: build-shader.sh [--check|--write]' >&2; exit 2 ;;
esac
