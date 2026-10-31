#!/usr/bin/env node
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import { prepareMacosEnvironment } from "./macos-runtime.mjs";
import { prepareWindowsEnvironment } from "./windows-runtime.mjs";

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
  }
  // Keep Tauri's own argument handling, exit status and dev-process lifecycle.
  createRequire(import.meta.url)("@tauri-apps/cli/tauri.js");
} catch (error) {
  console.error(`[native] ${error.message}`);
  process.exitCode = 1;
}
