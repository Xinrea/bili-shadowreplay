import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import * as svelteStore from "svelte/store";

async function transpile(path) {
  const source = await readFile(new URL(`../../src/lib/${path}`, import.meta.url), "utf8");
  return ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  }).outputText;
}

// Values built inside the vm context have their own prototypes.
const plain = (value) => JSON.parse(JSON.stringify(value));

const row = (id, status, created_at, metadata = "{}") =>
  ({ id, task_type: "clip_range", status, message: "", metadata, created_at });

async function fixture() {
  // With `manual`, each snapshot waits for the test to resolve it with rows.
  const backend = { rows: [], cancel: null, cancelled: null, manual: false, calls: [] };
  const listeners = new Map();
  let refresh = null;
  let watching = false;
  const context = vm.createContext({ Date, JSON, Map, Set, Promise });
  const helpers = new vm.SourceTextModule(await transpile("task-progress.ts"), { context });
  await helpers.link(() => { throw new Error("task-progress must have no runtime dependencies"); });
  const dependencies = {
    "svelte/store": svelteStore,
    "../invoker": {
      invoke: async (command, args) => {
        if (command === "get_tasks") {
          if (!backend.manual) return backend.rows.map((task) => ({ ...task }));
          let resolve;
          const rows = new Promise((res) => { resolve = res; });
          backend.calls.push({ resolve });
          return rows;
        }
        assert.equal(command, "cancel");
        if (backend.cancel) throw backend.cancel;
        backend.cancelled = args.eventId;
      },
      listen: async (name, callback) => {
        listeners.set(name, callback);
        return () => listeners.delete(name);
      },
    },
    "../state-refresh": {
      createStateRefresh: (run) => ({ run }),
      watchStateChanges: (stateRefresh) => {
        watching = true;
        refresh = stateRefresh.run;
        void refresh();
        return () => { watching = false; };
      },
    },
  };
  const module = new vm.SourceTextModule(await transpile("stores/tasks.ts"), { context });
  await module.link((name) => {
    if (name === "../task-progress") return helpers;
    const exports = dependencies[name];
    return new vm.SyntheticModule(Object.keys(exports), function () {
      for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
    }, { context });
  });
  await module.evaluate();
  const store = module.namespace;
  let current = [];
  const unsubscribe = store.activeTasks.subscribe((tasks) => { current = tasks; });
  // Let every pending microtask, including chained snapshots, run.
  const settle = () => new Promise((resolve) => setImmediate(resolve));
  await settle();
  return {
    store, backend, listeners, unsubscribe, settle,
    get tasks() { return current; },
    get watching() { return watching; },
    async refresh() { await refresh(); await settle(); },
    startRefresh: () => refresh(),
  };
}

test("only pending and processing tasks are listed, newest first", async () => {
  const f = await fixture();
  f.backend.rows = [
    row("old", "processing", "2026-10-18T10:00:00+00:00"),
    row("done", "success", "2026-10-18T11:00:00+00:00"),
    row("new", "pending", "2026-10-18T12:00:00+00:00"),
  ];
  await f.refresh();
  assert.deepEqual(plain(f.tasks.map((task) => task.id)), ["new", "old"]);
  assert.deepEqual([...f.listeners.keys()].sort(), ["progress-update:new", "progress-update:old"]);
  f.unsubscribe();
  assert.equal(f.watching, false);
  await f.settle();
  assert.equal(f.listeners.size, 0);
});

test("an announced task shows before the backend lists it and hands over to the backend row", async () => {
  const f = await fixture();
  f.store.announceTask({ id: "clip-1", task_type: "clip_range", metadata: "{}" });
  await f.settle();
  assert.deepEqual(plain(f.tasks.map((task) => [task.id, task.status])), [["clip-1", "pending"]]);
  assert.ok(f.listeners.has("progress-update:clip-1"));

  f.listeners.get("progress-update:clip-1")({
    payload: { id: "clip-1", content: "压制弹幕 40%", progress: { stage: "压制弹幕", processed: 12, total: 30 } },
  });
  assert.equal(f.tasks[0].update.progress.processed, 12);

  f.backend.rows = [{ ...row("clip-1", "processing", "2026-10-18T12:00:00+00:00"), message: "压制弹幕 40%" }];
  await f.refresh();
  assert.equal(f.tasks.length, 1);
  assert.equal(f.tasks[0].status, "processing");
  // Progress received before the handover is kept.
  assert.equal(f.tasks[0].update.progress.processed, 12);

  f.backend.rows = [row("clip-1", "success", "2026-10-18T12:00:00+00:00")];
  await f.refresh();
  assert.deepEqual(plain(f.tasks), []);
  await f.settle();
  assert.equal(f.listeners.size, 0);
});

test("settling an announced task the backend never recorded removes it", async () => {
  const f = await fixture();
  f.store.announceTask({ id: "clip-1", task_type: "clip_range", metadata: "{}" });
  await f.settle();
  f.store.settleTask("clip-1");
  await f.settle();
  assert.deepEqual(plain(f.tasks), []);
  assert.equal(f.listeners.size, 0);
});

test("a queued task stays listed when its submission settles", async () => {
  const f = await fixture();
  f.store.announceTask({ id: "encode-1", task_type: "encode_video_subtitle", metadata: "{}" });
  await f.settle();
  // The backend recorded the task before the submission returned.
  f.backend.rows = [row("encode-1", "pending", "2026-10-18T12:00:00+00:00")];
  f.store.settleTask("encode-1");
  await f.settle();
  assert.deepEqual(plain(f.tasks.map((task) => [task.id, task.status])), [["encode-1", "pending"]]);
});

test("a snapshot taken before settling cannot drop the announcement", async () => {
  const f = await fixture();
  f.store.announceTask({ id: "encode-1", task_type: "encode_video_subtitle", metadata: "{}" });
  await f.settle();
  f.backend.manual = true;
  const early = f.startRefresh();
  f.store.settleTask("encode-1");
  await f.settle();
  assert.equal(f.backend.calls.length, 2);

  // Started before the backend recorded the task.
  f.backend.calls[0].resolve([]);
  await early;
  await f.settle();
  assert.deepEqual(plain(f.tasks.map((task) => task.id)), ["encode-1"]);

  f.backend.calls[1].resolve([row("encode-1", "processing", "2026-10-18T12:00:00+00:00")]);
  await f.settle();
  assert.deepEqual(plain(f.tasks.map((task) => [task.id, task.status])), [["encode-1", "processing"]]);
});

test("cancellation is remembered only when the cancel request succeeds", async () => {
  const f = await fixture();
  await f.store.cancelTask("clip-1");
  assert.equal(f.backend.cancelled, "clip-1");
  assert.equal(f.store.wasCancelled("clip-1"), true);
  f.store.settleTask("clip-1");
  assert.equal(f.store.wasCancelled("clip-1"), false);

  f.backend.cancel = new Error("任务尚未进入队列或不支持取消，请稍后重试");
  await assert.rejects(f.store.cancelTask("clip-2"), /稍后重试/);
  assert.equal(f.store.wasCancelled("clip-2"), false);
});

test("opening the task center can highlight a task and closing clears it", async () => {
  const f = await fixture();
  f.store.openTaskCenter("clip-1");
  assert.deepEqual(plain(svelteStore.get(f.store.taskCenter)), { open: true, highlight: "clip-1" });
  f.store.closeTaskCenter();
  assert.deepEqual(plain(svelteStore.get(f.store.taskCenter)), { open: false, highlight: null });
  f.store.openTaskCenter();
  assert.deepEqual(plain(svelteStore.get(f.store.taskCenter)), { open: true, highlight: null });
});
