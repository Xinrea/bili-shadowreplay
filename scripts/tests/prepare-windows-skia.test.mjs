import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { createHash } from "node:crypto";

import { cachedSkiaArchive, checkArchiveIdentity, parseOptions, prepareWindowsSkia } from "../prepare-windows-skia.mjs";

test("prepares Vulkan by default and keeps the explicit Vulkan flag compatible", () => {
  const gpu = parseOptions([]);
  assert.deepEqual(parseOptions(["--vulkan"]), gpu);
  assert.equal(gpu.key, "7ca7f6be2332d3c9f3ad-x86_64-pc-windows-msvc-textlayout-vulkan");
  assert.equal(gpu.sha256, "2d483906db2c7ee55983605ea71a6ceecf64329c6e1586276b10e38870fd1fbc");
  assert.ok(gpu.url.endsWith(`/0.82.0/${gpu.archive}`));
});

test("selects the CPU archive only when explicitly requested", () => {
  const cpu = parseOptions(["--cpu"]);
  const gpu = parseOptions([]);
  assert.equal(cpu.key, "7ca7f6be2332d3c9f3ad-x86_64-pc-windows-msvc-textlayout");
  assert.equal(gpu.key, `${cpu.key}-vulkan`);
  assert.equal(gpu.outputDir, cpu.outputDir);
  assert.equal(cpu.sha256, "300dcfe6ed498b37b5f3c0ef4ac6b8f23e4f6cf5cd603cd28c3139ca7de29eaf");
  assert.notEqual(gpu.sha256, cpu.sha256);
  assert.ok(cpu.url.endsWith(`/0.82.0/${cpu.archive}`));
});

test("keeps the existing positional output directory and accepts either flag order", () => {
  const directory = path.join(os.tmpdir(), "skia cache with spaces");
  assert.equal(parseOptions([directory]).outputDir, directory);
  assert.deepEqual(parseOptions([directory, "--vulkan"]), parseOptions(["--vulkan", directory]));
  assert.deepEqual(parseOptions([directory, "--cpu"]), parseOptions(["--cpu", directory]));
});

test("rejects unknown options and ambiguous output paths before preparing an archive", () => {
  assert.throws(() => parseOptions(["--vulakn"]), /Unknown option: --vulakn/);
  assert.throws(() => parseOptions(["one", "two"]), /Only one output directory/);
  assert.throws(() => parseOptions(["--cpu", "--vulkan"]), /cannot be combined/);
  assert.throws(() => parseOptions(["--vulkan", "--cpu"]), /cannot be combined/);
});

function withIdentity(key, tag, callback) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "bsr-skia-identity-"));
  try {
    fs.writeFileSync(path.join(directory, "key.txt"), `${key}\r\n`);
    fs.writeFileSync(path.join(directory, "tag.txt"), `${tag}\r\n`);
    callback(directory);
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
}

test("rejects the CPU archive when the build requested Vulkan", () => {
  const gpu = parseOptions(["--vulkan"]);
  withIdentity(parseOptions(["--cpu"]).key, "0.82.0", (directory) => {
    assert.throws(() => checkArchiveIdentity(directory, gpu), /Skia archive identifies .* expected/);
  });
});

test("checks embedded release identity as well as the feature key", () => {
  const gpu = parseOptions(["--vulkan"]);
  withIdentity(gpu.key, "0.81.0", (directory) => {
    assert.throws(() => checkArchiveIdentity(directory, gpu), /expected 0\.82\.0/);
  });
  withIdentity(gpu.key, "0.82.0", (directory) => {
    assert.doesNotThrow(() => checkArchiveIdentity(directory, gpu));
  });
});

test("reuses only a verified patched Skia archive and rejects corruption without downloading", async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "bsr-skia-cache-"));
  try {
    const options = parseOptions([directory]);
    const archive = path.join(directory, options.archive);
    const receipt = `${archive}.json`;
    fs.writeFileSync(archive, "patched fixture archive");
    assert.equal(cachedSkiaArchive(options), false, "legacy archives without receipts must be prepared again");
    const saved = { source: options.sha256, patchRevision: 1, sha256: createHash("sha256").update(fs.readFileSync(archive)).digest("hex") };
    fs.writeFileSync(receipt, JSON.stringify(saved));
    assert.equal(cachedSkiaArchive(options), true);
    // No tools or network are needed when the cached output is verified.
    assert.equal(await prepareWindowsSkia(options, { PATH: "" }), `file://${directory.replaceAll("\\", "/")}/skia-binaries-{key}.tar.gz`);
    fs.appendFileSync(archive, "corrupted");
    await assert.rejects(prepareWindowsSkia(options, { PATH: "" }), /Invalid patched Skia cache/);
    fs.writeFileSync(receipt, JSON.stringify({ ...saved, patchRevision: 0 }));
    assert.equal(cachedSkiaArchive(options), false, "changed patch code invalidates older cached output");
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});
