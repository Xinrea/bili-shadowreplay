import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { compile } from "svelte/compiler";
import { render } from "svelte/server";
import { recordingStorage } from "./recording-storage-fixture.mjs";

const read = path => readFile(new URL(`../../src/${path}`, import.meta.url), "utf8");
const transpile = source => ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText;
const plain = value => JSON.parse(JSON.stringify(value));
const context = vm.createContext({});
const linked = new Map();
// Icons are decorative; their package expects Vite's extension resolution.
const icons = Object.fromEntries(["ArrowRight", "ArrowRightLeft", "Check", "CircleAlert",
  "FolderOpen", "HardDrive", "Loader2", "Plus", "Trash2", "X"].map(name => [name, () => {}]));
async function dependency(name, dependencies) {
  return synthetic(name, name === "lucide-svelte" ? icons
    : name === "../recording-storage" ? recordingStorage : dependencies[name] ?? await import(name));
}
function synthetic(name, exports) {
  if (!linked.has(name)) linked.set(name, new vm.SyntheticModule(Object.keys(exports), function () {
    for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
  }, { context }));
  return linked.get(name);
}
const helpers = new vm.SourceTextModule(transpile(await read("lib/storage-transfer.ts")), { context });
await helpers.link(name => synthetic(name, recordingStorage));
await helpers.evaluate();
const progress = new vm.SourceTextModule(transpile(await read("lib/task-progress.ts")), { context });
await progress.link(() => { throw new Error("Unexpected dependency"); });
await progress.evaluate();
const source = await read("lib/components/StorageTransferSettings.svelte");
const parentSource = await read("lib/components/RecordingStorageSettings.svelte");
const config = () => ({
  default_backend: "A",
  backends: { A: { type: "local", root: "/A" }, B: { type: "local", root: "/B" } },
});
const s3 = prefix => ({
  type: "s3", root: "", s3: {
    endpoint: "http://127.0.0.1:9000", bucket: "recordings",
    region: "us-east-1", prefix, path_style: true,
  },
});
const plan = (extra = {}) => ({
  source_backend_id: "A", target_backend_id: "B", recording_count: 3,
  total_bytes: 1024, required_bytes: 1024, available_bytes: 4096,
  cleanup_count: 0, can_transfer: true, blockers: [], transfer_mode: "copy", ...extra,
});
const task = (extra = {}) => ({
  id: "transfer-1", task_type: "storage_transfer", status: "pending", message: "等待转移",
  created_at: "2026-01-01T00:00:00Z",
  metadata: JSON.stringify({ source_backend_id: "A", target_backend_id: "B" }), ...extra,
});
function handlers(source, state) {
  const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
  const parsed = ts.createSourceFile("component.ts", script, ts.ScriptTarget.Latest, true);
  const context = vm.createContext(state);
  vm.runInContext(transpile(parsed.statements.filter(ts.isFunctionDeclaration)
    .map(node => node.getText(parsed)).join("\n")), context);
  // Evaluate the component's actual derivations, not a second implementation of
  // its safety gates (which could silently miss a newly added condition).
  for (const statement of parsed.statements.filter(ts.isVariableStatement)) {
    for (const declaration of statement.declarationList.declarations) {
      const init = declaration.initializer;
      if (init && ts.isCallExpression(init) && init.expression.getText(parsed) === "$derived") {
        Object.defineProperty(state, declaration.name.getText(parsed), {
          get: () => vm.runInContext(transpile(init.arguments[0].getText(parsed)), context),
        });
      }
    }
  }
  return state;
}
function fixture() {
  const requests = [];
  const state = handlers(source, {
    ...helpers.namespace, ...progress.namespace,
    storage: config(), sourceId: "A", targetId: "B", disabled: false, busy: false,
    starting: false, scanning: false, plan: null, planKey: "", confirmed: false,
    error: "", tasksError: "", tasksKnown: true, tasks: [],
    disposed: false, scanRequest: 0, taskRevision: 0, scheduled: 0,
    tick: async () => {},
    invoke: async (command, args) => {
      requests.push({ command, args: args && plain(args) });
      if (command === "preview_storage_transfer") return plan();
      if (command === "start_storage_transfer") return task();
      if (command === "get_tasks") return [];
      throw new Error(command);
    },
  });
  state.taskRefresh = { schedule() { state.scheduled++; } };
  return { state, requests };
}

test("all S3 and local directions can be selected without weakening missing-backend validation", () => {
  const storage = config();
  storage.backends.C = s3("one");
  storage.backends.D = s3("two");
  for (const [source, target] of [["A", "C"], ["C", "A"], ["C", "D"], ["D", "C"]]) {
    assert.equal(helpers.namespace.transferSelectionError(storage, source, target), "");
  }
  assert.notEqual(helpers.namespace.transferSelectionError(storage, "C", "missing"), "");
  assert.notEqual(helpers.namespace.transferSelectionError(storage, "C", "C"), "");
});

test("selection invalidation does not subscribe to its own request counter", () => {
  const effect = source.match(/\$effect\(\(\) => \{([\s\S]*?)\}\);/)[1];
  assert.match(effect, /selectionKey;/);
  assert.match(effect, /untrack\(invalidatePlan\)/);
});

test("storage transfer validates same, empty, missing and unsupported selections locally", async () => {
  const { state, requests } = fixture();
  for (const [source, target] of [["A", "A"], ["", "B"], ["A", ""], ["missing", "B"], ["constructor", "B"], ["A", "cloud"]]) {
    state.storage.backends.cloud = { type: "s3", root: "bucket" };
    state.sourceId = source;
    state.targetId = target;
    await state.scan();
    assert.ok(state.error);
    state.confirmed = true;
    await state.start();
  }
  assert.equal(requests.length, 0);
});

test("insufficient space, busy recordings and overlapping roots block confirmation", async () => {
  for (const blocker of ["目标空间不足", "录播正在写入或使用中", "来源与目标目录重叠"]) {
    const { state } = fixture();
    state.invoke = async command => {
      assert.equal(command, "preview_storage_transfer");
      return plan({ can_transfer: false, blockers: [blocker], available_bytes: 1 });
    };
    await state.scan();
    assert.equal(state.plan.blockers[0], blocker);
    state.confirmed = true;
    assert.equal(state.canStart, false);
    await state.start();
  }
  const { state } = fixture();
  state.invoke = async () => plan({ blockers: ["blocked"], can_transfer: true });
  await state.scan();
  state.confirmed = true;
  assert.equal(state.canStart, false, "blockers also fail closed if the flag is inconsistent");
});

test("scans reject stale results, stale errors, mismatched pairs and duplicate requests", async () => {
  const { state } = fixture();
  const pending = [];
  state.invoke = () => new Promise((resolve, reject) => pending.push({ resolve, reject }));
  const first = state.scan();
  await state.scan();
  assert.equal(pending.length, 1);
  state.targetId = "C";
  state.storage.backends.C = { type: "local", root: "/C" };
  state.invalidatePlan(); // selector/config effect
  const second = state.scan();
  pending[0].resolve(plan());
  await first;
  assert.equal(state.plan, null);
  assert.equal(state.scanning, true);
  pending[1].resolve(plan({ target_backend_id: "C" }));
  await second;
  assert.equal(state.plan.target_backend_id, "C");
  const third = state.scan();
  state.invalidatePlan();
  state.targetId = "B";
  pending[2].reject(new Error("stale error"));
  await third;
  assert.equal(state.error, "");
  state.invoke = async () => plan({ source_backend_id: "wrong" });
  await state.scan();
  assert.equal(state.plan, null);
  assert.match(state.error, /不一致/);
});

test("selector or config changes invalidate a confirmed plan, including change-away-and-back", async () => {
  for (const change of [
    state => { state.targetId = "A"; },
    state => { state.storage.backends.B.root = "/new"; },
    state => { state.storage.default_backend = "B"; },
    state => { state.disabled = true; },
  ]) {
    const { state, requests } = fixture();
    await state.scan();
    state.confirmed = true;
    change(state);
    await state.start(); // key guard works even before the Svelte effect flushes
    assert.equal(requests.length, 1);
    state.invalidatePlan();
    assert.equal(state.confirmed, false);
    assert.equal(state.plan, null);
  }
  const { state } = fixture();
  let resolve;
  state.invoke = () => new Promise(done => { resolve = done; });
  const pending = state.scan();
  state.targetId = "A";
  state.invalidatePlan();
  state.targetId = "B";
  state.invalidatePlan();
  resolve(plan());
  await pending;
  assert.equal(state.plan, null);
});

test("a matching plan requires explicit confirmation and queues only once", async () => {
  const { state, requests } = fixture();
  await state.scan();
  await state.start();
  assert.equal(requests.length, 1);
  state.confirmed = true;
  let resolve;
  state.invoke = (command, args) => {
    requests.push({ command, args: plain(args) });
    return new Promise(done => { resolve = done; });
  };
  const pending = state.start();
  assert.equal(state.starting, true);
  assert.equal(state.busy, true);
  assert.equal(state.confirmed, false);
  await state.start();
  await state.scan();
  assert.deepEqual(requests[1], {
    command: "start_storage_transfer", args: { sourceBackendId: "A", targetBackendId: "B" },
  });
  assert.equal(requests.length, 2);
  resolve(task());
  await pending;
  assert.equal(state.starting, false);
  assert.equal(state.busy, true);
  assert.equal(state.tasks[0].id, "transfer-1");
  assert.equal(state.scheduled, 1);
});

test("empty plans cannot start even if confirmed, but cleanup-only plans can", async () => {
  for (const cleanupCount of [0, 2]) {
    const { state, requests } = fixture();
    state.invoke = async (command, args) => {
      requests.push({ command, args: plain(args) });
      return command === "preview_storage_transfer"
        ? plan({ recording_count: 0, total_bytes: 0, required_bytes: 0, cleanup_count: cleanupCount })
        : task();
    };
    await state.scan();
    state.confirmed = true;
    assert.equal(state.canStart, cleanupCount > 0);
    await state.start();
    assert.deepEqual(requests.map(row => row.command), cleanupCount
      ? ["preview_storage_transfer", "start_storage_transfer"] : ["preview_storage_transfer"]);
  }
});

test("retry preselects the failed task pair, waits for tick and requires a new confirmation", async () => {
  const { state, requests } = fixture();
  await state.scan();
  state.confirmed = true;
  let flush;
  state.tick = () => new Promise(resolve => { flush = resolve; });
  state.invoke = async (command, args) => {
    requests.push({ command, args: plain(args) });
    assert.equal(command, "preview_storage_transfer", "retry never queues work automatically");
    return plan({ source_backend_id: "B", target_backend_id: "A" });
  };
  const retry = state.retryTask(task({
    status: "failed", metadata: JSON.stringify({ source_backend_id: "B", target_backend_id: "A" }),
  }));
  assert.equal(state.sourceId, "B");
  assert.equal(state.targetId, "A");
  assert.equal(state.plan, null);
  assert.equal(state.confirmed, false);
  assert.equal(requests.length, 1, "do not scan until selector effects flush");
  state.invalidatePlan(); // selection effect runs during the awaited Svelte tick
  flush();
  await retry;
  assert.deepEqual(requests[1], {
    command: "preview_storage_transfer", args: { sourceBackendId: "B", targetBackendId: "A" },
  });
  assert.equal(state.currentPlan.source_backend_id, "B");
  assert.equal(state.currentPlan.target_backend_id, "A");
  assert.equal(state.confirmed, false);
  assert.equal(state.canStart, false);
  await state.start();
  assert.equal(requests.length, 2);
});

test("retry rejects malformed or removed pairs and respects mutation locks", async () => {
  for (const metadata of ["bad json", "{}", '{"source_backend_id":1,"target_backend_id":"B"}',
    '{"source_backend_id":"missing","target_backend_id":"B"}']) {
    const { state, requests } = fixture();
    await state.retryTask(task({ status: "failed", metadata }));
    assert.ok(state.error);
    assert.deepEqual(requests, []);
  }
  for (const lock of ["disabled", "busy", "starting"]) {
    const { state, requests } = fixture();
    state[lock] = true;
    await state.retryTask(task({ metadata: '{"source_backend_id":"B","target_backend_id":"A"}' }));
    assert.equal(state.sourceId, "A");
    assert.equal(state.targetId, "B");
    assert.deepEqual(requests, []);
  }
});

test("readonly submission errors clear confirmation, require task recovery and allow a fresh scan", async () => {
  const { state } = fixture();
  await state.scan();
  state.confirmed = true;
  state.invoke = async () => { throw new Error("405 readonly mode"); };
  await state.start();
  assert.match(state.error, /405 readonly mode/);
  assert.equal(state.plan, null);
  assert.equal(state.confirmed, false);
  assert.equal(state.starting, false);
  assert.equal(state.busy, true);
  state.invoke = async () => [];
  await state.loadTasks();
  assert.equal(state.busy, false);
  await state.start();
  assert.equal(state.tasks.length, 0);
});

test("failed rechecks remove the old successful plan and retain backend errors", async () => {
  const { state } = fixture();
  await state.scan();
  state.confirmed = true;
  state.invoke = async () => { throw new Error("405 readonly mode"); };
  await state.scan();
  assert.equal(state.plan, null);
  assert.equal(state.confirmed, false);
  assert.equal(state.scanning, false);
  assert.match(state.error, /405 readonly mode/);
  await state.start();
});

test("reopening recovers active tasks; failures and cleanup results remain visible", async () => {
  const { state } = fixture();
  state.tasksKnown = false;
  state.busy = true;
  state.invoke = async command => {
    assert.equal(command, "get_tasks");
    return [task({ status: "processing", message: "正在清理来源 2/3" })];
  };
  await state.loadTasks();
  assert.equal(state.busy, true);
  assert.equal(state.tasks[0].message, "正在清理来源 2/3");
  const result = {
    transferred_recordings: 2, cleaned_recordings: 1, failed_recording: "bilibili/123/live",
    error: "来源清理失败",
  };
  state.invoke = async () => [task({
    status: "failed", metadata: JSON.stringify({ source_backend_id: "A", target_backend_id: "B", result }),
  })];
  await state.loadTasks();
  assert.equal(state.busy, false);
  assert.deepEqual(plain(state.transferResult(state.tasks[0])), result);
  assert.match(state.transferStatus(state.tasks[0].status), /重新|重试/);
  state.invoke = async () => plan({ recording_count: 1, cleanup_count: 1 });
  await state.scan();
  assert.equal(state.plan.cleanup_count, 1);
});

test("task recovery failures lock mutations and stale pre-queue snapshots cannot overwrite queued work", async () => {
  const { state } = fixture();
  state.invoke = async () => { throw new Error("offline"); };
  await state.loadTasks();
  assert.equal(state.busy, true);
  assert.match(state.tasksError, /offline/);
  state.invoke = async () => [];
  await state.loadTasks();
  assert.equal(state.busy, false);
  assert.equal(state.tasksError, "");
  state.invoke = async () => plan();
  await state.scan();
  state.confirmed = true;
  let resolve;
  state.invoke = async command => command === "get_tasks"
    ? new Promise(done => { resolve = done; }) : task();
  const old = state.loadTasks();
  await state.start();
  resolve([]);
  await old;
  assert.equal(state.busy, true);
  assert.equal(state.tasks[0].id, "transfer-1");
  state.invoke = async () => [];
  await state.loadTasks();
  assert.equal(state.busy, true, "retain an active local task omitted by an old snapshot");
});

test("late scan and task responses after disposal do not mutate state", async () => {
  for (const operation of ["scan", "loadTasks"]) {
    const { state } = fixture();
    let resolve;
    state.invoke = () => new Promise(done => { resolve = done; });
    const pending = state[operation]();
    state.disposed = true;
    resolve(operation === "scan" ? plan() : [task()]);
    await pending;
    assert.equal(state.plan, null);
    assert.equal(state.tasks.length, 0);
  }
  assert.match(source, /watchStateChanges\(taskRefresh, \["tasks-changed"\], 2000\)/);
  assert.match(source, /disposed = true;[\s\S]*?stop\(\);/);
});

test("parent locks CRUD during recovery/transfer but allows close after queue response", async () => {
  const calls = [];
  const state = handlers(parentSource, {
    storage: config(), loading: false, saving: false, browsing: false,
    transferBusy: true, transferStarting: false, removingId: "B",
    onclose: () => calls.push("closed"), invoke: () => calls.push("invoke"),
    pageVisible: () => true,
  });
  await state.save(true);
  await state.removeBackend("B");
  await state.browse();
  assert.deepEqual(calls, []);
  state.transferStarting = true;
  state.close();
  state.handleKeydown({ key: "Escape", preventDefault() {}, stopImmediatePropagation() {} });
  assert.deepEqual(calls, []);
  state.transferStarting = false;
  state.close();
  assert.deepEqual(calls, ["closed"]);
  assert.match(parentSource, /bind:busy=\{transferBusy\} bind:starting=\{transferStarting\}/);
  assert.match(parentSource, /<fieldset[^>]*disabled=\{saving \|\| browsing \|\| transferBusy\}/);
});

test("both settings views stay mounted with hidden/inert so background recovery is never gated by navigation", async () => {
  const dependencies = {
    "@tauri-apps/plugin-dialog": { open() {} },
    "../invoker": { invoke() {}, TAURI_ENV: false },
    "../recording-storage": recordingStorage,
    "../focus-trap": { trapFocus() {} },
    "../page-visibility": { pageVisibility: () => () => true },
    "../stores/tasks": { openTaskCenter() {} },
  };
  // Render the real transfer child, including its initial recovery state, rather
  // than a marker that could miss a parent/child prop or compilation regression.
  const transfer = new vm.SourceTextModule(compile(source, { generate: "server" }).js.code, { context });
  await transfer.link(async name => dependency(name, {
    "../invoker": { invoke() {}, TAURI_ENV: false },
    "../state-refresh": { createStateRefresh: () => ({}), watchStateChanges() {} },
    "../task-progress": progress.namespace,
    "../storage-transfer": helpers.namespace,
    "../stores/tasks": { openTaskCenter() {} },
  }));
  await transfer.evaluate();
  const credentialSource = await readFile(new URL("../../src/lib/components/S3CredentialFields.svelte", import.meta.url), "utf8");
  const credentialFields = new vm.SourceTextModule(compile(credentialSource, { generate: "server" }).js.code, { context });
  await credentialFields.link(async name => dependency(name, {}));
  await credentialFields.evaluate();
  for (const view of ["locations", "transfer"]) {
    const seeded = parentSource
      .replace("let storage: StorageConfig | null = $state(null);", `let storage: StorageConfig | null = $state(${JSON.stringify(config())});`)
      .replace("let loading = $state(true);", "let loading = $state(false);")
      .replace('("locations");', `("${view}");`);
    const component = new vm.SourceTextModule(compile(seeded, { generate: "server" }).js.code, { context });
    await component.link(async name => name === "./StorageTransferSettings.svelte" ? transfer
      : name === "./S3CredentialFields.svelte" ? credentialFields
      : dependency(name, dependencies));
    await component.evaluate();
    const body = render(component.namespace.default, { props: { onclose() {} } }).body;
    assert.match(body, /aria-label="存储设置视图"/);
    assert.match(body, /存储位置/);
    assert.match(body, /录播转移/);
    assert.match(body, /正在检查转移任务/, "recovery child exists even in locations view");
    assert.match(body, /\/A/);
    assert.match(body, /class="ui-badge tabular-nums" aria-label="2 个存储位置">2<\/span>/);
    assert.match(body, /role="group" aria-label="B 的存储操作"[^>]*>[\s\S]*?设为默认[\s\S]*?移除<\/button><\/div>/);
    assert.doesNotMatch(body, /id="new-storage-id"/, "add editor is absent until requested");
    if (view === "locations") {
      assert.match(body, /<div hidden="" inert=""><section[^>]*aria-labelledby="storage-transfer-title"/);
    } else {
      assert.match(body, /<div hidden="" inert=""><div[^>]*>[\s\S]*?\/A/);
    }
  }
  assert.match(parentSource, /hidden=\{view !== "locations"\} inert=\{view !== "locations"\}/);
  assert.match(parentSource, /hidden=\{view !== "transfer"\} inert=\{view !== "transfer"\}/);
  assert.doesNotMatch(parentSource, /\{#if view/);
  assert.match(parentSource, /onclick=\{\(\) => setDefault\(id\)\}/);
  assert.doesNotMatch(parentSource, /openTransfer|转移录播/);
  assert.match(parentSource, /\{#if removingId === id\}[\s\S]*?aria-label=\{`确认移除存储 \$\{id\}`\}/);
  assert.match(parentSource, /对应录播将从列表隐藏/);
  assert.match(parentSource, /存储上的文件和目录数据库不会删除/);
  assert.match(parentSource, /存储名称可以复用/);
  assert.doesNotMatch(parentSource, /此 ID 无法重新添加/);
  assert.match(parentSource, /type="button"[^>]*onclick=\{\(\) => \{ showAdd = false; clearCredentials\(\); \}\}>取消/);
  assert.match(source, /onMount\(\(\) => \{\s*const stop = watchStateChanges/);
});

test("transfer tasks have a Chinese name and A → B description, and cannot be cancelled", () => {
  assert.equal(progress.namespace.taskTypeName("storage_transfer"), "录播存储转移");
  assert.equal(progress.namespace.describeTask(task()), "A → B");
  assert.equal(progress.namespace.isCancellableTask("storage_transfer"), false);
  assert.equal(helpers.namespace.transferResult(task({ metadata: "bad json" })), null);
  assert.equal(helpers.namespace.transferBytes(1024), "1.0 KiB");
  assert.equal(helpers.namespace.transferModeLabel("copy"), "复制并校验后清理来源");
  assert.equal(helpers.namespace.transferModeLabel("rename"), "同文件系统，优先快速移动");
  assert.equal(helpers.namespace.transferModeLabel("mixed"), "快速移动与复制");
});

async function showTransfer({ preview = null, rows = [], known = true, confirmed = false, sourceId = "A", storage = config() } = {}) {
  const dependencies = {
    "../invoker": { invoke() {}, TAURI_ENV: false },
    "../state-refresh": { createStateRefresh: () => ({}), watchStateChanges() {} },
    "../task-progress": progress.namespace,
    "../storage-transfer": helpers.namespace,
    "../stores/tasks": { openTaskCenter() {} },
  };
  // Seed state without replacing bindable props. Fail loudly if declarations move,
  // rather than accidentally testing the unseeded recovery screen.
  let seeded = source;
  for (const [before, after] of [
    ['let targetId = $state("");', 'let targetId = $state("B");'],
    ["let plan = $state<StorageTransferPlan | null>(null);", `let plan = $state<StorageTransferPlan | null>(${JSON.stringify(preview)});`],
    ['let planKey = $state("");', 'let planKey = $state(JSON.stringify([sourceId, "B", storage, false]));'],
    ["let tasks = $state<TaskRow[]>([]);", `let tasks = $state<TaskRow[]>(${JSON.stringify(rows)});`],
    ["let tasksKnown = $state(false);", `let tasksKnown = $state(${known});`],
    ["let confirmed = $state(false);", `let confirmed = $state(${confirmed});`],
  ]) {
    assert.ok(seeded.includes(before), `Missing SSR seed declaration: ${before}`);
    seeded = seeded.replace(before, after);
  }
  const component = new vm.SourceTextModule(compile(seeded, { generate: "server" }).js.code, { context });
  await component.link(async name => dependency(name, dependencies));
  await component.evaluate();
  return render(component.namespace.default, {
    props: { storage, sourceId, busy: !known || rows.some(row => progress.namespace.isActiveTaskStatus(row.status)) },
  }).body;
}

test("transfer form shows three steps, bound source prefill and ordered cleanup guidance after recovery", async () => {
  const recovering = await showTransfer({ known: false });
  assert.match(recovering, /正在检查转移任务/);
  assert.doesNotMatch(recovering, /id="transfer-source"/);
  const body = await showTransfer();
  assert.match(body, /逐条完整复制、校验并切换录播绑定后，才清理来源文件/);
  assert.match(body, /从哪里转出/);
  assert.match(body, /默认存储和直播间偏好不变/);
  assert.match(body, /选择位置/);
  assert.match(body, /检查空间/);
  assert.match(body, /确认转移/);
  assert.match(body, /<li[^>]*aria-current="step"[^>]*>[\s\S]*?选择位置/);
  assert.match(body, /<select[^>]*id="transfer-source"[\s\S]*?<option value="A"[^>]*selected/);
  const prefilled = await showTransfer({ sourceId: "B" });
  assert.match(prefilled, /<select[^>]*id="transfer-source"[\s\S]*?<option value="B"[^>]*selected/);
  assert.match(source, /sourceId = \$bindable\(""\)/);
  assert.doesNotMatch(source, /let sourceId = \$state/);
});

test("remote capacity stays unknown rather than claiming zero or sufficient space", async () => {
  const storage = config();
  storage.backends.B = s3("archive");
  const body = await showTransfer({
    storage, preview: plan({ transfer_mode: "object", available_bytes: null }), confirmed: true,
  });
  assert.match(body, /s3:\/\/recordings\/archive/);
  assert.match(body, /逐对象复制并校验后清理来源/);
  assert.match(body, /未知（S3 不提供容量查询）/);
  assert.match(body, /系统临时目录需容纳单个最大对象/);
  assert.match(body, /5 GiB/);
  assert.doesNotMatch(body, /预计空间足够|预计空间不足；|仅支持本地存储/);
  assert.match(body, /<button(?=[^>]*>开始转移)(?![^>]*disabled)[^>]*>/);
});

test("rendered plans expose space and blockers, and task failures expose partial success and retry guidance", async () => {
  const successful = await showTransfer({ preview: plan({ cleanup_count: 2 }) });
  assert.match(successful, /待转移录播（估算）<\/dt><dd[^>]*>3 条 <span[^>]*>\/ 1.0 KiB/);
  assert.match(successful, /待补清理<\/dt><dd[^>]*>2 条/);
  assert.match(successful, /预计所需空间<\/dt><dd[^>]*>1.0 KiB/);
  assert.match(successful, /按数据库记录估算大小，不扫描录播文件/);
  assert.match(successful, /估算可能不包含全部附属文件/);
  assert.match(successful, /目标可用空间<\/dt><dd[^>]*>4.0 KiB/);
  assert.match(successful, /确认转移 A 中的录播到 B/);
  assert.match(successful, /开始后不可取消/);
  assert.match(successful, /<button[^>]*disabled[^>]*>开始转移/, "confirmation starts unchecked");
  assert.doesNotMatch(successful, /type="checkbox"[^>]*checked/);
  const confirmed = await showTransfer({ preview: plan(), confirmed: true });
  assert.match(confirmed, /<button(?=[^>]*>开始转移)(?![^>]*disabled)[^>]*>/);
  const blocked = await showTransfer({ preview: plan({ can_transfer: false, blockers: ["目标空间不足", "录播正在写入"] }) });
  assert.match(blocked, /目标空间不足/);
  assert.match(blocked, /录播正在写入/);
  assert.doesNotMatch(blocked, /开始转移|type="checkbox"/);
  const recovery = await showTransfer({ preview: plan({ available_bytes: 0 }), confirmed: true });
  assert.match(recovery, /预计空间不足；重试会复用已完成的复制/);
  assert.doesNotMatch(recovery, /预计空间足够/);
  assert.match(recovery, /<button(?=[^>]*>开始转移)(?![^>]*disabled)[^>]*>/,
    "the backend may permit a recovery check when the full-size estimate exceeds the remaining copy size");
  const empty = await showTransfer({ preview: plan({ recording_count: 0, cleanup_count: 0 }) });
  assert.match(empty, /没有需要转移或清理的录播/);
  assert.doesNotMatch(empty, /开始转移|type="checkbox"/);
  const cleanup = await showTransfer({ preview: plan({ recording_count: 0, cleanup_count: 2 }) });
  assert.match(cleanup, /开始转移/);
  assert.match(cleanup, /type="checkbox"/);
  const failed = await showTransfer({ rows: [task({
    status: "failed", message: "清理来源失败",
    metadata: JSON.stringify({ source_backend_id: "A", target_backend_id: "B", result: {
      transferred_recordings: 2, cleaned_recordings: 1, failed_recording: "live-3", error: "permission denied",
    } }),
  })] });
  assert.match(failed, /已转移 2 条，已清理来源 1 条/);
  assert.match(failed, /停止于：live-3/);
  assert.match(failed, /permission denied/);
  assert.match(failed, /已完成的录播不会回滚/);
  assert.match(failed, /重新检查这次转移/);
  assert.match(failed, /任务详情/);
  const active = await showTransfer({ rows: [task({ status: "processing", message: "正在复制 1/3" })] });
  assert.match(active, /正在复制 1\/3/);
  assert.match(active, /可以关闭窗口，转移会在后台继续/);
  assert.doesNotMatch(active, /id="transfer-source"|id="transfer-target"|aria-label="转移步骤"/,
    "active transfers replace the new-transfer form instead of competing with it");
});

test("same-filesystem preview explains rename without requiring a full copy's free space", async () => {
  const body = await showTransfer({
    preview: plan({ transfer_mode: "rename", required_bytes: 0, available_bytes: 0 }),
    confirmed: true,
  });
  assert.match(body, /同文件系统，优先快速移动/);
  assert.match(body, /预计所需空间<\/dt><dd[^>]*>0 B/);
  assert.match(body, /无需预留完整副本空间/);
  assert.match(body, /不复制媒体数据/);
  assert.match(body, /跨文件系统错误/);
  assert.doesNotMatch(body, /预计空间不足|每条复制校验完成后切换存储/);
  assert.match(body, /<button(?=[^>]*>开始转移)(?![^>]*disabled)[^>]*>/);
  const mixed = await showTransfer({ preview: plan({ transfer_mode: "mixed" }) });
  assert.match(mixed, /快速移动与复制/);
  assert.match(mixed, /空间只按 DB 大小估算/);
});
