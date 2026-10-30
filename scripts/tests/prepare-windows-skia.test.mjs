import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { checkArchiveIdentity, parseOptions } from "../prepare-windows-skia.mjs";

test("selects the Vulkan feature archive without changing the default CPU archive", () => {
  const cpu = parseOptions([]);
  const gpu = parseOptions(["--vulkan"]);
  assert.equal(cpu.key, "7ca7f6be2332d3c9f3ad-x86_64-pc-windows-msvc-textlayout");
  assert.equal(gpu.key, `${cpu.key}-vulkan`);
  assert.equal(gpu.outputDir, cpu.outputDir);
  assert.equal(gpu.sha256, "2d483906db2c7ee55983605ea71a6ceecf64329c6e1586276b10e38870fd1fbc");
  assert.notEqual(gpu.sha256, cpu.sha256);
  assert.ok(gpu.url.endsWith(`/0.82.0/${gpu.archive}`));
});

test("keeps the existing positional output directory and accepts either flag order", () => {
  const directory = path.join(os.tmpdir(), "skia cache with spaces");
  assert.equal(parseOptions([directory]).outputDir, directory);
  assert.deepEqual(parseOptions([directory, "--vulkan"]), parseOptions(["--vulkan", directory]));
});

test("rejects unknown options and ambiguous output paths before preparing an archive", () => {
  assert.throws(() => parseOptions(["--vulakn"]), /Unknown option: --vulakn/);
  assert.throws(() => parseOptions(["one", "two"]), /Only one output directory/);
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
  withIdentity(parseOptions([]).key, "0.82.0", (directory) => {
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
