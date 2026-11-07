import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { ffmpegEnvironment, prepareMoltenVk, verifyArchive } from "../macos-runtime.mjs";

test("macOS SDK discovery preserves an explicit SDK and its pkg-config dependencies", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "bsr-sdk with spaces-"));
  try {
    fs.mkdirSync(path.join(directory, "include/libavutil"), { recursive: true });
    fs.mkdirSync(path.join(directory, "lib"));
    fs.writeFileSync(path.join(directory, "include/libavutil/avutil.h"), "");
    fs.writeFileSync(path.join(directory, "lib/libavutil.dylib"), "");
    const env = { FFMPEG_DIR: directory, PKG_CONFIG_PATH: "/custom/dependency", PATH: "/tools" };
    const configured = ffmpegEnvironment(env);
    assert.equal(configured.FFMPEG_DIR, directory);
    assert.equal(configured.PKG_CONFIG_PATH, `${path.join(directory, "lib/pkgconfig")}${path.delimiter}/custom/dependency`);
    assert.equal(configured.PATH, env.PATH);
    assert.equal(env.PKG_CONFIG_PATH, "/custom/dependency", "do not mutate the caller's environment");
    assert.throws(() => ffmpegEnvironment({ FFMPEG_DIR: path.join(directory, "missing") }), /FFMPEG_DIR does not contain/);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});

test("MoltenVK downloads reject an incorrect checksum before extraction or publication", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "bsr-molten-checksum-"));
  try {
    assert.throws(() => prepareMoltenVk(directory, {
      download: (_url, file) => fs.writeFileSync(file, "untrusted archive"),
    }), /SHA-256 mismatch/);
    assert.deepEqual(fs.readdirSync(directory), []);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});

test("prepared macOS runtimes load through FFmpeg's bare filename, cache offline, and preserve source files", { skip: process.platform !== "darwin" }, () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "bsr-runtime with spaces-"));
  const run = (program, args) => execFileSync(program, args, { encoding: "utf8" });
  try {
    const source = path.join(directory, "source.c");
    const dylib = path.join(directory, "libMoltenVK.dylib");
    fs.writeFileSync(source, "void vkExportMetalObjectsEXT(void) {}\n");
    run("cc", ["-dynamiclib", "-mmacosx-version-min=15.0", "-Wl,-headerpad_max_install_names", source, "-o", dylib]);
    const original = fs.readFileSync(dylib);
    const cache = path.join(directory, "cache");
    const runtime = prepareMoltenVk(cache, { source: dylib });
    assert.deepEqual(fs.readFileSync(dylib), original);
    run("codesign", ["--verify", "--strict", runtime]);
    assert.equal(prepareMoltenVk(cache, { source: dylib }), runtime);

    const probe = path.join(directory, "probe");
    fs.writeFileSync(source, `#include <dlfcn.h>
int main(int argc, char **argv) {
  void *loaded = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
  return !loaded || loaded != dlopen("libvulkan.dylib", RTLD_NOW | RTLD_LOCAL)
    || !dlsym(loaded, "vkExportMetalObjectsEXT");
}\n`);
    run("cc", [source, "-o", probe]);
    const env = { ...process.env };
    delete env.DYLD_LIBRARY_PATH;
    delete env.DYLD_FALLBACK_LIBRARY_PATH;
    const result = spawnSync(probe, [runtime], { env, encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);

    // Exercise the same download/check/extract/cache path without networking.
    fs.mkdirSync(path.join(directory, "archive"));
    fs.copyFileSync(dylib, path.join(directory, "archive/libMoltenVK.dylib"));
    fs.writeFileSync(path.join(directory, "archive/LICENSE"), "fixture license");
    const tar = path.join(directory, "fixture.tar");
    run("tar", ["-cf", tar, "-C", path.join(directory, "archive"), "libMoltenVK.dylib", "LICENSE"]);
    const sha256 = createHash("sha256").update(fs.readFileSync(tar)).digest("hex");
    const archive = { sha256, library: "libMoltenVK.dylib", license: "LICENSE", url: "https://example.invalid/fixture.tar" };
    verifyArchive(tar, sha256);
    let downloads = 0;
    const options = { archive, download: (_url, file) => { downloads++; fs.copyFileSync(tar, file); } };
    const downloaded = prepareMoltenVk(cache, options);
    assert.equal(fs.readFileSync(path.join(path.dirname(downloaded), "LICENSE"), "utf8"), "fixture license");
    assert.equal(prepareMoltenVk(cache, { archive, download: () => { throw new Error("offline"); } }), downloaded);
    assert.equal(downloads, 1);

    // Another launcher publishes while this one downloads. Its receipt alone
    // is insufficient: the winning library must pass the native checks too.
    for (const invalid of [false, true]) {
      const racingCache = path.join(directory, `racing-cache-${invalid}`);
      const winner = path.join(racingCache, path.basename(path.dirname(downloaded)));
      const racingOptions = { archive, download: (_url, file) => {
        fs.copyFileSync(tar, file);
        fs.cpSync(path.dirname(downloaded), winner, { recursive: true });
        if (invalid) {
          const contents = "not a dynamic library";
          fs.writeFileSync(path.join(winner, "libvulkan.dylib"), contents);
          fs.writeFileSync(path.join(winner, "runtime.json"), JSON.stringify({
            sha256: createHash("sha256").update(contents).digest("hex"),
          }));
        }
      } };
      if (invalid) assert.throws(() => prepareMoltenVk(racingCache, racingOptions), /Invalid MoltenVK cache/);
      else assert.equal(prepareMoltenVk(racingCache, racingOptions), path.join(winner, "libvulkan.dylib"));
      assert.deepEqual(fs.readdirSync(racingCache), [path.basename(winner)], "temporary preparation files are removed");
    }

    fs.appendFileSync(downloaded, "corruption");
    assert.throws(() => prepareMoltenVk(cache, options), /Invalid MoltenVK cache/);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});
