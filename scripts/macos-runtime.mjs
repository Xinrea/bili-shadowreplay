import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { assertMacosDeployment } from "./macos-deployment.mjs";

export const MOLTENVK = {
  version: "1.4.2",
  url: "https://github.com/KhronosGroup/MoltenVK/releases/download/v1.4.2/MoltenVK-macos.tar",
  sha256: "f95765a6229cb7b915990a2890ce12ebe36a730b021545d3d52ae69ce4c4024e",
  library: "MoltenVK/MoltenVK/dynamic/dylib/macOS/libMoltenVK.dylib",
  license: "MoltenVK/LICENSE",
};

const run = (program, args, env = process.env) => execFileSync(program, args, {
  env, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"],
}).trim();
const digest = (file) => createHash("sha256").update(fs.readFileSync(file)).digest("hex");

export function verifyArchive(file, expected = MOLTENVK.sha256) {
  if (digest(file) !== expected) throw new Error(`MoltenVK archive SHA-256 mismatch: ${file}`);
}

// Respect explicit SDKs; otherwise prefer the complete Homebrew SDK over a
// possibly older/minimal ffmpeg installation elsewhere on PATH.
export function ffmpegEnvironment(env = process.env) {
  const complete = (prefix) => prefix
    && fs.existsSync(path.join(prefix, "include/libavutil/avutil.h"))
    && fs.existsSync(path.join(prefix, "lib/libavutil.dylib"));
  const query = (program, args) => {
    try { return run(program, args, env); } catch { return undefined; }
  };
  let prefix = env.FFMPEG_DIR;
  if (prefix && !complete(prefix)) throw new Error(`FFMPEG_DIR does not contain a macOS FFmpeg SDK: ${prefix}`);
  if (!prefix && env.PKG_CONFIG_PATH) prefix = query("pkg-config", ["--variable=prefix", "libavutil"]);
  if (!complete(prefix)) prefix = [
    query("brew", ["--prefix", "ffmpeg-full"]),
    "/opt/homebrew/opt/ffmpeg-full", "/usr/local/opt/ffmpeg-full",
    query("pkg-config", ["--variable=prefix", "libavutil"]),
  ].find(complete);
  if (!prefix) throw new Error("FFmpeg development libraries are missing. Install the build prerequisites: brew install ffmpeg-full pkg-config");
  prefix = path.resolve(prefix);
  return {
    ...env,
    FFMPEG_DIR: prefix,
    PKG_CONFIG_PATH: [path.join(prefix, "lib/pkgconfig"), env.PKG_CONFIG_PATH].filter(Boolean).join(path.delimiter),
  };
}

function validateLibrary(file) {
  const architectures = run("lipo", ["-archs", file]).split(/\s+/);
  const required = process.arch === "arm64" ? "arm64" : "x86_64";
  if (!architectures.includes(required)) throw new Error(`MoltenVK is missing the ${required} architecture: ${file}`);
  assertMacosDeployment(run("otool", ["-l", file]), "15.0", file);
  if (!run("nm", ["-gU", file]).includes("_vkExportMetalObjectsEXT")) {
    throw new Error(`MoltenVK must export VK_EXT_metal_objects: ${file}`);
  }
}

export function prepareMoltenVk(cache, { source, archive = MOLTENVK, download } = {}) {
  // Both the source checksum and preparation revision identify the cache.
  // A changed custom library must not reuse an old prepared runtime.
  const identity = source ? digest(source) : archive.sha256;
  const directory = path.join(cache, `moltenvk-${identity}-v1`);
  const library = path.join(directory, "libvulkan.dylib");
  const receipt = path.join(directory, "runtime.json");
  if (fs.existsSync(directory)) {
    try {
      if (JSON.parse(fs.readFileSync(receipt, "utf8")).sha256 !== digest(library)) throw new Error("checksum mismatch");
      validateLibrary(library);
      return library;
    } catch (error) {
      throw new Error(`Invalid MoltenVK cache; remove ${directory} and retry: ${error.message}`);
    }
  }
  fs.mkdirSync(cache, { recursive: true });
  const work = fs.mkdtempSync(path.join(cache, ".prepare-"));
  try {
    const prepared = path.join(work, "runtime");
    fs.mkdirSync(prepared);
    if (!source) {
      const tar = path.join(work, "MoltenVK.tar");
      console.log(`[native] Preparing MoltenVK ${MOLTENVK.version} (first run downloads the macOS runtime)`);
      if (download) download(archive.url, tar);
      else run("curl", ["--fail", "--location", "--retry", "2", "--connect-timeout", "15", "--max-time", "180", "--output", tar, archive.url]);
      verifyArchive(tar, archive.sha256);
      // Extract only the two known files, after checking the pinned archive.
      run("tar", ["-xf", tar, "-C", work, archive.library, archive.license]);
      source = path.join(work, archive.library);
      fs.copyFileSync(path.join(work, archive.license), path.join(prepared, "LICENSE"));
    }
    validateLibrary(source);
    const copy = path.join(prepared, "libvulkan.dylib");
    fs.copyFileSync(source, copy);
    fs.chmodSync(copy, 0o755);
    // FFmpeg dlopen()s this bare filename. An @rpath install name lets dyld
    // reuse the absolute-path preload; a symlink alone is insufficient.
    run("install_name_tool", ["-id", "@rpath/libvulkan.dylib", copy]);
    run("codesign", ["--force", "--sign", "-", copy]);
    fs.writeFileSync(path.join(prepared, "runtime.json"), JSON.stringify({ sha256: digest(copy) }));
    // Publish complete files only. Concurrent launchers may have won the race.
    try { fs.renameSync(prepared, directory); }
    catch (error) {
      if (!["EEXIST", "ENOTEMPTY"].includes(error.code)) throw error;
      const existing = JSON.parse(fs.readFileSync(receipt, "utf8"));
      if (existing.sha256 !== digest(library)) throw new Error(`Invalid MoltenVK cache; remove ${directory} and retry`);
    }
    return library;
  } finally {
    fs.rmSync(work, { recursive: true, force: true });
  }
}

export function prepareMacosEnvironment(root, env = process.env) {
  const prepared = ffmpegEnvironment(env);
  // Explicit loader overrides remain useful for validation/API capture.
  // Normal development and bundling use the same prepared MoltenVK image.
  if (!env.BSR_VULKAN_LIBRARY || !env.BSR_MOLTENVK_LIBRARY) {
    const runtime = prepareMoltenVk(path.join(root, "src-tauri/target/macos-runtime"), {
      source: env.BSR_MOLTENVK_LIBRARY,
    });
    prepared.BSR_VULKAN_LIBRARY = env.BSR_VULKAN_LIBRARY || runtime;
    prepared.BSR_MOLTENVK_LIBRARY = env.BSR_MOLTENVK_LIBRARY || runtime;
  }
  console.log(`[native] FFmpeg SDK: ${prepared.FFMPEG_DIR}`);
  console.log(`[native] Vulkan runtime: ${prepared.BSR_VULKAN_LIBRARY}`);
  return prepared;
}
