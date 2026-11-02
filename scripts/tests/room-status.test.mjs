import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../../src/lib/components/RoomStatusModal.svelte", import.meta.url), "utf8");
const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
const parsed = ts.createSourceFile("room-status.ts", script, ts.ScriptTarget.Latest, true);
const code = ts.transpileModule(parsed.statements.filter((node) => !ts.isImportDeclaration(node))
  .map((node) => node.getText(parsed)).join("\n"), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText;

const room = { room_info: { platform: "bilibili", room_id: "123" } };
const records = (count) => Array.from({ length: count }, (_, index) => ({ live_id: String(count - index) }));

function fixture(invoke, get = async () => { throw new Error("unexpected image request"); }) {
  let mount;
  const subscriptions = [];
  const createdUrls = new Map();
  const revokedUrls = [];
  let urlId = 0;
  const context = vm.createContext({
    invoke,
    get,
    pageVisibility: () => () => true,
    URL: {
      createObjectURL: (blob) => { const url = `blob:cover-${++urlId}`; createdUrls.set(url, blob); return url; },
      revokeObjectURL: (url) => revokedUrls.push(url),
    },
    loadRoomArchives: async (scope, active) => {
      const result = await invoke("get_archives", { platform: scope.platform, roomId: scope.room_id });
      return active() ? result : [];
    },
    summarizeRoomArchives: () => ({ count: 0, length: 0, size: 0 }),
    $props: () => ({ room, onclose() {} }),
    $state: (value) => value,
    $derived: (value) => value,
    onMount: (callback) => { mount = callback; },
    createStateRefresh: (refresh) => ({ run: () => Promise.resolve().then(refresh) }),
    watchStateChanges: (_refresh, events, fallbackMs) => {
      const subscription = { events: [...events], fallbackMs, stopped: false };
      subscriptions.push(subscription);
      return () => { subscription.stopped = true; };
    },
  });
  vm.runInContext(code, context);
  return {
    context,
    get: (expression) => vm.runInContext(expression, context),
    mount: () => mount(),
    subscriptions,
    createdUrls,
    revokedUrls,
  };
}

test("status cover follows fresh metadata, reuses unchanged sources, and releases replaced images", async () => {
  let source = "https://example.test/cover-1.jpg";
  const requests = [];
  const f = fixture(async () => ({ recorder: { ...room, room_info: { ...room.room_info, room_cover: source } } }), async (url) => {
    requests.push(url);
    return { ok: true, blob: async () => url };
  });
  const cleanup = f.mount();
  const settle = () => new Promise(resolve => setImmediate(resolve));
  await f.context.refreshStatus();
  await settle();
  const first = f.get("coverUrl");
  assert.equal(f.createdUrls.get(first), source);
  await f.context.refreshStatus();
  await settle();
  assert.equal(requests.length, 1);
  source = "https://example.test/cover-2.jpg";
  await f.context.refreshStatus();
  await settle();
  const second = f.get("coverUrl");
  assert.notEqual(second, first);
  assert.equal(f.createdUrls.get(second), source);
  assert.deepEqual(f.revokedUrls, [first]);
  cleanup();
  assert.deepEqual(f.revokedUrls, [first, second]);
});

test("slow and failed cover requests cannot overwrite newer images or update a closed dialog", async () => {
  const requests = new Map();
  const f = fixture(async () => {}, url => new Promise(resolve => requests.set(url, resolve)));
  const cleanup = f.mount();
  const first = f.context.refreshCover("old");
  const second = f.context.refreshCover("new");
  requests.get("new")({ ok: true, blob: async () => "new" });
  await second;
  requests.get("old")({ ok: true, blob: async () => "old" });
  await first;
  assert.equal(f.createdUrls.get(f.get("coverUrl")), "new");
  const failed = f.context.refreshCover("failed");
  requests.get("failed")({ ok: false, status: 404 });
  await failed;
  assert.equal(f.get("coverUrl"), null);
  const pending = f.context.refreshCover("pending");
  cleanup();
  requests.get("pending")({ ok: true, blob: async () => "pending" });
  await pending;
  assert.equal(f.get("coverUrl"), null);
  assert.equal(f.createdUrls.size, 1);
});

test("room history passes the full room identity to the shared loader and replaces stale snapshots", async () => {
  let rows = records(45);
  const requests = [];
  const f = fixture(async (command, args) => {
    assert.equal(command, "get_archives");
    requests.push({ ...args });
    return rows;
  });
  await f.context.refreshArchives();
  assert.deepEqual(requests[0], { platform: "bilibili", roomId: "123" });
  assert.equal(f.get("archives.length"), 45);
  assert.equal(f.get("archivesLoaded"), true);
  rows = [{ live_id: "new recording" }, ...rows.filter((row) => row.live_id !== "40")];
  await f.context.refreshArchives();
  assert.equal(requests.length, 2);
  assert.deepEqual(Array.from(f.get("archives"), (row) => row.live_id), rows.map((row) => row.live_id));
});

test("record history keeps loaded rows on failure and permits a successful retry", async () => {
  let fail = false;
  const f = fixture(async () => {
    if (fail) throw "offline";
    return records(2);
  });
  await f.context.refreshArchives();
  fail = true;
  await f.context.refreshArchives();
  assert.equal(f.get("archives.length"), 2);
  assert.match(f.get("archivesError"), /offline/);
  assert.equal(f.get("archivesLoading"), false);
  fail = false;
  await f.context.refreshArchives();
  assert.equal(f.get("archivesError"), "");
});

test("status refresh keeps the last snapshot during an API failure and recovers on retry", async () => {
  let response = { recorder: room, current_record: { length: 60, size: 30_000_000 } };
  let fail = false;
  const f = fixture(async (command, args) => {
    assert.equal(command, "get_recorder_status");
    assert.deepEqual({ ...args }, { platform: "bilibili", roomId: "123" });
    if (fail) throw "connection lost";
    return response;
  });
  await f.context.refreshStatus();
  assert.equal(f.get("status.current_record.length"), 60);
  fail = true;
  await f.context.refreshStatus();
  assert.equal(f.get("status.current_record.length"), 60);
  assert.match(f.get("statusError"), /connection lost/);
  assert.equal(f.get("refreshing"), false);
  fail = false;
  response = { ...response, current_record: { length: 120, size: 60_000_000 } };
  await f.context.refreshStatus();
  assert.equal(f.get("status.current_record.length"), 120);
  assert.equal(f.get("statusError"), "");
});

test("closing stops subscriptions and ignores pending status and history responses", async () => {
  const pending = [];
  const f = fixture(() => new Promise((resolve) => pending.push(resolve)));
  const cleanup = f.mount();
  assert.deepEqual(f.subscriptions.map(({ events }) => events), [["recorders-changed", "records-changed"], ["records-changed"]]);
  assert.equal(f.subscriptions[0].fallbackMs, 5000);
  const status = f.context.refreshStatus();
  const history = f.context.refreshArchives();
  cleanup();
  pending[0]({ recorder: room });
  pending[1](records(5));
  await Promise.all([status, history]);
  assert.ok(f.subscriptions.every(({ stopped }) => stopped));
  assert.equal(f.get("status"), null);
  assert.equal(f.get("archives.length"), 0);
  assert.equal(f.get("archivesLoaded"), false);
});
