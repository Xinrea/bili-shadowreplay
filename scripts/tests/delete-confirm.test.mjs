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

test("project batch deletes only confirmed targets even if selection and results change", async () => {
  const text = await read("src/page/Clip.svelte");
  const projects = [1, 2, 3].map((id) => ({ id, title: `Project ${id}` }));
  const calls = [];
  const context = vm.createContext({
    projects,
    selected: projects.slice(0, 2),
    selectedProjects: new Set([1, 2]),
    projectsToDelete: null,
    invoke: async (command, { id }) => {
      assert.equal(command, "delete_project");
      calls.push(id);
      // A background refresh can replace the visible results during deletion.
      context.projects = [];
    },
    stateRefresh: { run: async () => {} },
  });
  vm.runInContext(scriptCode(text, ["confirmDelete", "deleteProjects"]), context);
  context.confirmDelete();
  assert.deepEqual(Array.from(context.projectsToDelete, ({ id }) => id), [1, 2]);
  context.selected = [projects[2]];
  context.selectedProjects.clear();
  context.selectedProjects.add(3);
  await context.deleteProjects();
  assert.deepEqual(calls, [1, 2]);
  assert.deepEqual([...context.selectedProjects], [3]);
  assert.equal(context.projectsToDelete, null);
});

test("project batch refreshes after partial failure and retries only unfinished deletions", async () => {
  const text = await read("src/page/Clip.svelte");
  const projects = [1, 2, 3, 4].map((id) => ({ id, title: `Project ${id}` }));
  const calls = [];
  let fail = true;
  let refreshes = 0;
  const refresh = deferred();
  const context = vm.createContext({
    projects,
    selected: projects.slice(0, 3),
    selectedProjects: new Set([1, 2, 3]),
    projectsToDelete: null,
    invoke: async (_command, { id }) => {
      calls.push(id);
      if (id === 2 && fail) throw new Error("busy");
    },
    stateRefresh: { run: async () => {
      refreshes++;
      if (!fail) await refresh.promise;
    } },
  });
  vm.runInContext(scriptCode(text, ["confirmDelete", "deleteProjects"]), context);
  context.confirmDelete();
  await assert.rejects(context.deleteProjects(), /busy/);
  assert.equal(refreshes, 1);
  assert.deepEqual(Array.from(context.projectsToDelete, ({ id }) => id), [2, 3]);
  assert.deepEqual(Array.from(context.projects, ({ id }) => id), [2, 3, 4]);
  assert.deepEqual([...context.selectedProjects], [2, 3]);

  fail = false;
  const retry = context.deleteProjects();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(calls, [1, 2, 2, 3]);
  assert.notEqual(context.projectsToDelete, null, "keep the modal until refresh completes");
  refresh.resolve();
  await retry;
  assert.equal(refreshes, 2);
  assert.equal(context.projectsToDelete, null);
  assert.deepEqual(Array.from(context.projects, ({ id }) => id), [4]);
  assert.equal(context.selectedProjects.size, 0);
});

test("single project deletion leaves other selected projects intact", async () => {
  const text = await read("src/page/Clip.svelte");
  const projects = [1, 2].map((id) => ({ id, title: `Project ${id}` }));
  const calls = [];
  const context = vm.createContext({
    projects,
    selected: projects,
    selectedProjects: new Set([1, 2]),
    projectsToDelete: null,
    invoke: async (_command, { id }) => { calls.push(id); },
    stateRefresh: { run: async () => {} },
  });
  vm.runInContext(scriptCode(text, ["confirmDelete", "deleteProjects"]), context);
  context.confirmDelete(projects[0]);
  await context.deleteProjects();
  assert.deepEqual(calls, [1]);
  assert.deepEqual([...context.selectedProjects], [2]);
  assert.deepEqual(Array.from(context.projects, ({ id }) => id), [2]);
});

const uploadSource = await read("src/page/Upload.svelte");
function uploadFixture(records, invoke = async () => {}, refresh = async () => {}) {
  const context = vm.createContext({
    uploads: [...records],
    selected: [...records],
    selectable: records.filter((upload) => !["exporting", "posting"].includes(upload.status)),
    selectedUploads: new Set(records.map((upload) => upload.id)),
    allSelected: false,
    uploadsToDelete: null,
    editingId: null,
    isUploadActive: (upload) => ["exporting", "posting"].includes(upload.status),
    invoke: async (command, args) => command === "get_uploads" ? [...context.uploads] : invoke(command, args),
    stateRefresh: { run: refresh },
  });
  vm.runInContext(scriptCode(uploadSource, [
    "toggleUploadSelection", "selectAllUploads", "confirmDelete", "deleteUploads",
  ]), context);
  return context;
}

const uploadRecords = () => [1, 2, 3].map((id) => ({ id, title: `Upload ${id}`, status: "draft" }));

test("upload selection and confirmation exclude active records", () => {
  const records = uploadRecords();
  records[1].status = "exporting";
  records[2].status = "posting";
  const f = uploadFixture(records);
  f.selectedUploads.clear();
  f.selectAllUploads();
  assert.deepEqual([...f.selectedUploads], [1]);
  f.toggleUploadSelection(records[1]);
  f.toggleUploadSelection(records[2]);
  assert.deepEqual([...f.selectedUploads], [1]);
  f.confirmDelete(records[1]);
  assert.equal(f.uploadsToDelete, null);
  f.confirmDelete();
  assert.deepEqual(Array.from(f.uploadsToDelete, ({ id }) => id), [1]);
  f.allSelected = true;
  f.selectAllUploads();
  assert.equal(f.selectedUploads.size, 0);
});

test("upload deletion uses confirmed targets despite a selection or list change", async () => {
  const records = uploadRecords();
  const calls = [];
  const f = uploadFixture(records, async (command, { id }) => {
    assert.equal(command, "delete_upload");
    calls.push(id);
    f.uploads = [];
  });
  f.selected = records.slice(0, 2);
  f.confirmDelete();
  f.selected = [records[2]];
  f.selectedUploads = new Set([3]);
  await f.deleteUploads();
  assert.deepEqual(calls, [1, 2]);
  assert.deepEqual([...f.selectedUploads], [3]);
  assert.equal(f.uploadsToDelete, null);
});

test("upload partial failure refreshes and retries only unfinished records", async () => {
  const calls = [];
  let fail = true;
  let refreshes = 0;
  const refresh = deferred();
  const f = uploadFixture(uploadRecords(), async (_command, { id }) => {
    calls.push(id);
    if (id === 2 && fail) throw new Error("busy");
  }, async () => {
    refreshes++;
    if (!fail) await refresh.promise;
  });
  f.editingId = 1;
  f.confirmDelete();
  await assert.rejects(f.deleteUploads(), /busy/);
  assert.equal(refreshes, 1);
  assert.equal(f.editingId, null);
  assert.deepEqual([...f.selectedUploads], [2, 3]);
  assert.deepEqual(Array.from(f.uploads, ({ id }) => id), [2, 3]);
  assert.deepEqual(Array.from(f.uploadsToDelete, ({ id }) => id), [2, 3]);
  fail = false;
  const retry = f.deleteUploads();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(calls, [1, 2, 2, 3]);
  assert.notEqual(f.uploadsToDelete, null, "wait for reconciliation before closing");
  refresh.resolve();
  await retry;
  assert.equal(refreshes, 2);
  assert.equal(f.uploadsToDelete, null);
  assert.equal(f.selectedUploads.size, 0);
});

test("upload deletion rejects records that became active after confirmation", async () => {
  const calls = [];
  let refreshed = false;
  const f = uploadFixture(uploadRecords(), async (_command, { id }) => calls.push(id), async () => { refreshed = true; });
  f.confirmDelete();
  f.uploads[1] = { ...f.uploads[1], status: "posting" };
  await assert.rejects(f.deleteUploads(), /正在投稿，无法删除/);
  assert.deepEqual(calls, [1]);
  assert.equal(refreshed, true);
  assert.deepEqual(Array.from(f.uploadsToDelete, ({ id }) => id), [2, 3]);
});

test("single upload deletion keeps unrelated selection and editor open", async () => {
  const records = uploadRecords();
  const calls = [];
  const f = uploadFixture(records, async (_command, { id }) => calls.push(id));
  f.editingId = 2;
  f.confirmDelete(records[0]);
  await f.deleteUploads();
  assert.deepEqual(calls, [1]);
  assert.deepEqual([...f.selectedUploads], [2, 3]);
  assert.equal(f.editingId, 2);
});

for (const timing of ["before confirmation is submitted", "during the delete request"]) {
  test(`upload batch continues when another client deletes a target ${timing}`, async () => {
    const records = uploadRecords();
    const server = new Map(records.map((upload) => [upload.id, upload]));
    const calls = [];
    let reads = 0;
    const f = uploadFixture(records);
    f.invoke = async (command, args) => {
      if (command === "get_uploads") {
        reads++;
        return [...server.values()];
      }
      assert.equal(command, "delete_upload");
      calls.push(args.id);
      if (args.id === 2) server.delete(2);
      // The backend reports the same rejection for missing and active records.
      if (!server.delete(args.id)) throw new Error("投稿进行中，暂时无法删除");
    };
    f.stateRefresh.run = async () => { f.uploads = [...server.values()]; };
    f.confirmDelete();
    if (timing === "before confirmation is submitted") {
      server.delete(2);
      await f.stateRefresh.run();
    }
    await f.deleteUploads();
    assert.deepEqual(calls, [1, 2, 3]);
    assert.equal(reads, 1);
    assert.equal(server.size, 0);
    assert.equal(f.uploadsToDelete, null);
    assert.equal(f.selectedUploads.size, 0);
  });
}

test("upload batch preserves uncertain targets when verification fails and recovers on retry", async () => {
  const records = uploadRecords();
  const server = new Map(records.map((upload) => [upload.id, upload]));
  const calls = [];
  let readFails = true;
  const f = uploadFixture(records);
  f.invoke = async (command, args) => {
    if (command === "get_uploads") {
      if (readFails) throw new Error("snapshot unavailable");
      return [...server.values()];
    }
    assert.equal(command, "delete_upload");
    calls.push(args.id);
    if (args.id === 2 && server.delete(2)) throw new Error("response lost");
    if (!server.delete(args.id)) throw new Error("投稿进行中，暂时无法删除");
  };
  f.confirmDelete();
  await assert.rejects(f.deleteUploads(), /response lost/);
  assert.deepEqual(calls, [1, 2]);
  assert.deepEqual(Array.from(f.uploadsToDelete, ({ id }) => id), [2, 3]);
  assert.deepEqual([...f.selectedUploads], [2, 3]);
  readFails = false;
  await f.deleteUploads();
  assert.deepEqual(calls, [1, 2, 2, 3]);
  assert.equal(server.size, 0);
  assert.equal(f.uploadsToDelete, null);
});
