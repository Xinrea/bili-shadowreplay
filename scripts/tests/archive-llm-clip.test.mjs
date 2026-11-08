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

// One macrotask tick drains every pending microtask, so the run is parked on
// the next unresolved promise (a generateMaterials request) once this settles.
const tick = () => new Promise((resolve) => setImmediate(resolve));

function summary(overrides = {}) {
  return {
    status: "success",
    stage: "",
    highlights_json: JSON.stringify([
      { title: "开场", start_seconds: 10, end_seconds: 70 },
      { title: "连麦", start_seconds: 100, end_seconds: 160 },
      { title: "空段", start_seconds: 200, end_seconds: 200 },
    ]),
    error_message: null,
    ...overrides,
  };
}

async function fixture({ commands = {} } = {}) {
  const alerts = [], announced = [], opened = [], settled = [];
  const added = [], removed = [];
  const requests = [], calls = [];
  const cancelled = new Set();
  let nextId = 0;
  const context = vm.createContext({
    $state: (value) => value,
    $derived: (value) => value,
    $props: () => ({
      archive: { title: "Archive", room_id: "1", live_id: "2", platform: "bilibili", length: 3600 },
      menu: false,
      onaction: () => {},
    }),
    localStorage: { getItem: () => null },
    alert: (message) => alerts.push(message),
    // Poll delays resolve immediately so the bounded loop finishes instantly.
    setTimeout: (fn) => { fn(); return 0; },
    window: {
      addEventListener: (...args) => added.push(args),
      removeEventListener: (...args) => removed.push(args),
    },
  });
  const dependencies = {
    "svelte/reactivity": {
      SvelteMap: class {
        #map = new Map();
        get(key) { return this.#map.get(key); }
        set(key, value) { this.#map.set(key, value); }
      },
    },
    "../archives": { archiveKey: (a) => `${a.platform}:${a.room_id}:${a.live_id}` },
    "lucide-svelte": { AlertCircle() {}, CheckCircle2() {}, Loader2() {}, Scissors() {} },
    "../interface": {
      generateEventId: () => `llm-clip-${++nextId}`,
      generateMaterials: (eventId, params) => {
        const result = deferred();
        requests.push({ eventId, params, ...result });
        return result.promise;
      },
    },
    "../invoker": {
      invoke: (command, args) => {
        calls.push({ command, args });
        const handler = commands[command];
        if (!handler) return Promise.reject(new Error(`unexpected command: ${command}`));
        try {
          return Promise.resolve(handler(args));
        } catch (error) {
          return Promise.reject(error);
        }
      },
    },
    "../stores/tasks": {
      announceTask: (task) => announced.push(task),
      openTaskCenter: (highlight) => opened.push(highlight),
      settleTask: (id) => settled.push(id),
      wasCancelled: (id) => cancelled.has(id),
    },
  };
  // The flow lives in a <script module> block (shared state and unload guard)
  // plus the instance block; concatenate both so they share one scope.
  const source = await readFile(new URL("../../src/lib/components/ArchiveLlmClipButton.svelte", import.meta.url), "utf8");
  const moduleScript = source.match(/<script module lang="ts">([\s\S]*?)<\/script>/)[1];
  const instanceScript = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
  const output = ts.transpileModule(`${moduleScript}
${instanceScript}
    export { generateLlmClips, llmClipStates as states, activeLlmClipRuns as activeRuns };
  `, {
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
    handlers: module.namespace, alerts, announced, opened, settled, requests, calls,
    cancelled, added, removed,
    // Registered when a run starts; the same module-level handler is reused.
    unloadHandler: () => added.at(-1)?.[1],
    commandsCalled: (command) => calls.filter((call) => call.command === command),
    state: () => module.namespace.states.get("bilibili:1:2"),
  };
}

function queueSummaries(steps) {
  // Consume in order, then hold the last entry so polls keep seeing it.
  return () => {
    if (steps.length > 1) return steps.shift();
    const last = steps[0];
    return last === undefined ? null : last;
  };
}

// Clips run sequentially: settle the parked request, tick until the next one
// appears, and repeat until the run's completion settles.
async function settleRequests(f, outcome) {
  let handled = 0;
  while (handled < f.requests.length) {
    outcome(f.requests[handled]);
    handled += 1;
    await tick();
  }
}

test("reuses a successful summary and clips each highlight with danmu", async () => {
  const f = await fixture({ commands: { get_archive_summary: () => summary() } });
  const completion = f.handlers.generateLlmClips();
  await tick();
  assert.equal(f.requests.length, 1);
  assert.deepEqual(f.commandsCalled("generate_archive_summary"), []);
  assert.deepEqual(f.commandsCalled("get_llm_config"), []);

  const first = f.requests[0];
  assert.equal(first.params.note, "开场");
  // Highlights are parsed inside the sandbox realm; round-trip through JSON
  // so the structural comparison does not trip on cross-realm prototypes.
  assert.deepEqual(JSON.parse(JSON.stringify(first.params.ranges)), [{ start: 10, end: 70 }]);
  assert.equal(first.params.danmu, true);
  assert.equal(first.params.fix_encoding, false);
  assert.equal(first.params.local_offset, 0);
  assert.equal(first.params.cover, "");
  assert.equal(JSON.parse(f.announced[0].metadata).create_project, false);
  assert.equal(JSON.parse(f.announced[0].metadata).merge, false);
  // Only the first clip pulls focus into the task center.
  assert.deepEqual(f.opened, [first.eventId]);

  first.resolve({ id: 1 });
  await tick();
  const second = f.requests[1];
  assert.equal(second.params.note, "连麦");
  assert.deepEqual(JSON.parse(JSON.stringify(second.params.ranges)), [{ start: 100, end: 160 }]);
  assert.deepEqual(f.opened, [first.eventId]);
  second.resolve({ id: 2 });
  await completion;

  assert.equal(f.state().phase, "done");
  assert.equal(f.state().message, "已生成 2/2 个切片");
  assert.deepEqual(f.alerts, []);
  assert.deepEqual(f.settled, [first.eventId, second.eventId]);
  // The unload guard is released once the run finishes.
  assert.equal(f.added.length, 1);
  assert.equal(f.removed.length, 1);
});

test("submits a new summary when none exists and polls until it succeeds", async () => {
  const f = await fixture({
    commands: {
      get_archive_summary: queueSummaries([
        null,
        { status: "processing", stage: "transcribing" },
        { status: "processing", stage: "summarizing" },
        summary(),
      ]),
      get_llm_config: () => ({ provider: "openai", endpoint: "e", api_key: "k", model: "m" }),
      generate_archive_summary: () => ({ id: "task-1" }),
    },
  });
  const completion = f.handlers.generateLlmClips();
  await tick();
  const submits = f.commandsCalled("generate_archive_summary");
  assert.equal(submits.length, 1);
  assert.equal(submits[0].args.force, false);
  assert.equal(f.requests.length, 1);
  await settleRequests(f, (request) => request.resolve({ id: 1 }));
  await completion;
  assert.equal(f.state().phase, "done");
  assert.equal(f.state().message, "已生成 2/2 个切片");
  assert.deepEqual(f.alerts, []);
});

test("reuses an in-flight summary without resubmitting", async () => {
  const f = await fixture({
    commands: {
      get_archive_summary: queueSummaries([
        { status: "processing", stage: "extracting_audio" },
        summary(),
      ]),
    },
  });
  const completion = f.handlers.generateLlmClips();
  await tick();
  assert.deepEqual(f.commandsCalled("generate_archive_summary"), []);
  assert.deepEqual(f.commandsCalled("get_llm_config"), []);
  await settleRequests(f, (request) => request.resolve({ id: 1 }));
  await completion;
  assert.equal(f.state().phase, "done");
  assert.equal(f.state().message, "已生成 2/2 个切片");
});

test("re-reads instead of failing when the resubmit races a completed summary", async () => {
  const f = await fixture({
    commands: {
      get_archive_summary: queueSummaries([null, summary()]),
      get_llm_config: () => ({ provider: "ollama", endpoint: "e", api_key: "", model: "m" }),
      generate_archive_summary: () => Promise.reject("该录播已经生成 Summary"),
    },
  });
  const completion = f.handlers.generateLlmClips();
  await tick();
  assert.equal(f.commandsCalled("generate_archive_summary").length, 1);
  await settleRequests(f, (request) => request.resolve({ id: 1 }));
  await completion;
  assert.equal(f.state().phase, "done");
  assert.equal(f.state().message, "已生成 2/2 个切片");
  assert.deepEqual(f.alerts, []);
});

test("surfaces the original submit error when no summary ever appears", async () => {
  const f = await fixture({
    commands: {
      get_archive_summary: queueSummaries([null, null]),
      get_llm_config: () => ({ provider: "ollama", endpoint: "e", api_key: "", model: "m" }),
      generate_archive_summary: () => Promise.reject("生成 Summary 失败：数据库不可用"),
    },
  });
  await f.handlers.generateLlmClips();
  assert.equal(f.state().phase, "failed");
  assert.equal(f.state().message, "生成 Summary 失败：数据库不可用");
  assert.deepEqual(f.alerts, ["LLM 切片生成失败：生成 Summary 失败：数据库不可用"]);
  assert.deepEqual(f.requests, []);
  assert.equal(f.removed.length, 1);
});

test("accounts cancellations separately from failures", async () => {
  const f = await fixture({
    commands: {
      get_archive_summary: () => summary({
        highlights_json: JSON.stringify([
          { title: "开场", start_seconds: 10, end_seconds: 70 },
          { title: "连麦", start_seconds: 100, end_seconds: 160 },
          { title: "合唱", start_seconds: 200, end_seconds: 260 },
        ]),
      }),
    },
  });
  const completion = f.handlers.generateLlmClips();
  await tick();
  const first = f.requests[0];
  first.resolve({ id: 1 });
  await tick();
  const second = f.requests[1];
  // Cancelled from another entry such as the task page: the invoke wrapper
  // prefixes the backend error, so only the tail of the message matches.
  second.reject(new Error("Failed to invoke generate_materials:\n生成素材任务已取消"));
  await tick();
  f.requests[2].reject("encoder unavailable");
  await completion;
  assert.equal(f.state().phase, "done");
  assert.equal(f.state().message, "已生成 1/3 个切片，1 个失败：合唱：encoder unavailable，1 个已取消");
  assert.deepEqual(f.alerts, ["部分切片生成失败：\n合唱：encoder unavailable"]);
  assert.deepEqual(f.settled, [first.eventId, second.eventId, f.requests[2].eventId]);
});

test("a fully cancelled run is reported as done, not failed", async () => {
  const f = await fixture({ commands: { get_archive_summary: () => summary() } });
  const completion = f.handlers.generateLlmClips();
  await tick();
  const first = f.requests[0];
  f.cancelled.add(first.eventId);
  first.reject("生成素材任务失败");
  await tick();
  f.requests[1].reject("生成素材任务已取消");
  await completion;
  assert.equal(f.state().phase, "done");
  assert.equal(f.state().message, "已取消 2/2 个切片的生成");
  assert.deepEqual(f.alerts, []);
});

test("gives up after the polling bound when a summary never settles", async () => {
  const f = await fixture({
    commands: { get_archive_summary: () => summary({ status: "processing", stage: "transcribing" }) },
  });
  await f.handlers.generateLlmClips();
  const reads = f.commandsCalled("get_archive_summary");
  // Initial read plus one poll per attempt: 1800 attempts at 2s = the 60
  // minutes backstop; the backend already fails summaries orphaned by restarts.
  assert.equal(reads.length, 1801);
  assert.equal(f.state().phase, "failed");
  assert.match(f.state().message, /Summary 生成超时/);
  assert.deepEqual(f.requests, []);
});

test("validates the LLM config before submitting a new summary", async () => {
  const f = await fixture({
    commands: {
      get_archive_summary: queueSummaries([null]),
      get_llm_config: () => ({ provider: "openai", endpoint: "e", api_key: "k", model: " " }),
    },
  });
  await f.handlers.generateLlmClips();
  assert.deepEqual(f.commandsCalled("generate_archive_summary"), []);
  assert.equal(f.state().phase, "failed");
  // Frontend-thrown errors carry the Error prefix; backend rejections are strings.
  assert.equal(f.state().message, "Error: 请先在设置中配置 Summary 使用的 LLM 模型");
});

test("warns before unloading only while a run is active", async () => {
  const f = await fixture({ commands: { get_archive_summary: () => summary() } });
  const completion = f.handlers.generateLlmClips();
  await tick();
  assert.equal(f.added.length, 1);
  assert.equal(f.removed.length, 0);
  const activeEvent = { preventDefault() {}, returnValue: undefined };
  f.unloadHandler()(activeEvent);
  assert.match(activeEvent.returnValue, /剩余切片将不会生成/);

  await settleRequests(f, (request) => request.resolve({ id: 1 }));
  await completion;
  assert.equal(f.removed.length, 1);
  const idleEvent = { preventDefault() {}, returnValue: undefined };
  f.unloadHandler()(idleEvent);
  assert.equal(idleEvent.returnValue, undefined);
});
