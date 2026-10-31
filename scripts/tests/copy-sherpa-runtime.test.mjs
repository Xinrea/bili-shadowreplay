import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const script = fileURLToPath(new URL("../copy_sherpa_runtime.cjs", import.meta.url));
const dlls = [
  "onnxruntime.dll",
  "onnxruntime_providers_shared.dll",
  "sherpa-onnx-c-api.dll",
  "sherpa-onnx-cxx-api.dll",
];

function writeDlls(directory, marker) {
  fs.mkdirSync(directory, { recursive: true });
  for (const dll of dlls) fs.writeFileSync(path.join(directory, dll), `${marker}:${dll}`);
}

function fixture(callback) {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "bsr sherpa runtime-"));
  try {
    // The unmodified hook executes inside this isolated repository. Its writes
    // cannot reach the real ignored windows-runtime DLLs or Cargo artifacts.
    const repository = path.join(temporary, "repository");
    const native = path.join(repository, "src-tauri");
    const hook = path.join(repository, "scripts", "copy_sherpa_runtime.cjs");
    const destination = path.join(native, "windows-runtime");
    const cwd = path.join(temporary, "unrelated working directory");
    fs.mkdirSync(path.dirname(hook), { recursive: true });
    fs.mkdirSync(destination, { recursive: true });
    fs.mkdirSync(cwd);
    fs.copyFileSync(script, hook);
    callback({ temporary, native, hook, destination, cwd });
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

function runHook(files, profile, targetDirectory) {
  const env = { ...process.env };
  // Windows environment keys are case-insensitive. Avoid inheriting another
  // spelling from a developer's shell alongside this test's controlled values.
  for (const key of Object.keys(env)) {
    if (["CARGO_TARGET_DIR", "TAURI_ENV_DEBUG"].includes(key.toUpperCase())) delete env[key];
  }
  if (targetDirectory !== undefined) env.CARGO_TARGET_DIR = targetDirectory;
  if (profile === "debug") env.TAURI_ENV_DEBUG = "true";
  const result = spawnSync(process.execPath, [files.hook], {
    cwd: files.cwd,
    env,
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr || result.stdout);
}

for (const kind of ["absolute", "relative"]) {
  for (const profile of ["debug", "release"]) {
    test(`copies ${profile} DLLs from ${kind} CARGO_TARGET_DIR instead of stale default files`, () => {
      fixture((files) => {
        const targetDirectory = kind === "absolute"
          ? path.join(files.temporary, "external cargo target")
          : path.join("..", "relative cargo target");
        const current = path.join(path.resolve(files.native, targetDirectory), profile);
        const stale = path.join(files.native, "target", profile);
        writeDlls(current, "current");
        writeDlls(stale, "stale");
        writeDlls(files.destination, "previous package");

        runHook(files, profile, targetDirectory);

        for (const dll of dlls) {
          assert.equal(fs.readFileSync(path.join(files.destination, dll), "utf8"), `current:${dll}`);
          assert.equal(fs.readFileSync(path.join(stale, dll), "utf8"), `stale:${dll}`);
        }
      });
    });
  }
}

test("keeps the default release directory when CARGO_TARGET_DIR is absent", () => {
  fixture((files) => {
    writeDlls(path.join(files.native, "target", "release"), "default release");
    writeDlls(path.join(files.native, "target", "debug"), "wrong profile");
    runHook(files, "release");
    for (const dll of dlls) {
      assert.equal(fs.readFileSync(path.join(files.destination, dll), "utf8"), `default release:${dll}`);
    }
  });
});
