import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function loadModule() {
  const source = await readFile(new URL("../../src/lib/time-format.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const module = new vm.SourceTextModule(outputText, { context: vm.createContext({}) });
  await module.link(() => { throw new Error("time formatting must have no runtime dependencies"); });
  await module.evaluate();
  return module.namespace;
}

test("timecodes read as m:ss.s and grow hours past one hour", async () => {
  const { formatTimecode } = await loadModule();
  assert.equal(formatTimecode(0), "0:00.0");
  assert.equal(formatTimecode(9.25), "0:09.3");
  assert.equal(formatTimecode(75), "1:15.0");
  assert.equal(formatTimecode(3600), "1:00:00.0");
  assert.equal(formatTimecode(3723.4), "1:02:03.4");
});

test("seconds round into the next field instead of showing :60", async () => {
  const { formatTimecode } = await loadModule();
  // Per-field toFixed renders 59.96 as "0:60.0"; rounding first must not.
  assert.equal(formatTimecode(59.96), "1:00.0");
  assert.equal(formatTimecode(3599.97), "1:00:00.0");
  assert.equal(formatTimecode(59.6, 0), "1:00");
});

test("precision is configurable and negatives clamp to zero", async () => {
  const { formatTimecode } = await loadModule();
  assert.equal(formatTimecode(61, 0), "1:01");
  assert.equal(formatTimecode(3661, 0), "1:01:01");
  assert.equal(formatTimecode(9.876, 2), "0:09.88");
  assert.equal(formatTimecode(-5), "0:00.0");
});
