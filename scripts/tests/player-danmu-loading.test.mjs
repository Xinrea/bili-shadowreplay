import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const read = path => readFile(new URL(`../../src/${path}`, import.meta.url), "utf8");
async function load(path) {
  const source = await read(path);
  const context = { exports: {}, setTimeout, clearTimeout, AbortController };
  vm.runInNewContext(ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText, context);
  return context.exports;
}
const historyApi = await load("lib/danmu-history.ts");
const batchApi = await load("lib/live-preview-perf.ts");
const subscriptions = await load("lib/event-subscription.ts");
const source = await read("lib/components/Player.svelte");
const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
const parsed = ts.createSourceFile("Player.ts", script, ts.ScriptTarget.Latest, true);
const functions = name => parsed.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === name);
const code = ts.transpileModule(functions("startDanmuUpdates").getText(parsed), {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText;
const plain = value => JSON.parse(JSON.stringify(value));
const settle = () => new Promise(resolve => setImmediate(resolve));
const event = (ts, content, rest = {}) => ({ ts, type: "danmu", content, ...rest });
function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function fixture(loadHistory) {
  const calls = [], drawn = [], requests = [];
  let callback, live = true;
  const clock = {
    timers: new Map(), id: 0, now: () => 0,
    schedule(fn) { const id = ++this.id; this.timers.set(id, fn); return id; },
    cancel(id) { this.timers.delete(id); },
    flush() { const fns = [...this.timers.values()]; this.timers.clear(); fns.forEach(fn => fn()); },
  };
  const state = {
    destroyed: false, danmu_records: [], global_offset: 0,
    platform: "bilibili", room_id: "room", live_id: "live",
    danmuHistoryState: "idle", reloadDanmuHistory: undefined,
    stopDanmu: undefined, cancelDanmuBatch: undefined,
    danmuSubscription: new AbortController(),
    ...historyApi, ...subscriptions,
    DANMU_FLUSH_INTERVAL_MS: 500,
    createDanmuBatcher: flush => batchApi.createDanmuBatcher(flush, 500, clock),
    listen: async (name, cb, scope) => {
      calls.push("subscribe"); callback = cb;
      assert.equal(name, "danmu"); assert.deepEqual(plain(scope), { platform: "bilibili", room: "room" });
      return () => calls.push("unsubscribe");
    },
    invoke: (command, args) => {
      calls.push("history"); requests.push(plain(args));
      assert.equal(command, "get_danmu_record");
      return loadHistory();
    },
    log: { error: (...args) => calls.push(["error", ...args]) },
  };
  vm.runInContext(code, vm.createContext(state));
  state.startDanmuUpdates(() => live, entry => drawn.push(entry), () => calls.push("history-published"));
  return {
    state, calls, drawn, requests, clock,
    emit: entry => callback({ payload: entry }),
    live: value => { live = value; },
    dispose() { state.destroyed = true; state.danmuSubscription.abort(); state.cancelDanmuBatch?.(); },
  };
}

test("live reception starts before history and does not wait for a missing initial offset", async () => {
  const pending = deferred();
  const f = fixture(() => pending.promise);
  assert.deepEqual(f.calls, ["subscribe", "history"]);
  f.emit(event(100, "live"));
  f.clock.flush();
  assert.deepEqual(plain(f.state.danmu_records), [event(100, "live")]);
  assert.equal(f.drawn.length, 1);
  assert.equal(f.state.danmuHistoryState, "loading");
  f.dispose();
  pending.resolve([]);
  await settle();
});

test("late history merges flushed and still-pending live batches without dropping or duplicating overlap", async () => {
  const pending = deferred();
  const f = fixture(() => pending.promise);
  f.emit(event(20, "overlap")); f.clock.flush();
  f.emit(event(30, "new"));
  pending.resolve([event(10, "history"), event(20, "overlap")]);
  await settle();
  assert.deepEqual(plain(f.state.danmu_records), [event(10, "history"), event(20, "overlap"), event(30, "new")]);
  assert.equal(f.state.danmuHistoryState, "loaded");
  assert.equal(f.clock.timers.size, 0);
  assert.equal(f.drawn.length, 2, "history must not re-emit live overlay events");
  f.dispose();
});

test("history failure preserves live events and retry does not create a second subscription", async () => {
  const first = deferred(), retry = deferred();
  let attempt = 0;
  const f = fixture(() => (++attempt === 1 ? first.promise : retry.promise));
  f.emit(event(20, "live")); f.clock.flush();
  first.reject(new Error("backend unavailable"));
  await settle();
  assert.equal(f.state.danmuHistoryState, "error");
  assert.equal(f.state.danmu_records.length, 1);
  const next = f.state.reloadDanmuHistory();
  void f.state.reloadDanmuHistory(); // duplicate click while pending
  f.emit(event(30, "still live"));
  retry.resolve([event(10, "history"), event(20, "live")]);
  await next;
  assert.equal(attempt, 2);
  assert.equal(f.calls.filter(c => c === "subscribe").length, 1);
  assert.deepEqual(plain(f.state.danmu_records).map(e => e.content), ["history", "live", "still live"]);
  assert.deepEqual(f.requests[1], { platform: "bilibili", roomId: "room", liveId: "live" });
  f.dispose();
});

test("unmount ignores late success or failure and discards pending live batches", async () => {
  for (const reject of [false, true]) {
    const pending = deferred();
    const f = fixture(() => pending.promise);
    f.emit(event(10, "pending"));
    f.dispose();
    if (reject) pending.reject(new Error("late failure"));
    else pending.resolve([event(1, "late history")]);
    await settle();
    assert.equal(f.state.danmu_records.length, 0);
    assert.equal(f.calls.includes("history-published"), false);
    assert.equal(f.calls.some(c => Array.isArray(c) && c[0] === "error"), false);
    assert.equal(f.calls.filter(c => c === "unsubscribe").length, 1);
    assert.equal(f.clock.timers.size, 0);
  }
});

test("VOD ignores unrelated live events and a later live transition starts accepting them", async () => {
  const f = fixture(async () => [event(1, "archive")]);
  f.live(false); f.emit(event(2, "wrong recording"));
  await settle();
  assert.deepEqual(plain(f.state.danmu_records), [event(1, "archive")]);
  f.live(true); f.state.global_offset = 1;
  f.emit(event(500, "before recording")); f.emit(event(1001, "current"));
  f.clock.flush();
  assert.deepEqual(plain(f.state.danmu_records).map(e => e.content), ["archive", "current"]);
  f.dispose();
});

test("history overlap keeps duplicate multiplicity and distinct sender or paid-message fields", () => {
  const a = event(1, "repeat", { user_name: "alice" });
  const b = event(1, "repeat", { user_name: "bob" });
  const sc = event(2, "paid", { type: "super_chat", price: 30, sc_duration: 60 });
  assert.deepEqual(plain(historyApi.mergeDanmuHistory([a, a, sc], [a, a, a, b, { ...sc, price: 50 }])),
    [a, a, a, b, sc, { ...sc, price: 50 }]);
});

test("player initialization does not await history or sending accounts before creating the overlay", () => {
  const init = functions("init");
  const awaits = [];
  function visit(node) {
    if (node !== init && ts.isFunctionLike(node)) return;
    if (ts.isAwaitExpression(node)) awaits.push(node.getText(parsed));
    ts.forEachChild(node, visit);
  }
  visit(init);
  assert.ok(awaits.length > 0);
  assert.ok(!awaits.some(text => /get_danmu_record|get_accounts/.test(text)));
  const text = init.getText(parsed);
  assert.ok(text.indexOf("startDanmuUpdates(") > text.indexOf("floatingOverlay = {"));
  assert.match(source, /danmuHistoryState === "loading"/);
  assert.match(source, /danmuHistoryState === "error"/);
  assert.match(source, /reloadDanmuHistory\?\.\(\)/);
});
