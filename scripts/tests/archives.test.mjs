import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function loadModule(invoke = async () => []) {
  const source = await readFile(new URL("../../src/lib/archives.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const context = vm.createContext({});
  const invoker = new vm.SyntheticModule(["invoke", "get_static_url"], function () {
    this.setExport("invoke", invoke);
    this.setExport("get_static_url", async (base, path) => `/${base}/${path}`);
  }, { context });
  const module = new vm.SourceTextModule(outputText, { context });
  await module.link(() => invoker);
  await module.evaluate();
  return module.namespace;
}

const room = { platform: "bilibili", room_id: "123" };
const record = (index, props = {}) => ({
  ...room, live_id: String(index).padStart(4, "0"), parent_id: "parent",
  title: `record ${index}`, created_at: new Date(index * 1000).toISOString(),
  length: index, size: index * 1000, cover: null, ...props,
});

test("room archives load beyond 100 entries and hydrate covers without changing API rows", async () => {
  const rows = Array.from({ length: 237 }, (_, index) => record(index));
  const requests = [];
  const api = await loadModule(async (command, args) => {
    assert.equal(command, "get_archives");
    requests.push({ ...args });
    return rows.slice(args.offset, args.offset + args.limit);
  });
  const result = await api.loadRoomArchives(room);
  assert.deepEqual(requests, [0, 100, 200].map((offset) => ({ platform: "bilibili", roomId: "123", offset, limit: 100 })));
  assert.equal(result.length, 237);
  assert.equal(result[236].cover, "/cache/bilibili/123/0236/cover.jpg");
  assert.equal(rows[236].cover, null);
});

test("closing during a page request prevents later requests and cover work", async () => {
  let active = true;
  let calls = 0;
  const api = await loadModule(async () => {
    calls++;
    active = false;
    return Array.from({ length: 100 }, (_, index) => record(index));
  });
  const result = await api.loadRoomArchives(room, () => active);
  assert.equal(calls, 1);
  assert.equal(result.length, 0);
});

test("a server repeating the same full page cannot create duplicates or an endless fetch", async () => {
  let calls = 0;
  const api = await loadModule(async () => {
    calls++;
    return Array.from({ length: 100 }, (_, index) => record(index));
  });
  assert.equal((await api.loadRoomArchives(room)).length, 100);
  assert.equal(calls, 2);
});

test("sorting works across the full archive collection with stable ties and no mutation", async () => {
  const api = await loadModule();
  const rows = [record(3, { title: "b" }), record(1, { title: "A" }), record(2, { title: "a" })];
  const ids = (items) => Array.from(items, (item) => item.live_id);
  assert.deepEqual(ids(api.sortArchives(rows, "title", "asc")), ["0001", "0002", "0003"]);
  assert.deepEqual(ids(api.sortArchives(rows, "length", "desc")), ["0003", "0002", "0001"]);
  assert.deepEqual(ids(api.sortArchives(rows, "created_at", "asc")), ["0001", "0002", "0003"]);
  assert.deepEqual(ids(rows), ["0003", "0001", "0002"]);
  assert.notEqual(api.archiveKey(rows[0]), api.archiveKey({ ...rows[0], platform: "douyin" }));
});

test("room totals span every archive page and include the ongoing recording only once", async () => {
  const rows = Array.from({ length: 237 }, (_, index) => record(index));
  const api = await loadModule(async (_command, args) => rows.slice(args.offset, args.offset + args.limit));
  const archives = await api.loadRoomArchives(room);
  const current = { ...rows[236], length: rows[236].length + 60, size: rows[236].size + 5_000_000 };
  const totals = api.summarizeRoomArchives(archives, room, current);
  const length = rows.reduce((sum, item) => sum + item.length, 0);
  const size = rows.reduce((sum, item) => sum + item.size, 0);
  assert.deepEqual({ ...totals }, { count: 237, length: length + 60, size: size + 5_000_000 });
  assert.deepEqual({ ...api.summarizeRoomArchives([...archives, archives[236]], room, rows[236]) }, { count: 237, length, size });
  assert.equal(archives[236].length, rows[236].length);
});

test("room totals scope by platform and room, recover current recordings missing from an older snapshot, and handle empty rooms", async () => {
  const api = await loadModule();
  const rows = [record(1), record(2, { platform: "douyin" }), record(3, { room_id: "other" })];
  assert.deepEqual({ ...api.summarizeRoomArchives(rows, room, record(4)) }, { count: 2, length: 5, size: 5_000 });
  assert.deepEqual({ ...api.summarizeRoomArchives(rows, room, record(4, { platform: "douyin" })) }, { count: 1, length: 1, size: 1_000 });
  assert.deepEqual({ ...api.summarizeRoomArchives([], room) }, { count: 0, length: 0, size: 0 });
  assert.deepEqual({ ...api.summarizeRoomArchives([record(1, { length: NaN, size: -1 })], room) }, { count: 1, length: 0, size: 0 });
});
