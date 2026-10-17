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

async function fixture() {
  const alerts = [], failures = [], requests = [], cancellations = [];
  const listeners = new Map();
  let nextId = 0;
  const context = vm.createContext({
    $state: (value) => value,
    $derived: (value) => value,
    $bindable: (value) => value,
    $props: () => ({
      archive: { title: "Archive", room_id: "1", live_id: "2", platform: "bilibili" },
      ranges: [{ start: 0, end: 10 }],
      onFailed: (message) => failures.push(message),
    }),
    localStorage: { getItem: () => null },
    alert: (message) => alerts.push(message),
  });
  const dependencies = {
    svelte: { onMount() {}, onDestroy() {} },
    "lucide-svelte": { Loader2() {}, Scissors() {} },
    "../interface": {
      generateEventId: () => `clip-${++nextId}`,
      clipRange: (eventId) => {
        const result = deferred();
        requests.push({ eventId, ...result });
        return result.promise;
      },
    },
    "../invoker": {
      invoke: (command, { eventId }) => {
        assert.equal(command, "cancel");
        const result = deferred();
        cancellations.push({ eventId, ...result });
        return result.promise;
      },
      listen: async (name, listener) => {
        listeners.set(name, listener);
        return () => listeners.delete(name);
      },
      log: { warn() {} },
    },
  };
  // Run the component's actual async handlers with plain fixture state; no DOM
  // or reactive rendering is needed to reproduce the two RPC completion orders.
  const source = await readFile(new URL("../../src/lib/components/ArchiveClipButton.svelte", import.meta.url), "utf8");
  const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
  const output = ts.transpileModule(`${script}\nexport { generateClip, cancelClip };`, {
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
  return { handlers: module.namespace, alerts, failures, requests, cancellations, listeners };
}

async function startClip(f) {
  const count = f.requests.length;
  const completion = f.handlers.generateClip();
  // Allow both asynchronous event subscriptions to attach before clipRange.
  for (let i = 0; i < 10; i++) await Promise.resolve();
  assert.equal(f.requests.length, count + 1);
  return { completion, request: f.requests.at(-1) };
}

test("cancelled clip result can reject before cancel returns without showing a failure", async () => {
  const f = await fixture();
  const { completion, request } = await startClip(f);
  const cancellation = f.handlers.cancelClip();
  await f.handlers.cancelClip();
  assert.equal(f.cancellations.length, 1);
  assert.equal(f.cancellations[0].eventId, request.eventId);

  request.reject("切片任务失败");
  await completion;
  assert.deepEqual(f.alerts, []);
  assert.deepEqual(f.failures, []);
  assert.equal(f.listeners.size, 0);
  f.cancellations[0].resolve();
  await cancellation;

  const retry = await startClip(f);
  retry.request.reject("real encoding failure");
  await retry.completion;
  assert.deepEqual(f.alerts, ["切片生成失败：real encoding failure"]);
  assert.deepEqual(f.failures, ["real encoding failure"]);
});

test("failure progress arriving during cancellation does not show an FFmpeg error", async () => {
  const f = await fixture();
  const { completion, request } = await startClip(f);
  const cancellation = f.handlers.cancelClip();
  f.listeners.get(`progress-finished:${request.eventId}`)({
    payload: { success: false, message: "任务已取消" },
  });
  request.reject("切片任务失败");
  await completion;
  assert.deepEqual(f.alerts, []);
  assert.deepEqual(f.failures, []);
  f.cancellations[0].resolve();
  await cancellation;
});

test("real failure progress still alerts and reports the failure once", async () => {
  const f = await fixture();
  const { completion, request } = await startClip(f);
  f.listeners.get(`progress-finished:${request.eventId}`)({
    payload: { success: false, message: "切片失败: encoder unavailable" },
  });
  request.reject("encoder unavailable");
  await completion;
  assert.deepEqual(f.alerts, ["请检查 ffmpeg 是否配置正确：切片失败: encoder unavailable"]);
  assert.deepEqual(f.failures, ["切片失败: encoder unavailable"]);
  assert.equal(f.listeners.size, 0);
});
