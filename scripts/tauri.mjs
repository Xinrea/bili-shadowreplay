#!/usr/bin/env node
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import { prepareMacosEnvironment } from "./macos-runtime.mjs";

const args = process.argv.slice(2);
const command = args.find((arg) => arg !== "-v" && arg !== "--verbose");
try {
  if (process.platform === "darwin" && ["dev", "build", "bundle"].includes(command)
      && !args.includes("--help") && !args.includes("-h")) {
    const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
    Object.assign(process.env, prepareMacosEnvironment(root));
  }
  // Keep Tauri's own argument handling, exit status and dev-process lifecycle.
  createRequire(import.meta.url)("@tauri-apps/cli/tauri.js");
} catch (error) {
  console.error(`[native] ${error.message}`);
  process.exitCode = 1;
}
