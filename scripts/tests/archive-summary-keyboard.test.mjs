import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../../src/lib/components/ArchiveSummaryModal.svelte", import.meta.url), "utf8");
const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
const { outputText } = ts.transpileModule(`${script}
  export { handleKeydown };
  export function isOpen() { return showModal; }
`, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } });

async function fixture() {
  let visible = true;
  const context = vm.createContext({
    $state: value => value,
    $derived: value => value,
    $bindable: value => value,
    $effect() {},
    $props: () => ({ showModal: true }),
  });
  const dependencies = {
    svelte: { onDestroy() {} },
    "../page-visibility": { pageVisibility: () => () => visible },
    "../invoker": { invoke() { throw new Error("Unexpected API request"); } },
  };
  const module = new vm.SourceTextModule(outputText, { context });
  await module.link(name => {
    const exports = dependencies[name];
    assert.ok(exports, `Unexpected dependency: ${name}`);
    return new vm.SyntheticModule(Object.keys(exports), function () {
      for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
    }, { context });
  });
  await module.evaluate();
  return { handlers: module.namespace, setVisible: value => { visible = value; } };
}

function keyEvent(changes = {}) {
  return {
    key: "Escape", defaultPrevented: false, isComposing: false,
    preventCalls: 0, stopCalls: 0,
    preventDefault() { this.defaultPrevented = true; this.preventCalls++; },
    stopImmediatePropagation() { this.stopCalls++; },
    ...changes,
  };
}

test("a hidden summary preserves Escape for the active page's dialog", async () => {
  const f = await fixture();
  f.setVisible(false);
  const event = keyEvent();
  f.handlers.handleKeydown(event);
  assert.equal(f.handlers.isOpen(), true);
  assert.equal(event.preventCalls, 0);
  assert.equal(event.stopCalls, 0);
});

test("returning to the summary's page restores Escape dismissal", async () => {
  const f = await fixture();
  f.setVisible(false);
  f.handlers.handleKeydown(keyEvent());
  f.setVisible(true);
  const event = keyEvent();
  f.handlers.handleKeydown(event);
  assert.equal(f.handlers.isOpen(), false);
  assert.equal(event.preventCalls, 1);
  assert.equal(event.stopCalls, 1);
  const closed = keyEvent();
  f.handlers.handleKeydown(closed);
  assert.equal(closed.preventCalls, 0);
  assert.equal(closed.stopCalls, 0);
});

test("a visible summary leaves composed, already handled, and other keys untouched", async () => {
  const f = await fixture();
  for (const changes of [{ isComposing: true }, { defaultPrevented: true }, { key: "Enter" }]) {
    const event = keyEvent(changes);
    f.handlers.handleKeydown(event);
    assert.equal(f.handlers.isOpen(), true);
    assert.equal(event.preventCalls, 0);
    assert.equal(event.stopCalls, 0);
  }
});
