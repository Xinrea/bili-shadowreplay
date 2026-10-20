import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function loadModule() {
  const source = await readFile(new URL("../../src/lib/size-format.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const module = new vm.SourceTextModule(outputText, { context: vm.createContext({}) });
  await module.link(() => { throw new Error("size formatting must have no runtime dependencies"); });
  await module.evaluate();
  return module.namespace;
}

test("sizes below 1 KiB print as whole bytes", async () => {
  const { formatSize } = await loadModule();
  assert.equal(formatSize(0), "0 B");
  assert.equal(formatSize(512), "512 B");
  assert.equal(formatSize(1023), "1023 B");
});

test("larger sizes switch to binary units with two decimals", async () => {
  const { formatSize } = await loadModule();
  assert.equal(formatSize(1024), "1.00 KiB");
  assert.equal(formatSize(1536), "1.50 KiB");
  assert.equal(formatSize(1024 * 1024), "1.00 MiB");
  assert.equal(formatSize(5.25 * 1024 * 1024), "5.25 MiB");
  assert.equal(formatSize(1024 ** 3), "1.00 GiB");
  assert.equal(formatSize(2.5 * 1024 ** 4), "2560.00 GiB");
});
