import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function loadModule() {
  const source = await readFile(new URL("../../src/lib/virtual-list.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const module = new vm.SourceTextModule(outputText, { context: vm.createContext({}) });
  await module.link(() => {
    throw new Error("The list must have no runtime dependencies");
  });
  await module.evaluate();
  return module.namespace;
}

const { shownRows } = await loadModule();
const plain = (value) => JSON.parse(JSON.stringify(value));

test("the rows in view are drawn, with some more each way", () => {
  // 7 rows of 38 px in view from row 100.
  assert.deepEqual(plain(shownRows({ top: 3800, height: 266 }, 38, 2, 3000)), [98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108]);
  // At the top, none before the first.
  assert.deepEqual(plain(shownRows({ top: 0, height: 76 }, 38, 2, 3000)), [0, 1, 2, 3, 4, 5]);
});

test("the selected row is drawn too, wherever it is", () => {
  const rows = shownRows({ top: 0, height: 76 }, 38, 1, 3000, 2500);
  assert.deepEqual(plain(rows), [0, 1, 2, 3, 2500]);
  // Not twice when it is in view, and not when it is gone.
  assert.deepEqual(plain(shownRows({ top: 0, height: 76 }, 38, 1, 3000, 1)), [0, 1, 2, 3]);
  assert.deepEqual(plain(shownRows({ top: 0, height: 76 }, 38, 1, 10, 50)), [0, 1, 2, 3]);
});

test("a view left past the end of a list that got shorter shows its last rows", () => {
  // Scrolled to row 1500 of 3000, then only 200 are left.
  assert.deepEqual(plain(shownRows({ top: 57000, height: 266 }, 38, 6, 200)), Array.from({ length: 19 }, (_, i) => 181 + i));
  // Fewer than fit: all of them.
  assert.deepEqual(plain(shownRows({ top: 57000, height: 266 }, 38, 6, 5)), [0, 1, 2, 3, 4]);
  assert.deepEqual(plain(shownRows({ top: 0, height: 266 }, 38, 6, 0)), []);
});
