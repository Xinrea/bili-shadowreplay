import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

const settle = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };
const deferred = () => {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
};

async function fixture({ delayedSubscriptions = false } = {}) {
  const timeouts = new Map();
  const intervals = new Map();
  const handlers = new Map();
  const subscriptions = [];
  const removed = [];
  const documentHandlers = new Map();
  const windowHandlers = new Map();
  let id = 0;
  const document = {
    visibilityState: "visible",
    addEventListener: (name, cb) => documentHandlers.set(name, cb),
    removeEventListener: (name) => documentHandlers.delete(name),
  };
  const context = vm.createContext({
    console, document,
    window: {
      addEventListener: (name, cb) => windowHandlers.set(name, cb),
      removeEventListener: (name) => windowHandlers.delete(name),
    },
    setTimeout: (cb) => { timeouts.set(++id, cb); return id; },
    clearTimeout: (id) => timeouts.delete(id),
    setInterval: (cb) => { intervals.set(++id, cb); return id; },
    clearInterval: (id) => intervals.delete(id),
  });
  const invoker = new vm.SyntheticModule(["listen"], function () {
    this.setExport("listen", async (name, cb) => {
      handlers.set(name, cb);
      const ready = deferred();
      subscriptions.push(ready);
      if (delayedSubscriptions) await ready.promise;
      return () => { removed.push(name); handlers.delete(name); };
    });
  }, { context });
  const source = await readFile(new URL("../../src/lib/state-refresh.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, { compilerOptions: {
    target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext,
  } });
  const module = new vm.SourceTextModule(outputText, { context });
  await module.link(() => invoker);
  await module.evaluate();
  return {
    api: module.namespace, timeouts, intervals, handlers, subscriptions, removed,
    document, documentHandlers, windowHandlers,
    async flushTimers() {
      const callbacks = [...timeouts.values()];
      timeouts.clear();
      callbacks.forEach((cb) => cb());
      await settle();
    },
  };
}

test("a burst of invalidations causes one snapshot", async () => {
  const f = await fixture();
  let calls = 0;
  const refresh = f.api.createStateRefresh(async () => { calls++; });
  for (let i = 0; i < 100; i++) refresh.schedule();
  assert.equal(f.timeouts.size, 1);
  await f.flushTimers();
  assert.equal(calls, 1);
});

test("changes during a request produce a trailing snapshot without overlap", async () => {
  const f = await fixture();
  const first = deferred();
  let calls = 0;
  let active = 0;
  let maxActive = 0;
  const refresh = f.api.createStateRefresh(async () => {
    calls++;
    active++;
    maxActive = Math.max(active, maxActive);
    if (calls === 1) await first.promise;
    active--;
  });
  const initial = refresh.run();
  await settle();
  refresh.schedule();
  refresh.schedule();
  const manual = refresh.run();
  assert.equal(calls, 1);
  first.resolve();
  await Promise.all([initial, manual]);
  assert.equal(calls, 2);
  assert.equal(maxActive, 1);
});

test("a failed fetch does not stop queued changes or later refreshes", async () => {
  const f = await fixture();
  const first = deferred();
  let calls = 0;
  const errors = [];
  const refresh = f.api.createStateRefresh(async () => {
    calls++;
    if (calls === 1) { await first.promise; throw new Error("offline"); }
  }, 200, (error) => errors.push(error));
  const pending = refresh.run();
  await settle();
  refresh.schedule();
  first.resolve();
  await pending;
  assert.equal(calls, 2);
  assert.equal(errors.length, 1);
  await refresh.run();
  assert.equal(calls, 3);
});

test("initial snapshot waits for every subscription", async () => {
  const f = await fixture({ delayedSubscriptions: true });
  let calls = 0;
  const refresh = f.api.createStateRefresh(async () => { calls++; });
  const stop = f.api.watchStateChanges(refresh, ["videos-changed", "tasks-changed"]);
  await settle();
  assert.equal(calls, 0);
  // Events during setup are covered by the snapshot after all subscriptions.
  f.handlers.get("videos-changed")();
  f.handlers.get("state-resync")();
  await f.flushTimers();
  assert.equal(calls, 0);
  f.subscriptions[0].resolve();
  await settle();
  assert.equal(calls, 0);
  f.subscriptions.slice(1).forEach((subscription) => subscription.resolve());
  await settle();
  assert.equal(calls, 1);
  stop();
});

test("reconnect, visibility restoration, online and fallback repair missed events", async () => {
  const f = await fixture();
  let calls = 0;
  const refresh = f.api.createStateRefresh(async () => { calls++; });
  const stop = f.api.watchStateChanges(refresh, ["videos-changed"]);
  await settle();
  f.handlers.get("state-resync")();
  await settle();
  assert.equal(calls, 2);
  f.document.visibilityState = "hidden";
  f.handlers.get("videos-changed")();
  f.documentHandlers.get("visibilitychange")();
  [...f.intervals.values()][0]();
  await f.flushTimers();
  assert.equal(calls, 2);
  f.document.visibilityState = "visible";
  f.documentHandlers.get("visibilitychange")();
  await settle();
  assert.equal(calls, 3);
  f.windowHandlers.get("online")();
  await settle();
  assert.equal(calls, 4);
  [...f.intervals.values()][0]();
  await f.flushTimers();
  assert.equal(calls, 5);
  stop();
  assert.equal(f.handlers.size, 0);
  assert.equal(f.documentHandlers.size, 0);
  assert.equal(f.windowHandlers.size, 0);
  assert.equal(f.intervals.size, 0);
});

test("unmount during async subscription cleans up late listeners and skips fetch", async () => {
  const f = await fixture({ delayedSubscriptions: true });
  let calls = 0;
  const refresh = f.api.createStateRefresh(async () => { calls++; });
  const stop = f.api.watchStateChanges(refresh, ["videos-changed"]);
  stop();
  f.subscriptions.forEach((subscription) => subscription.resolve());
  await settle();
  assert.equal(calls, 0);
  assert.equal(f.handlers.size, 0);
  assert.equal(f.removed.length, 2);
  assert.equal(f.intervals.size, 0);
});

test("unmount discards timers and trailing refreshes while a request finishes", async () => {
  const f = await fixture();
  const first = deferred();
  let calls = 0;
  const refresh = f.api.createStateRefresh(async () => { calls++; await first.promise; });
  refresh.schedule();
  assert.equal(f.timeouts.size, 1);
  const pending = refresh.run();
  await settle();
  refresh.schedule();
  refresh.stop();
  first.resolve();
  await pending;
  await refresh.run();
  assert.equal(calls, 1);
  assert.equal(f.timeouts.size, 0);
});

async function invokerFixture({ tauri = false } = {}) {
  const socketHandlers = new Map();
  const tauriListeners = new Map();
  const context = vm.createContext({
    console: { log() {}, info() {}, error() {} },
    window: tauri ? { __TAURI_INTERNALS__: {} } : {},
    localStorage: { getItem: () => "" },
    fetch: async () => ({ ok: true, json: async () => ({ code: 0, data: null }) }),
  });
  const dependencies = {
    "@tauri-apps/api/core": { invoke: async () => null },
    "@tauri-apps/api/webviewWindow": { getCurrentWebviewWindow: () => ({}) },
    "@tauri-apps/plugin-http": { fetch: async () => ({}) },
    "@tauri-apps/api/event": { listen: async (name, cb) => {
      tauriListeners.set(name, cb);
      return () => tauriListeners.delete(name);
    } },
    "@tauri-apps/plugin-shell": { open: async () => {} },
    "@tauri-apps/plugin-deep-link": { onOpenUrl: async () => {} },
    "socket.io-client": { io: () => ({
      on: (name, cb) => socketHandlers.set(name, cb),
      onAny: (cb) => socketHandlers.set("*", cb),
      disconnect() {},
    }) },
  };
  const source = await readFile(new URL("../../src/lib/invoker.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, { compilerOptions: {
    target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext,
  } });
  const eventSource = await readFile(new URL("../../src/lib/events.ts", import.meta.url), "utf8");
  const eventOutput = ts.transpileModule(eventSource, { compilerOptions: {
    target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext,
  } }).outputText;
  const eventModule = new vm.SourceTextModule(eventOutput, { context });
  const module = new vm.SourceTextModule(outputText, { context });
  await module.link((name) => {
    if (name === "./events") return eventModule;
    assert.ok(dependencies[name], `Unexpected dependency: ${name}`);
    return new vm.SyntheticModule(Object.keys(dependencies[name]), function () {
      for (const [key, value] of Object.entries(dependencies[name])) this.setExport(key, value);
    }, { context });
  });
  await module.evaluate();
  return { api: module.namespace, socketHandlers, tauriListeners };
}

test("Web events reach the same listen API and each connect triggers reconciliation", async () => {
  const f = await invokerFixture();
  let resyncs = 0;
  const stop = await f.api.listen("state-resync", () => { resyncs++; });
  f.socketHandlers.get("connect")();
  f.socketHandlers.get("connect")();
  assert.equal(resyncs, 2);
  stop();
  for (const event of ["videos-changed", "tasks-changed", "recorders-changed", "records-changed", "state-resync"]) {
    let calls = 0;
    const unlisten = await f.api.listen(event, ({ payload }) => {
      assert.equal(payload, null);
      calls++;
    });
    f.socketHandlers.get("*")(event, null);
    assert.equal(calls, 1);
    unlisten();
    f.socketHandlers.get("*")(event, null);
    assert.equal(calls, 1);
  }
});

test("desktop state subscriptions use native Tauri events", async () => {
  const f = await invokerFixture({ tauri: true });
  let calls = 0;
  const unlisten = await f.api.listen("videos-changed", () => { calls++; });
  assert.equal(f.socketHandlers.size, 0);
  f.tauriListeners.get("videos-changed")({ payload: null });
  assert.equal(calls, 1);
  unlisten();
  assert.equal(f.tauriListeners.size, 0);
});

test("one dispatch path handles scoped progress, legacy progress and new event types", async () => {
  const f = await invokerFixture();
  const received = [];
  await f.api.listen("progress-update:task", ({ payload }) => received.push(payload.content));
  await f.api.listen("progress-finished:task", ({ payload }) => received.push(payload.success));
  await f.api.listen("custom-event", ({ payload }) => received.push(payload));
  const receive = f.socketHandlers.get("*");
  receive("progress-update:task", { id: "task", content: "encoding" });
  receive("progress-update", { id: "task", content: "legacy" });
  receive("progress-finished:task", { id: "task", success: true });
  receive("progress-update:another-task", { id: "another-task", content: "wrong task" });
  receive("custom-event", 42);
  assert.deepEqual(received, ["encoding", "legacy", true, 42]);
});

test("failing listeners and independent duplicate registrations do not disrupt dispatch", async () => {
  const f = await invokerFixture();
  let calls = 0;
  await f.api.listen("danmu", () => { throw new Error("broken listener"); });
  const callback = () => { calls++; };
  const first = await f.api.listen("danmu", callback);
  const second = await f.api.listen("danmu", callback);
  f.socketHandlers.get("*")("danmu", {});
  assert.equal(calls, 2);
  first();
  first();
  f.socketHandlers.get("*")("danmu", {});
  assert.equal(calls, 3);
  second();
  f.socketHandlers.get("*")("danmu", {});
  assert.equal(calls, 3);
});
