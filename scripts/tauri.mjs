#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import { prepareMacosEnvironment } from "./macos-runtime.mjs";
import { prepareWindowsEnvironment } from "./windows-runtime.mjs";

// Route rustc through sccache when it is installed, so repeated full rebuilds
// (upstream merges, cargo clean, feature switches) serve unchanged crates from
// the compile cache instead of recompiling them. RUSTC_WRAPPER is not part of
// cargo's build fingerprint, so enabling it never invalidates src-tauri/target:
// the cache simply starts filling on the next compile.
//
// Another one-off lever for future rebuilds (each costs one full rebuild when
// first enabled, so flip it right before an upstream merge that forces one):
//   process.env.CARGO_PROFILE_DEV_DEBUG = "1"; // line tables only, much faster dev links
function compilerCacheEnvironment(env) {
  const executable = process.platform === "win32" ? "sccache.exe" : "sccache";
  const directories = [
    ...(env.PATH ?? env.Path ?? "").split(path.delimiter),
    // winget portable installs and `cargo install`, for shells whose PATH
    // predates the installation.
    path.join(env.LOCALAPPDATA ?? "", "Microsoft", "WinGet", "Links"),
    path.join(env.USERPROFILE ?? env.HOME ?? "", ".cargo", "bin"),
  ];
  for (const directory of directories) {
    if (!directory) continue;
    const candidate = path.join(directory, executable);
    if (fs.existsSync(candidate)) {
      env.RUSTC_WRAPPER = candidate;
      // The default 10G cache is too small for this workspace's artifact volume.
      env.SCCACHE_CACHE_SIZE = "50G";
      return candidate;
    }
  }
  return null;
}

const args = process.argv.slice(2);
const command = args.find((arg) => arg !== "-v" && arg !== "--verbose");
try {
  if (["darwin", "win32"].includes(process.platform) && ["dev", "build", "bundle"].includes(command)
      && !args.includes("--help") && !args.includes("-h")) {
    const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
    const env = process.platform === "darwin"
      ? prepareMacosEnvironment(root)
      : await prepareWindowsEnvironment(root);
    Object.assign(process.env, env);
    const wrapper = compilerCacheEnvironment(process.env);
    if (wrapper) console.log(`[native] Rust compile cache: ${wrapper}`);
  }
  // Keep Tauri's own argument handling, exit status and dev-process lifecycle.
  createRequire(import.meta.url)("@tauri-apps/cli/tauri.js");
} catch (error) {
  console.error(`[native] ${error.message}`);
  process.exitCode = 1;
}
