import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { once } from "node:events";
import { Worker } from "node:worker_threads";
import { downloadMoltenVkArchive, ffmpegEnvironment, prepareMoltenVk, verifyArchive } from "../macos-runtime.mjs";

test("interrupted MoltenVK downloads retain their bytes and verified archives work offline", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "bsr-resume-"));
  const bytes = Buffer.from("complete fixture archive");
  const archive = { url: "https://example.invalid/runtime.tar", sha256: createHash("sha256").update(bytes).digest("hex") };
  try {
    assert.throws(() => downloadMoltenVkArchive(directory, archive, (_url, file) => {
      fs.writeFileSync(file, bytes.subarray(0, 8));
      throw new Error("connection interrupted");
    }), /connection interrupted/);
    const partials = fs.readdirSync(directory);
    assert.equal(partials.length, 1);
    assert.deepEqual(fs.readFileSync(path.join(directory, partials[0])), bytes.subarray(0, 8));
    const tar = downloadMoltenVkArchive(directory, archive, (_url, file) => {
      assert.deepEqual(fs.readFileSync(file), bytes.subarray(0, 8));
      fs.appendFileSync(file, bytes.subarray(8));
    });
    assert.deepEqual(fs.readFileSync(tar), bytes);
    assert.deepEqual(fs.readdirSync(directory), [path.basename(tar)]);
    assert.equal(downloadMoltenVkArchive(directory, archive, () => { throw new Error("offline"); }), tar);
    fs.writeFileSync(tar, "corrupt cache");
    assert.throws(() => downloadMoltenVkArchive(directory, archive, (_url, file) => {
      assert.equal(fs.statSync(file).size, 0);
      fs.writeFileSync(file, "corrupt response");
    }), /SHA-256 mismatch/);
    assert.deepEqual(fs.readdirSync(directory), [], "corrupt data must not poison the next resume");
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});

test("MoltenVK resumes after process exit without touching a live launcher's download", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "bsr-resume-exit-"));
  const bytes = Buffer.from("complete fixture archive");
  const archive = { url: "https://example.invalid/runtime.tar", sha256: createHash("sha256").update(bytes).digest("hex") };
  try {
    const result = spawnSync(process.execPath, ["--input-type=module", "-e", `
      import fs from 'node:fs';
      import { downloadMoltenVkArchive } from ${JSON.stringify(new URL("../macos-runtime.mjs", import.meta.url).href)};
      downloadMoltenVkArchive(process.argv[1], ${JSON.stringify(archive)}, (_url, file) => {
        fs.writeFileSync(file, 'complete');
        process.exit(23);
      });
    `, directory], { encoding: "utf8" });
    assert.equal(result.status, 23, result.stderr);
    const tar = downloadMoltenVkArchive(directory, archive, (_url, outer) => {
      assert.equal(fs.readFileSync(outer, "utf8"), "complete");
      downloadMoltenVkArchive(directory, archive, (_url, inner) => {
        assert.notEqual(inner, outer);
        assert.equal(fs.statSync(inner).size, 0, "an active writer cannot be used as a resume source");
        fs.writeFileSync(inner, bytes);
      });
      assert.equal(fs.readFileSync(outer, "utf8"), "complete");
      fs.appendFileSync(outer, bytes.subarray(8));
    });
    assert.deepEqual(fs.readFileSync(tar), bytes);
    assert.deepEqual(fs.readdirSync(directory), [path.basename(tar)]);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});

test("curl retries from received bytes and safely restarts when HTTP resume is unavailable", async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "bsr-curl-resume-"));
  const bytes = "0123456789-complete-download";
  const history = path.join(directory, "requests.json");
  // A separate worker serves HTTP while the launcher is in execFileSync.
  const worker = new Worker(`
    const { parentPort, workerData } = require('node:worker_threads');
    const http = require('node:http');
    const fs = require('node:fs');
    const requests = [];
    const body = Buffer.from(workerData.bytes);
    const server = http.createServer((req, res) => {
      const range = req.headers.range || null;
      requests.push([req.url, range]);
      fs.writeFileSync(workerData.history, JSON.stringify(requests));
      if (req.url === '/resume' && !range) {
        res.writeHead(200, { 'Content-Length': body.length });
        res.write(body.subarray(0, 8));
        setTimeout(() => res.destroy(), 30);
      } else if (req.url === '/out-of-range' && range) {
        res.writeHead(416, { 'Content-Range': 'bytes */' + body.length });
        res.end();
      } else if (req.url === '/resume' && range) {
        const offset = Number(range.match(/bytes=(\\d+)-/)[1]);
        res.writeHead(206, { 'Content-Length': body.length - offset,
          'Content-Range': 'bytes ' + offset + '-' + (body.length - 1) + '/' + body.length });
        res.end(body.subarray(offset));
      } else {
        res.writeHead(200, { 'Content-Length': body.length });
        res.end(body);
      }
    });
    server.listen(0, '127.0.0.1', () => parentPort.postMessage(server.address().port));
  `, { eval: true, workerData: { bytes, history } });
  try {
    const [port] = await once(worker, "message");
    for (const route of ["resume", "no-range", "out-of-range"]) {
      const cache = path.join(directory, route);
      const archive = { url: `http://127.0.0.1:${port}/${route}`, sha256: createHash("sha256").update(bytes).digest("hex") };
      if (route !== "resume") {
        assert.throws(() => downloadMoltenVkArchive(cache, archive, (_url, file) => {
          fs.writeFileSync(file, bytes.slice(0, 8));
          throw new Error("interrupted");
        }), /interrupted/);
      }
      const tar = downloadMoltenVkArchive(cache, archive);
      assert.equal(fs.readFileSync(tar, "utf8"), bytes);
    }
    assert.deepEqual(JSON.parse(fs.readFileSync(history, "utf8")), [
      ["/resume", null], ["/resume", "bytes=8-"],
      ["/no-range", "bytes=8-"], ["/no-range", null],
      ["/out-of-range", "bytes=8-"], ["/out-of-range", null],
    ]);
  } finally {
    await worker.terminate();
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

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
      assert.deepEqual(fs.readdirSync(racingCache).sort(), [`MoltenVK-${sha256}.tar`, path.basename(winner)].sort(), "only verified archives and prepared runtimes remain");
    }

    fs.appendFileSync(downloaded, "corruption");
    assert.throws(() => prepareMoltenVk(cache, options), /Invalid MoltenVK cache/);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});
