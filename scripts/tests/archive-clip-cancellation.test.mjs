import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

function deferred() {
  let resolve, reject;
  const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

async function fixture({ variant = "primary" } = {}) {
  const alerts = [], failures = [], generated = [], requests = [];
  const announced = [], settled = [], opened = [];
  const cancelled = new Set();
  let nextId = 0;
  const context = vm.createContext({
    $state: (value) => value,
    $derived: (value) => value,
    $bindable: (value) => value,
    $props: () => ({
      archive: { title: "Archive", room_id: "1", live_id: "2", platform: "bilibili" },
      ranges: [{ start: 0, end: 10 }],
      variant,
      onGenerated: (video) => generated.push(video),
      onFailed: (message) => failures.push(message),
    }),
    localStorage: { getItem: () => null },
    alert: (message) => alerts.push(message),
  });
  const dependencies = {
    svelte: { onMount() {}, onDestroy() {} },
    "lucide-svelte": { Scissors() {} },
    "../interface": {
      generateEventId: () => `clip-${++nextId}`,
      generateMaterials: (eventId, params) => {
        const result = deferred();
        requests.push({ eventId, params, ...result });
        return result.promise;
      },
    },
    "../stores/tasks": {
      announceTask: (task) => announced.push(task),
      openTaskCenter: (highlight) => opened.push(highlight),
      settleTask: (id) => settled.push(id),
      wasCancelled: (id) => cancelled.has(id),
    },
  };
  // Run the component's actual async handler with plain fixture state; no DOM
  // or reactive rendering is needed to reproduce the RPC completion orders.
  const source = await readFile(new URL("../../src/lib/components/ArchiveClipButton.svelte", import.meta.url), "utf8");
  const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
  const output = ts.transpileModule(`${script}\nexport { generateClip };`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  }).outputText;
  const module = new vm.SourceTextModule(output, { context });
  const linked = new Map();
  await module.link((name) => {
    const exports = dependencies[name];
    if (!linked.has(name)) {
      linked.set(name, new vm.SyntheticModule(Object.keys(exports), function () {
        for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
      }, { context }));
    }
    return linked.get(name);
  });
  await module.evaluate();
  return {
    handlers: module.namespace, alerts, failures, generated, requests,
    announced, settled, opened, cancelled,
  };
}

async function startClip(f) {
  const count = f.requests.length;
  const completion = f.handlers.generateClip();
  await Promise.resolve();
  assert.equal(f.requests.length, count + 1);
  return { completion, request: f.requests.at(-1) };
}

test("a started clip is announced, highlighted, and settled with its result", async () => {
  const f = await fixture();
  const { completion, request } = await startClip(f);
  assert.equal(f.announced.length, 1);
  assert.equal(f.announced[0].id, request.eventId);
  assert.equal(f.announced[0].task_type, "generate_materials");
  // The cover is a data URL the task list never needs.
  assert.equal(JSON.parse(f.announced[0].metadata).params.cover, "");
  assert.deepEqual(f.opened, [request.eventId]);
  assert.deepEqual(f.settled, []);

  request.resolve({ id: 1 });
  await completion;
  assert.deepEqual(f.generated, [{ id: 1 }]);
  assert.deepEqual(f.settled, [request.eventId]);
  assert.deepEqual(f.alerts, []);
});

test("clips can be generated while another is still running", async () => {
  const f = await fixture();
  const first = await startClip(f);
  const second = await startClip(f);
  assert.notEqual(first.request.eventId, second.request.eventId);
  assert.deepEqual(f.opened, [first.request.eventId, second.request.eventId]);

  second.request.resolve({ id: 2 });
  await second.completion;
  first.request.resolve({ id: 1 });
  await first.completion;
  assert.deepEqual(f.generated, [{ id: 2 }, { id: 1 }]);
  assert.deepEqual(f.settled, [second.request.eventId, first.request.eventId]);
});

test("a clip cancelled from the task center fails silently whatever the backend reports", async () => {
  const f = await fixture();
  const { completion, request } = await startClip(f);
  f.cancelled.add(request.eventId);
  request.reject("生成素材任务失败");
  await completion;
  assert.deepEqual(f.alerts, []);
  assert.deepEqual(f.failures, []);
  assert.deepEqual(f.settled, [request.eventId]);
});

for (const variant of ["primary", "compact"]) {
  test(`task-page cancellation of a ${variant} clip is silent`, async () => {
    // Raw backend rejection and the desktop/web invoke wrappers carry the same
    // terminal cancellation message, with different Error prefixes.
    for (const error of [
      "生成素材任务已取消",
      new Error("Failed to invoke generate_materials:\n生成素材任务已取消"),
      new Error("Failed to invoke generate_materials:\nError: 生成素材任务已取消"),
    ]) {
      const f = await fixture({ variant });
      const { completion, request } = await startClip(f);
      request.reject(error);
      await completion;
      assert.deepEqual(f.alerts, []);
      assert.deepEqual(f.failures, []);
      assert.deepEqual(f.settled, [request.eventId]);
    }
  });
}

test("a real failure alerts and reports once", async () => {
  const f = await fixture();
  const { completion, request } = await startClip(f);
  request.reject("encoder unavailable");
  await completion;
  assert.deepEqual(f.alerts, ["素材生成失败：encoder unavailable"]);
  assert.deepEqual(f.failures, ["encoder unavailable"]);
  assert.deepEqual(f.settled, [request.eventId]);
});
