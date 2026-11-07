import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

/** The module with a fake ResizeObserver and timers driven by the test. */
async function loadModule() {
  const source = await readFile(new URL("../../src/lib/floating-overlay.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const observers = [];
  const timers = new Map();
  let nextTimer = 1;
  const context = vm.createContext({
    ResizeObserver: class {
      constructor(callback) {
        this.callback = callback;
        this.disconnected = false;
        observers.push(this);
      }
      observe() {}
      disconnect() {
        this.disconnected = true;
      }
    },
    setTimeout: (callback) => {
      timers.set(nextTimer, callback);
      return nextTimer++;
    },
    clearTimeout: (id) => timers.delete(id),
  });
  const module = new vm.SourceTextModule(outputText, { context });
  await module.link(() => {
    throw new Error("floating-overlay.ts must be self-contained");
  });
  await module.evaluate();
  const harness = {
    /** The observer reports a resize. */
    resize() {
      for (const observer of observers) if (!observer.disconnected) observer.callback([]);
    },
    /** The pending timers fire. */
    settle() {
      const due = [...timers.values()];
      timers.clear();
      for (const callback of due) callback();
    },
    pending: () => timers.size,
  };
  return { ...module.namespace, harness };
}

test("observeResize lays out again once a new size settles", async () => {
  const { observeResize, harness } = await loadModule();
  const overlay = { clientWidth: 992, clientHeight: 558 };
  let relayouts = 0;
  observeResize(overlay, () => relayouts++);

  // The first report, at the size it started with, changes nothing.
  harness.resize();
  harness.settle();
  assert.equal(relayouts, 0);

  // Reports while dragging wait for the size to settle, then lay out once.
  overlay.clientWidth = 900;
  harness.resize();
  overlay.clientWidth = 800;
  overlay.clientHeight = 450;
  harness.resize();
  assert.equal(harness.pending(), 1);
  harness.settle();
  assert.equal(relayouts, 1);

  // Back and forth to the same size is no new size.
  harness.resize();
  harness.settle();
  assert.equal(relayouts, 1);
});

test("observeResize stops when told", async () => {
  const { observeResize, harness } = await loadModule();
  const overlay = { clientWidth: 992, clientHeight: 558 };
  let relayouts = 0;
  const stop = observeResize(overlay, () => relayouts++);
  overlay.clientWidth = 800;
  harness.resize();
  stop();
  assert.equal(harness.pending(), 0);
  harness.resize();
  harness.settle();
  assert.equal(relayouts, 0);
});
