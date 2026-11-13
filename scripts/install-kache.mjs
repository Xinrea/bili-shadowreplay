#!/usr/bin/env node
import { spawnSync } from "node:child_process";

// Bootstrap without the project's wrapper: kache is not installed yet.
// Keep this version aligned with setup-kache and the container images.
const result = spawnSync("cargo", ["install", "kache", "--locked", "--version", "2.1.1"], {
  stdio: "inherit",
  env: { ...process.env, RUSTC_WRAPPER: "", RUSTC_WORKSPACE_WRAPPER: "" },
});
if (result.error) console.error(`Could not install kache: ${result.error.message}`);
process.exitCode = result.status ?? 1;
