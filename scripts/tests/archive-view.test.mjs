import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { get, writable } from "svelte/store";

async function load(storage) {
  const source = await readFile(new URL("../../src/lib/stores/archive-view.ts",import.meta.url),"utf8");
  const { outputText } = ts.transpileModule(source,{ compilerOptions:{ target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext } });
  const context = vm.createContext({ localStorage:storage });
  const stores = new vm.SyntheticModule(["writable"],function() { this.setExport("writable",writable); },{ context });
  const module = new vm.SourceTextModule(outputText,{ context });
  await module.link(() => stores);
  await module.evaluate();
  return module.namespace;
}

test("both archive surfaces see the same switch and restore the saved view only once", async () => {
  let reads = 0;
  const saved = new Map([["archive-view-mode","calendar"]]);
  const api = await load({ getItem(key) { reads++; return saved.get(key); },setItem(key,value) { saved.set(key,value); } });
  api.restoreArchiveViewPreference();
  assert.equal(get(api.archiveViewMode),"calendar");
  const first = [], second = [];
  const a = api.archiveViewMode.subscribe(value => first.push(value));
  const b = api.archiveViewMode.subscribe(value => second.push(value));
  api.setArchiveViewMode("list");
  api.restoreArchiveViewPreference();
  assert.equal(reads,1);
  assert.deepEqual(first,["calendar","list"]);
  assert.deepEqual(second,first);
  assert.equal(saved.get("archive-view-mode"),"list");
  a(); b();
});

test("invalid preferences and unavailable storage leave the view switch usable", async () => {
  const invalid = await load({ getItem:() => "unknown",setItem() {} });
  invalid.restoreArchiveViewPreference();
  assert.equal(get(invalid.archiveViewMode),"list");
  const unavailable = await load({ getItem() { throw new Error("blocked"); },setItem() { throw new Error("blocked"); } });
  unavailable.restoreArchiveViewPreference();
  unavailable.setArchiveViewMode("calendar");
  assert.equal(get(unavailable.archiveViewMode),"calendar");
});

test("the previous timeline preference opens the replacement calendar", async () => {
  const api = await load({ getItem:() => "timeline",setItem() {} });
  api.restoreArchiveViewPreference();
  assert.equal(get(api.archiveViewMode),"calendar");
});
