import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { compile } from "svelte/compiler";
import { render } from "svelte/server";

const plain = value => JSON.parse(JSON.stringify(value));
const config = () => ({
  default_backend: "local",
  backends: {
    local: { type: "local", root: "/srv/recordings" },
    archive: { type: "local", root: "/mnt/archive" },
  },
});
const transpile = source => ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText;
const helperSource = await readFile(new URL("../../src/lib/recording-storage.ts", import.meta.url), "utf8");
const helper = new vm.SourceTextModule(transpile(helperSource));
await helper.link(() => { throw new Error("Unexpected runtime dependency"); });
await helper.evaluate();
const { storageBackend, storageBackendEntries, storageBackendSupported, storageBackendLocation } = helper.namespace;

async function handlers(path, state, { mount = false } = {}) {
  const source = await readFile(new URL(`../../src/${path}`, import.meta.url), "utf8");
  const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
  const parsed = ts.createSourceFile("component.ts", script, ts.ScriptTarget.Latest, true);
  const functions = parsed.statements.filter(ts.isFunctionDeclaration);
  const context = vm.createContext(state);
  vm.runInContext(transpile(functions.map(node => node.getText(parsed)).join("\n")), context);
  if (mount) {
    const runtime = new vm.SourceTextModule(transpile(await readFile(new URL("../../src/lib/state-refresh.ts", import.meta.url), "utf8")), { context });
    await runtime.link(() => new vm.SyntheticModule(["listen"], function () {
      this.setExport("listen", state.listen);
    }, { context }));
    await runtime.evaluate();
    Object.assign(state, runtime.namespace);
    for (const node of parsed.statements) {
      const watcher = ts.isVariableStatement(node) && node.declarationList.declarations
        .some(item => item.name.getText(parsed) === "catalogRefresh");
      const mount = ts.isExpressionStatement(node) && ts.isCallExpression(node.expression)
        && node.expression.expression.getText(parsed) === "onMount";
      if (watcher || mount) vm.runInContext(transpile(node.getText(parsed)), context);
    }
  }
  return state;
}

async function settingsFixture({ mount = false, overrides = {} } = {}) {
  const requests = [];
  let persisted = config();
  const state = await handlers("lib/components/RecordingStorageSettings.svelte", {
    storage: config(), defaultId: "local", newId: "", newRoot: "",
    newType: "local", s3Endpoint: "", s3Bucket: "", s3Region: "us-east-1",
    s3Prefix: "", s3PathStyle: true, s3AccessKeyId: "", s3SecretAccessKey: "", s3SessionToken: "", editingCredentials: null,
    loading: false, saving: false, browsing: false, error: "", saved: "", removingId: null, storageBackend,
    storageBackendSupported,
    transferBusy: false, transferStarting: false,
    view: "locations", showAdd: false,
    catalogStatuses: {}, catalogRequest: 0, disposed: false,
    tick: async () => {}, newIdInput: { focus() {} },
    invoke: async (command, args) => {
      if (command === "get_config") return { storage: plain(persisted) };
      if (command === "get_storage_catalog_status") return [];
      requests.push({ command, args: plain(args) });
      persisted = plain(args.storage);
      return plain(args.storage);
    },
    ...overrides,
  }, { mount });
  return { state, requests };
}

test("storage settings open on demand in a focused modal instead of expanding the settings page", async () => {
  const settings = await readFile(new URL("../../src/page/Setting.svelte", import.meta.url), "utf8");
  assert.match(settings, /data-setting="recording-storage"/);
  assert.match(settings, /onclick=\{\(\) => showStorageSettings = true\}/);
  assert.match(settings, /\{#if showStorageSettings\}\s*<RecordingStorageSettings onclose=/);
  const modal = await readFile(new URL("../../src/lib/components/RecordingStorageSettings.svelte", import.meta.url), "utf8");
  assert.match(modal, /role="dialog" aria-modal="true" aria-labelledby="storage-settings-title"/);
  assert.match(modal, /use:trapFocus/);
  assert.match(modal, /class="ui-modal-body space-y-3"/);
  assert.match(modal, /w-\[800px\]/);
  assert.match(modal, /onMount\(\(\) => \{[\s\S]*?void load\(\);/);
});

test("viewport modal leaves page containment and removes its relocated node on destroy", async () => {
  const { state } = await settingsFixture();
  let appended;
  let removed = false;
  state.document = { body: { appendChild(node) { appended = node; } } };
  const node = { remove() { removed = true; } };
  const action = state.mountToViewport(node);
  assert.equal(appended, node);
  action.destroy();
  assert.equal(removed, true);
  const source = await readFile(new URL("../../src/lib/components/RecordingStorageSettings.svelte", import.meta.url), "utf8");
  assert.match(source, /style:display=\{pageVisible\(\) \? undefined : "none"\} inert=\{!pageVisible\(\)\}/);
});

test("view switches clear hidden confirmations but preserve the add draft", async () => {
  const { state } = await settingsFixture();
  state.showAdd = true;
  state.newId = "draft";
  state.removingId = "archive";
  state.selectView("transfer");
  assert.equal(state.view, "transfer");
  assert.equal(state.removingId, null);
  assert.equal(state.newId, "draft");
  state.transferStarting = true;
  state.selectView("locations");
  assert.equal(state.view, "transfer", "submission owns navigation until its outcome is known");
});

test("storage rows group default/remove actions and leave transfers to the transfer view", async () => {
  const source = await readFile(new URL("../../src/lib/components/RecordingStorageSettings.svelte", import.meta.url), "utf8");
  assert.match(source, /class="ui-badge tabular-nums"/);
  assert.match(source, /aria-label=\{`\$\{Object.keys\(storage.backends\).length\} 个存储位置`\}/);
  const group = source.match(/<div class="storage-actions[\s\S]*?<\/div>/)[0];
  assert.match(group, /<Trash2[^>]*\/>移除/);
  assert.doesNotMatch(group, /setDefault/, "default switch belongs beside the name, not the action group");
  assert.match(source, /group-hover:opacity-100[\s\S]{0,200}?onclick=\{\(\) => setDefault\(id\)\}/);
  assert.doesNotMatch(source, /openTransfer|转移录播|ArrowRightLeft/);
});

test("add editor opens on demand and focuses only after tick, without writing config", async () => {
  const { state, requests } = await settingsFixture();
  let flush;
  let focused = 0;
  state.tick = () => new Promise(resolve => { flush = resolve; });
  state.newIdInput = { focus() { focused++; } };
  state.removingId = "archive";
  state.error = "Old error";
  state.saved = "Old success";
  assert.equal(state.showAdd, false);
  const opening = state.openAdd();
  assert.equal(state.showAdd, true);
  assert.equal(state.removingId, null);
  assert.equal(state.error, "");
  assert.equal(state.saved, "");
  assert.equal(focused, 0);
  flush();
  await opening;
  assert.equal(focused, 1);
  assert.deepEqual(requests, []);
});

test("storage modal closes with Escape but protects writes and native folder selection", async () => {
  const { state } = await settingsFixture();
  let closed = 0;
  state.onclose = () => { closed++; };
  state.pageVisible = () => true;
  const escape = (extra = {}) => ({
    key: "Escape", defaultPrevented: false, isComposing: false,
    preventDefault() {}, stopImmediatePropagation() {}, ...extra,
  });
  state.saving = true;
  state.close();
  state.handleKeydown(escape());
  state.saving = false;
  state.browsing = true;
  state.close();
  state.handleKeydown(escape());
  assert.equal(closed, 0);
  state.browsing = false;
  state.removingId = "archive";
  state.showAdd = true;
  state.handleKeydown(escape());
  assert.equal(state.removingId, null, "Escape cancels the removal confirmation first");
  assert.equal(state.showAdd, true);
  assert.equal(closed, 0);
  state.handleKeydown(escape());
  assert.equal(state.showAdd, false, "the next Escape cancels the add editor before closing");
  assert.equal(closed, 0);
  state.handleKeydown(escape({ isComposing: true }));
  state.handleKeydown(escape({ defaultPrevented: true }));
  state.pageVisible = () => false;
  state.handleKeydown(escape());
  assert.equal(closed, 0);
  state.pageVisible = () => true;
  state.handleKeydown(escape());
  assert.equal(closed, 1);
});

test("backend lookup never treats unknown IDs or inherited object properties as configured", () => {
  assert.equal(storageBackend(config(), "missing"), undefined);
  assert.equal(storageBackend(config(), "constructor"), undefined);
  assert.equal(storageBackend(config(), "__proto__"), undefined);
  assert.equal(storageBackend(config(), "local").root, "/srv/recordings");
});

test("backend ordering is stable across response insertion order and default changes", () => {
  const ids = ["disk10", "disk2", "Test3", "test3", "default"];
  const entry = id => [id, { type: "local", root: `/${id}` }];
  const first = { default_backend: "default", backends: Object.fromEntries(ids.map(entry)) };
  const next = { default_backend: "disk10", backends: Object.fromEntries([...ids].reverse().map(entry)) };
  const order = value => Array.from(storageBackendEntries(value), ([id]) => id);
  assert.deepEqual(order(first), ["default", "disk2", "disk10", "Test3", "test3"]);
  assert.deepEqual(order(next), order(first));
  assert.deepEqual(Object.keys(first.backends), ids, "sorting must not mutate the config snapshot used by CAS");
});

test("every storage list and selector uses the shared stable ordering", async () => {
  for (const name of ["RecordingStorageSettings", "StorageBackendSelect", "StorageTransferSettings"]) {
    const source = await readFile(new URL(`../../src/lib/components/${name}.svelte`, import.meta.url), "utf8");
    assert.match(source, /\{#each storageBackendEntries\(storage\)/);
    assert.doesNotMatch(source, /\{#each Object.entries\(storage.backends\)/);
  }
});

test("catalog status reads do not reconnect unless refresh is requested", async () => {
  const { state } = await settingsFixture();
  const calls = [];
  state.invoke = async (command, args) => {
    calls.push([command, plain(args)]);
    return [{ alias: "archive", ready: true, writable: false, error: null }];
  };
  await state.loadCatalogStatus();
  await state.loadCatalogStatus(true);
  assert.deepEqual(calls, [
    ["get_storage_catalog_status", { refresh: false }],
    ["get_storage_catalog_status", { refresh: true }],
  ]);
  assert.deepEqual(plain(state.catalogStatuses.archive), {
    alias: "archive", ready: true, writable: false, error: null,
  });
  const before = plain(state.storage);
  state.invoke = async () => { throw new Error("offline"); };
  await state.loadCatalogStatus(true);
  assert.deepEqual(plain(state.catalogStatuses), {});
  assert.deepEqual(plain(state.storage), before);
});

for (const status of [
  { alias: "archive", ready: true, writable: true, error: null },
  { alias: "archive", ready: false, writable: false, error: "S3 connection failed" },
]) {
  test(`an open storage dialog refreshes startup ${status.ready ? "success" : "failure"} without resetting edits`, async () => {
    const events = new Map();
    const timers = new Map();
    const intervals = new Map();
    const calls = [];
    const initialReady = !status.ready;
    let next = { ...status, ready: initialReady, writable: initialReady,
      error: initialReady ? null : "正在连接存储目录" };
    let cleanup;
    let timerId = 0;
    const settle = async () => { for (let i = 0; i < 40; i++) await Promise.resolve(); };
    const { state } = await settingsFixture({ mount: true, overrides: {
      console,
      onMount: callback => { cleanup = callback(); },
      listen: async (name, callback) => {
        events.set(name, callback);
        return () => events.delete(name);
      },
      document: { visibilityState: "visible", addEventListener() {}, removeEventListener() {} },
      window: { addEventListener() {}, removeEventListener() {} },
      setTimeout: callback => { timers.set(++timerId, callback); return timerId; },
      clearTimeout: id => timers.delete(id),
      setInterval: callback => { intervals.set(++timerId, callback); return timerId; },
      clearInterval: id => intervals.delete(id),
      invoke: async (command, args) => {
        calls.push({ command, args: args && plain(args) });
        if (command === "get_config") return { storage: config() };
        assert.equal(command, "get_storage_catalog_status");
        return [{ ...next }];
      },
    } });
    await settle();
    assert.equal(state.catalogStatuses.archive.ready, initialReady);
    assert.ok(events.has("records-changed"));
    const configReads = calls.filter(call => call.command === "get_config").length;
    state.newId = "keep this draft";
    state.showAdd = true;
    state.defaultId = "archive";
    state.view = "transfer";
    next = status;
    events.get("records-changed")({ payload: null });
    for (const callback of [...timers.values()]) callback();
    timers.clear();
    await settle();
    assert.deepEqual(plain(state.catalogStatuses.archive), status);
    assert.equal(calls.at(-1).args.refresh, false, "completion reads must not force another S3 scan");
    assert.equal(calls.filter(call => call.command === "get_config").length, configReads);
    assert.equal(state.newId, "keep this draft");
    assert.equal(state.showAdd, true);
    assert.equal(state.defaultId, "archive");
    assert.equal(state.view, "transfer");
    cleanup();
    assert.equal(events.size, 0);
    assert.equal(timers.size, 0);
    assert.equal(intervals.size, 0);
  });
}

test("older catalog responses and responses after closing cannot overwrite current status", async () => {
  const { state } = await settingsFixture();
  const pending = [];
  state.invoke = () => new Promise((resolve, reject) => pending.push({ resolve, reject }));
  const old = state.loadCatalogStatus(true);
  const current = state.loadCatalogStatus();
  const ready = [{ alias: "archive", ready: true, writable: true, error: null }];
  pending[1].resolve(ready);
  await current;
  pending[0].resolve([{ ...ready[0], ready: false, error: "stale placeholder" }]);
  await old;
  assert.equal(state.catalogStatuses.archive.ready, true);
  const closing = state.loadCatalogStatus();
  state.disposed = true;
  pending[2].reject(new Error("late failure"));
  await closing;
  assert.equal(state.catalogStatuses.archive.ready, true);
});

test("changing the default preserves all backend IDs and roots without moving recordings", async () => {
  const { state, requests } = await settingsFixture();
  await state.setDefault("archive");
  assert.deepEqual(requests, [{
    command: "set_storage_config",
    args: { storage: { ...config(), default_backend: "archive" }, expected: config() },
  }]);
  assert.equal(state.storage.default_backend, "archive");
  assert.match(state.saved, /已保存/);
});

test("row default actions and add editor honor config and transfer locks", async () => {
  for (const lock of ["saving", "browsing", "transferBusy"]) {
    const { state, requests } = await settingsFixture();
    state[lock] = true;
    await state.setDefault("archive");
    await state.openAdd();
    assert.equal(state.defaultId, "local");
    assert.equal(state.showAdd, false);
    assert.deepEqual(requests, []);
  }
});

test("adding a local backend uses the server path and never overwrites an existing ID", async () => {
  const { state, requests } = await settingsFixture();
  await state.openAdd();
  state.newId = " local ";
  state.newRoot = "/different/location";
  await state.save(true);
  assert.equal(requests.length, 0);
  assert.match(state.error, /未使用/);
  assert.equal(state.showAdd, true, "invalid input keeps the editor open");
  state.newId = "disk-2";
  state.newRoot = " /server/disk-2 ";
  await state.save(true);
  assert.equal(requests.length, 1);
  assert.deepEqual(requests[0].args.storage, {
    ...config(),
    backends: { ...config().backends, "disk-2": { type: "local", root: "/server/disk-2" } },
  });
  assert.equal(state.newId, "");
  assert.equal(state.newRoot, "");
  assert.equal(state.showAdd, false);
  assert.deepEqual(requests[0].args.expected, config(), "add preserves the CAS snapshot");
});

test("unknown and unsupported defaults cannot silently fall back to local", async () => {
  const { state, requests } = await settingsFixture();
  for (const id of ["missing", "__proto__", "s3"]) {
    state.storage.backends.s3 = { type: "s3", root: "bucket" };
    state.defaultId = id;
    await state.save(false);
    assert.match(state.error, /请选择/);
  }
  assert.equal(requests.length, 0);
});

test("S3 settings submit write-only credentials separately from the public configuration", async () => {
  const { state, requests } = await settingsFixture();
  state.newType = "s3";
  state.newId = "rustfs";
  state.s3Endpoint = " http://127.0.0.1:9000 ";
  state.s3Bucket = "recordings";
  state.s3Prefix = "app";
  state.s3AccessKeyId = "test-access";
  state.s3SecretAccessKey = "test-secret";
  state.s3SessionToken = "test-token";
  await state.save(true);
  const definition = requests[0].args.storage.backends.rustfs;
  assert.deepEqual(definition, {
    type: "s3", root: "", s3: {
      endpoint: "http://127.0.0.1:9000", bucket: "recordings", region: "us-east-1",
      prefix: "app", path_style: true,
    },
  });
  assert.deepEqual(requests[0].args.credentials, {
    rustfs: { access_key_id: "test-access", secret_access_key: "test-secret", session_token: "test-token" },
  });
  assert.equal(state.s3AccessKeyId, "");
  assert.equal(state.s3SecretAccessKey, "");
  assert.equal(state.s3SessionToken, "");
  assert.equal(storageBackendLocation(definition), "s3://recordings/app · http://127.0.0.1:9000");
  assert.equal(storageBackendSupported(definition), true);
  await state.setDefault("rustfs");
  assert.equal(requests[1].args.storage.default_backend, "rustfs");
  assert.equal(requests[1].args.credentials, undefined, "ordinary writes must preserve saved credentials");
  const recorder = await recorderFixture();
  recorder.state.storage.backends.rustfs = definition;
  recorder.state.choice = "rustfs";
  await recorder.state.save();
  assert.equal(recorder.requests[0].args.storageBackendId, "rustfs");
});

test("S3 credential edits require complete input and preserve the CAS snapshot without changing the namespace", async () => {
  const { state, requests } = await settingsFixture();
  const before = plain(state.storage);
  state.editCredentials("remote");
  await state.saveCredentials();
  assert.equal(requests.length, 0);
  state.s3AccessKeyId = "updated-id";
  state.s3SecretAccessKey = "updated-secret";
  await state.saveCredentials();
  assert.deepEqual(requests[0].args.storage, before);
  assert.deepEqual(requests[0].args.expected, before);
  assert.deepEqual(requests[0].args.credentials, {
    remote: { access_key_id: "updated-id", secret_access_key: "updated-secret", session_token: "" },
  });
  assert.equal(state.editingCredentials, null);
  assert.equal(state.s3SecretAccessKey, "");
  state.editCredentials("remote");
  state.s3SecretAccessKey = "discarded";
  state.cancelCredentials();
  assert.equal(state.s3SecretAccessKey, "");
});

test("removal requires explicit confirmation and cannot remove the persisted default", async () => {
  const { state, requests } = await settingsFixture();
  await state.removeBackend("archive");
  assert.equal(requests.length, 0);
  state.removingId = "local";
  state.defaultId = "archive"; // Unsaved draft does not unlock default removal.
  await state.removeBackend("local");
  assert.match(state.error, /先保存其他默认存储/);
  assert.equal(requests.length, 0);
  state.removingId = "archive";
  await state.removeBackend("archive");
  assert.deepEqual(requests, [{
    command: "set_storage_config",
    args: { storage: { default_backend: "local", backends: { local: config().backends.local } }, expected: config() },
  }]);
  assert.equal(state.removingId, null);
  assert.match(state.saved, /从列表隐藏/);
  assert.match(state.saved, /文件和目录数据库未删除/);
});

test("failed removal keeps the backend and confirmation and clears stale success", async () => {
  const { state } = await settingsFixture();
  state.removingId = "archive";
  state.saved = "Old success";
  const invoke = state.invoke;
  state.invoke = async (command, args) => {
    if (command === "set_storage_config") throw new Error("active recording lease");
    return invoke(command, args);
  };
  await state.removeBackend("archive");
  assert.deepEqual(plain(state.storage), config());
  assert.equal(state.removingId, "archive");
  assert.equal(state.saved, "");
  assert.equal(state.saving, false);
  assert.match(state.error, /active recording lease/);
});

test("stale full-config writes cannot silently remove another client's backend", async () => {
  for (const operation of ["default", "add", "remove"]) {
    const { state, requests } = await settingsFixture();
    state.invoke = async (command, args) => {
      assert.equal(command, "set_storage_config");
      assert.deepEqual(plain(args.expected), config(), "send the original DB snapshot for atomic CAS");
      requests.push({ command, args });
      throw new Error("存储配置已被其他窗口或用户修改，请重新加载");
    };
    state.defaultId = "archive";
    state.newId = "draft";
    state.newRoot = "/draft";
    state.showAdd = true;
    state.removingId = "archive";
    if (operation === "remove") await state.removeBackend("archive");
    else await state.save(operation === "add");
    assert.equal(requests.length, 1);
    assert.match(state.error, /其他窗口或用户修改/);
    assert.equal(state.newId, "draft");
    assert.equal(state.showAdd, true, "failed writes must not dismiss the editor");
    assert.deepEqual(plain(state.storage), config());
  }
});

test("readonly failures retain the last saved config and draft, then allow retry", async () => {
  const { state } = await settingsFixture();
  state.defaultId = "archive";
  const invoke = state.invoke;
  state.invoke = async () => { throw new Error("405 readonly mode"); };
  await state.save(false);
  assert.equal(state.storage.default_backend, "local");
  assert.equal(state.defaultId, "archive");
  assert.equal(state.saving, false);
  assert.equal(state.saved, "");
  assert.match(state.error, /405 readonly mode/);
  state.invoke = invoke;
  await state.save(false);
  assert.equal(state.storage.default_backend, "archive");
  assert.equal(state.error, "");
});

test("storage load failures can retry; loading and pending writes block mutations", async () => {
  const { state, requests } = await settingsFixture();
  state.loading = true;
  await state.save(false);
  state.loading = false;
  let resolve;
  state.invoke = async command => command === "get_config"
    ? { storage: config() }
    : new Promise(done => { resolve = done; });
  const pending = state.save(false);
  assert.equal(state.saving, true);
  await state.save(true);
  await new Promise(done => setImmediate(done));
  resolve(config());
  await pending;
  assert.equal(state.saving, false);
  assert.equal(requests.length, 0);
  state.invoke = async () => { throw new Error("offline"); };
  await state.load();
  assert.match(state.error, /offline/);
  assert.equal(state.loading, false);
  state.invoke = async () => ({ storage: config() });
  await state.load();
  assert.equal(state.error, "");
  assert.equal(state.defaultId, "local");
});

async function recorderFixture() {
  const requests = [];
  const state = {
    status: { recorder: { recording: false, room_info: { platform: "bilibili", room_id: "123" } } },
    storage: config(), choice: null, disabled: false, loading: false, saving: false,
    error: "", saved: "", storageBackend, storageBackendSupported, refreshed: 0,
    onchanged: async () => { state.refreshed++; },
    invoke: async (command, args) => { requests.push({ command, args: plain(args) }); return {}; },
  };
  Object.defineProperty(state, "locked", {
    get: () => state.disabled || state.loading || state.saving,
  });
  await handlers("lib/components/RecorderStorageSettings.svelte", state);
  return { state, requests };
}

test("room save distinguishes inherited null from an explicit choice of the same default", async () => {
  const { state, requests } = await recorderFixture();
  await state.save();
  state.choice = "local";
  await state.save();
  assert.deepEqual(requests.map(request => request.args), [
    { platform: "bilibili", roomId: "123", storageBackendId: null },
    { platform: "bilibili", roomId: "123", storageBackendId: "local" },
  ]);
  assert.ok(requests.every(request => request.command === "set_recorder_storage"));
  assert.equal(state.refreshed, 2);
});

test("active recording accepts a next-recording preference; unknown and readonly changes fail", async () => {
  const { state, requests } = await recorderFixture();
  state.status.recorder.recording = true;
  await state.save();
  assert.equal(requests.length, 1);
  assert.match(state.saved, /已有录播仍绑定原存储/);
  state.status.recorder.recording = false;
  state.choice = "missing";
  await state.save();
  assert.equal(requests.length, 1);
  assert.match(state.error, /不会自动改用/);
  state.choice = "archive";
  state.invoke = async () => { throw new Error("405 readonly"); };
  await state.save();
  assert.match(state.error, /405 readonly/);
  assert.equal(state.saved, "");
  assert.equal(state.refreshed, 1);
  assert.equal(state.choice, "archive");
});

test("a recorder refresh failure does not misreport a successful preference save as failed", async () => {
  const { state, requests } = await recorderFixture();
  state.onchanged = async () => { throw new Error("offline"); };
  await state.save();
  assert.equal(requests.length, 1);
  assert.match(state.saved, /已保存到数据库/);
  assert.match(state.error, /偏好已保存，但刷新/);
});

test("add recorder sends the selected override and preserves errors for web clients", async () => {
  const requests = [];
  const state = await handlers("page/Room.svelte", {
    creatingRoom: false, storageLoading: false, addStorage: config(),
    storageError: "", addStorageId: "archive", addError: "", addModal: true, addRoom: "123",
    storageBackend, storageBackendSupported, URLSearchParams,
    invoke: async (command, args) => { requests.push({ command, args: plain(args) }); return {}; },
  });
  state.addNewRecorder("123", "bilibili");
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(requests, [{
    command: "add_recorder",
    args: { platform: "bilibili", roomId: "123", extra: "", storageBackendId: "archive" },
  }]);
  state.addStorageId = null;
  state.invoke = async () => { throw new Error("405 readonly"); };
  state.addNewRecorder("123", "bilibili");
  await new Promise(resolve => setImmediate(resolve));
  assert.match(state.addError, /405 readonly/);
  assert.equal(state.creatingRoom, false);
});

test("closing and reopening the add dialog ignores an older storage lookup", async () => {
  const pending = [];
  const state = await handlers("page/Room.svelte", {
    storageLoadRequest: 0, addModal: true, storageLoading: false,
    storageError: "", addStorage: null,
    invoke: () => new Promise(resolve => pending.push(resolve)),
  });
  const old = state.loadAddStorage();
  ++state.storageLoadRequest; // effect cleanup on close
  const current = state.loadAddStorage();
  pending[1]({ storage: { ...config(), default_backend: "archive" } });
  await current;
  pending[0]({ storage: config() });
  await old;
  assert.equal(state.addStorage.default_backend, "archive");
  assert.equal(state.storageLoading, false);
});

test("selector renders explicit missing IDs and unsupported backends without replacing them", async () => {
  const context = vm.createContext({});
  const source = await readFile(new URL("../../src/lib/components/StorageBackendSelect.svelte", import.meta.url), "utf8");
  const component = new vm.SourceTextModule(compile(source, { generate: "server" }).js.code, { context });
  const linked = new Map();
  await component.link(async name => {
    const exports = name === "../recording-storage" ? { storageBackend, storageBackendEntries, storageBackendSupported, storageBackendLocation } : await import(name);
    if (!linked.has(name)) linked.set(name, new vm.SyntheticModule(Object.keys(exports), function () {
      for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
    }, { context }));
    return linked.get(name);
  });
  await component.evaluate();
  const storage = config();
  storage.backends.future = { type: "s3", root: "bucket" };
  const show = value => render(component.namespace.default, {
    props: { storage, id: "test-storage", value },
  }).body;
  assert.match(show(null), /跟随应用默认（local）/);
  assert.match(show(null), /\/srv\/recordings/);
  assert.match(show("missing"), /未知存储：missing/);
  assert.match(show("missing"), /不会自动改用其他存储/);
  assert.match(show("missing"), /value="missing"[^>]*selected/);
  assert.match(show("local"), /value="local"[^>]*selected/);
  assert.match(show("local"), /value="future"[^>]*disabled/);
  const order = () => [...show("local").matchAll(/<option value="([^"]+)"/g)]
    .map(match => match[1]).filter(id => storageBackend(storage, id));
  assert.deepEqual(order(), ["archive", "future", "local"]);
  storage.backends = Object.fromEntries(Object.entries(storage.backends).reverse());
  storage.default_backend = "archive";
  assert.deepEqual(order(), ["archive", "future", "local"]);
  assert.match(show("local"), /value="local"[^>]*selected/);
});

test("generic desktop and HTTP invokers preserve storage wire keys and null overrides", async () => {
  const source = await readFile(new URL("../../src/lib/invoker.ts", import.meta.url), "utf8");
  for (const tauri of [false, true]) {
    const requests = [];
    const context = vm.createContext({
      window: tauri ? { __TAURI_INTERNALS__: {} } : {},
      localStorage: { getItem: () => "http://server:3000" },
      fetch: async (url, options) => {
        requests.push({ command: url.split("/").pop(), args: JSON.parse(options.body) });
        return { ok: true, status: 200, json: async () => ({ code: 0, data: {} }) };
      },
    });
    const dependencies = {
      "@tauri-apps/api/core": { invoke: async (command, args) => { requests.push({ command, args: plain(args) }); return {}; } },
      "@tauri-apps/plugin-http": { fetch() {} },
      "@tauri-apps/plugin-shell": { open() {} },
      "@tauri-apps/plugin-deep-link": { onOpenUrl() {} },
      "./events": { createEventClient: () => ({ listen() {} }) },
    };
    const module = new vm.SourceTextModule(transpile(source), { context });
    const linked = new Map();
    await module.link(name => {
      const exports = dependencies[name];
      assert.ok(exports, name);
      if (!linked.has(name)) linked.set(name, new vm.SyntheticModule(Object.keys(exports), function () {
        for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
      }, { context }));
      return linked.get(name);
    });
    await module.evaluate();
    const operations = [
      { command: "set_storage_config", args: { storage: config(), expected: config() } },
      { command: "set_recorder_storage", args: { platform: "bilibili", roomId: "123", storageBackendId: null } },
      { command: "add_recorder", args: { platform: "bilibili", roomId: "123", extra: "", storageBackendId: "archive" } },
      { command: "add_recorder", args: { platform: "bilibili", roomId: "456", extra: "" } },
    ];
    for (const { command, args } of operations) await module.namespace.invoke(command, args);
    assert.deepEqual(requests, operations);
  }
});
