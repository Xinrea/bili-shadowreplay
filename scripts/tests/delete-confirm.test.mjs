import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const read = (path) => readFile(new URL(`../../${path}`, import.meta.url), "utf8");
const source = await read("src/lib/components/DeleteConfirmActions.svelte");

function scriptCode(source, functions) {
  const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
  const parsed = ts.createSourceFile("component.ts", script, ts.ScriptTarget.Latest, true);
  const statements = parsed.statements.filter((node) => functions
    ? ts.isFunctionDeclaration(node) && functions.includes(node.name?.text)
    : !ts.isImportDeclaration(node));
  return ts.transpileModule(statements.map((node) => node.getText(parsed)).join("\n"), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  }).outputText;
}

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function fixture(onconfirm, props = {}) {
  let cancels = 0;
  const context = vm.createContext({
    $props: () => ({ onconfirm, oncancel: () => cancels++, ...props }),
    $state: (value) => value,
    $bindable: (value) => value,
    tick: () => Promise.resolve(),
  });
  vm.runInContext(scriptCode(source), context);
  return {
    confirm: () => context.confirm(),
    cancel: () => context.cancel(),
    get: (expression) => vm.runInContext(expression, context),
    cancels: () => cancels,
  };
}

test("pending begins before the operation and blocks duplicate confirmation and cancellation", async () => {
  const operation = deferred();
  let calls = 0;
  const f = fixture(() => { calls++; return operation.promise; });
  const pending = f.confirm();
  assert.equal(f.get("busy"), true);
  assert.equal(calls, 0, "the pending state is flushed before invoking the operation");
  f.cancel();
  await f.confirm();
  assert.equal(calls, 1);
  assert.equal(f.cancels(), 0);
  assert.equal(f.get("busy"), true);
  operation.resolve();
  await pending;
  assert.equal(f.get("busy"), false);
  assert.equal(f.get("error"), "");
  f.cancel();
  assert.equal(f.cancels(), 1);
});

test("failure remains visible, releases busy state and allows retry", async () => {
  let calls = 0;
  const operation = deferred();
  const f = fixture(() => {
    if (++calls === 1) throw "磁盘忙";
    return operation.promise;
  });
  await f.confirm();
  assert.equal(f.get("busy"), false);
  assert.equal(f.get("error"), "删除失败：磁盘忙");
  const retry = f.confirm();
  assert.equal(f.get("error"), "");
  assert.equal(f.get("busy"), true);
  operation.resolve();
  await retry;
  assert.equal(calls, 2);
  assert.equal(f.get("busy"), false);
});

test("removal reports its own action label and Error message", async () => {
  const f = fixture(() => Promise.resolve(), { confirmLabel: "移除" });
  // Create the Error in the component's realm, as a real frontend callback would.
  f.get('onconfirm = async () => { throw new Error("无法移除"); }');
  await f.confirm();
  assert.equal(f.get("error"), "移除失败：无法移除");
});

test("the UI exposes a spinner, live status, inline errors and disabled actions", () => {
  assert.match(source, /<Loader2[^>]+animate-spin/);
  assert.match(source, /role="status"/);
  assert.match(source, /正在处理中/);
  assert.match(source, /role="alert"/);
  assert.equal((source.match(/disabled=\{busy\}/g) ?? []).length, 2);
  assert.match(source, /aria-busy=\{busy\}/);
});

test("all destructive confirmation modals await their action through the shared component", async () => {
  for (const page of ["Archive", "Material", "Clip", "Upload", "Room"]) {
    const text = await read(`src/page/${page}.svelte`);
    assert.match(text, /<DeleteConfirmActions[\s\S]*?onconfirm=/, page);
  }
  const room = await read("src/page/Room.svelte");
  assert.match(room, /bind:busy=\{deletingRoom\}/);
  assert.match(room, /if \(deleteModal\) \{\s*if \(deletingRoom\) return;/);
});

test("archive batch survives list refreshes and retries only unfinished deletions", async () => {
  const text = await read("src/page/Archive.svelte");
  const records = ["one", "two", "three"].map((live_id) => ({ live_id, room_id: "room", platform: "bilibili" }));
  const calls = [];
  let fail = true;
  const refresh = deferred();
  const context = vm.createContext({
    selectedArchives: new Set(records.map((record) => record.live_id)),
    filteredArchives: records,
    showDeleteConfirm: true,
    archiveToDelete: null,
    invoke: async (command, args) => {
      assert.equal(command, "delete_archive");
      calls.push(args.liveId);
      if (args.liveId === "two" && fail) throw new Error("busy");
      // A deletion event replaces the list while the request is awaited.
      context.filteredArchives = context.filteredArchives.filter((record) => record.live_id !== args.liveId);
    },
    loadArchives: () => refresh.promise,
  });
  vm.runInContext(scriptCode(text, ["deleteSelectedArchives"]), context);
  await assert.rejects(context.deleteSelectedArchives(), /busy/);
  assert.deepEqual([...context.selectedArchives], ["two", "three"]);
  assert.equal(context.showDeleteConfirm, true);
  fail = false;
  const retry = context.deleteSelectedArchives();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(calls, ["one", "two", "two", "three"]);
  assert.equal(context.showDeleteConfirm, true, "keep the modal until list refresh completes");
  refresh.resolve();
  await retry;
  assert.equal(context.showDeleteConfirm, false);
  assert.equal(context.selectedArchives.size, 0);
});

test("archive batch uses its initial targets even if a refresh replaces the entire visible page", async () => {
  const text = await read("src/page/Archive.svelte");
  const records = ["one", "two", "three"].map((live_id) => ({ live_id, room_id: "room", platform: "bilibili" }));
  const calls = [];
  const context = vm.createContext({
    selectedArchives: new Set(records.map((record) => record.live_id)),
    filteredArchives: records,
    showDeleteConfirm: true,
    archiveToDelete: null,
    invoke: async (_command, args) => {
      calls.push(args.liveId);
      context.filteredArchives = [];
    },
    loadArchives: async () => {},
  });
  vm.runInContext(scriptCode(text, ["deleteSelectedArchives"]), context);
  await context.deleteSelectedArchives();
  assert.deepEqual(calls, ["one", "two", "three"]);
  assert.equal(context.showDeleteConfirm, false);
});

test("material batch retains only failed and unattempted IDs for retry", async () => {
  const text = await read("src/page/Material.svelte");
  let closed = false;
  const context = vm.createContext({
    selectedVideos: new Set([1, 2, 3]),
    invoke: async (_command, { id }) => { if (id === 2) throw new Error("busy"); },
    loadVideos: async () => {},
    closeDeleteConfirm: () => { closed = true; },
  });
  vm.runInContext(scriptCode(text, ["deleteSelectedVideos"]), context);
  await assert.rejects(context.deleteSelectedVideos(), /busy/);
  assert.deepEqual([...context.selectedVideos], [2, 3]);
  assert.equal(closed, false);
});
