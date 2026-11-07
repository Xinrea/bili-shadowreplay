import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { windowsEnvironment, parseDeveloperEnvironment } from "../windows-runtime.mjs";

test("Windows setup has one case-insensitive PATH and preserves values containing equals signs", () => {
  const base = { Path: "old path", FFMPEG_DIR: "C:\\SDK with spaces" };
  assert.deepEqual(windowsEnvironment(base), { PATH: "old path", FFMPEG_DIR: "C:\\SDK with spaces" });
  const env = parseDeveloperEnvironment("VS banner\r\nPath=C:\\MSVC;old path\r\nINCLUDE=C:\\include\r\nVALUE=one=two\r\n=C:=C:\\work\r\n", base);
  assert.equal(env.PATH, "C:\\MSVC;old path");
  assert.equal(env.FFMPEG_DIR, base.FFMPEG_DIR);
  assert.equal(env.VALUE, "one=two");
  assert.equal(Object.hasOwn(env, "Path"), false);
  assert.equal(Object.hasOwn(env, ""), false);
  assert.equal(base.Path, "old path");
});

test("Windows FFmpeg setup honors a custom SDK, exports environment and deploys only DLLs", { skip: process.platform !== "win32" }, () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "bsr ffmpeg setup-"));
  try {
    const script = path.join(directory, "ffmpeg_setup.ps1");
    fs.copyFileSync(new URL("../../ffmpeg_setup.ps1", import.meta.url), script);
    const sdk = path.join(directory, "custom SDK");
    const files = ["include/libavformat/avformat.h", "include/libswresample/swresample.h", "include/libavfilter/avfilter.h", "lib/avformat.lib", "lib/avcodec.lib", "lib/avutil.lib", "lib/swresample.lib", "lib/avfilter.lib", "bin/avformat-63.dll", "bin/avcodec-63.dll", "bin/avutil-61.dll", "bin/swresample-7.dll", "bin/avfilter-12.dll", "bin/codec-helper.dll", "bin/ffmpeg.exe"];
    for (const file of files) {
      fs.mkdirSync(path.dirname(path.join(sdk, file)), { recursive: true });
      fs.writeFileSync(path.join(sdk, file), file);
    }
    const runtime = path.join(directory, "src-tauri/windows-runtime");
    fs.mkdirSync(runtime, { recursive: true });
    fs.writeFileSync(path.join(runtime, "sherpa-onnx-c-api.dll"), "keep");
    const receipt = path.join(directory, "environment.json");
    const env = windowsEnvironment({ ...process.env, FFMPEG_DIR: sdk, LIBCLANG_PATH: "C:\\LLVM" });
    delete env.GITHUB_ENV;
    delete env.GITHUB_PATH;
    const args = ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", script, "-EnvironmentFile", receipt];
    const run = () => execFileSync("powershell.exe", args, { env, stdio: "pipe" });
    run();
    const configured = JSON.parse(fs.readFileSync(receipt, "utf8").replace(/^\uFEFF/, ""));
    assert.equal(configured.FFMPEG_DIR, sdk);
    assert.equal(configured.LIBCLANG_PATH, "C:\\LLVM");
    assert.ok(configured.PATH.startsWith(`${sdk}\\bin;`));
    assert.equal(fs.existsSync(path.join(runtime, "ffmpeg.exe")), false);
    assert.equal(fs.readFileSync(path.join(runtime, "sherpa-onnx-c-api.dll"), "utf8"), "keep");
    const copied = path.join(runtime, "avformat-63.dll");
    const before = fs.statSync(copied).mtimeMs;
    run();
    assert.equal(fs.statSync(copied).mtimeMs, before, "warm launches do not rewrite loaded DLLs");
    assert.equal(fs.readFileSync(path.join(runtime, "codec-helper.dll"), "utf8"), "bin/codec-helper.dll");

    const cached = path.join(directory, "src-tauri/target/ffmpeg-sdk/9.0.2-4d2060a8b34a");
    fs.cpSync(sdk, cached, { recursive: true });
    const hashes = Object.fromEntries(files.map((file) => [file, createHash("sha256").update(fs.readFileSync(path.join(cached, file))).digest("hex")]));
    fs.writeFileSync(path.join(cached, "bsr-sdk.json"), JSON.stringify({ archive: "4d2060a8b34a940aa47d785142055bb92a63053781e55f2ace4546edd519a8f5", files: hashes }));
    const cachedEnv = { ...env };
    delete cachedEnv.FFMPEG_DIR;
    execFileSync("powershell.exe", args, { env: cachedEnv, stdio: "pipe" });
    fs.appendFileSync(path.join(cached, "lib/avformat.lib"), "corrupted");
    const corrupt = spawnSync("powershell.exe", args, { env: cachedEnv, encoding: "utf8" });
    assert.notEqual(corrupt.status, 0);
    assert.match(corrupt.stderr, /Invalid FFmpeg cache/);

    fs.rmSync(path.join(sdk, "lib/avformat.lib"));
    const rejected = spawnSync("powershell.exe", args, { env, encoding: "utf8" });
    assert.notEqual(rejected.status, 0);
    assert.match(rejected.stderr, /Incomplete FFmpeg 9 SDK/);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});
