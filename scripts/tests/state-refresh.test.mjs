import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

const settle = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
};

async function fixture({ delayedSubscriptions = false } = {}) {
  const timeouts = new Map();
  const intervals = new Map();
  const handlers = new Map();
  const subscriptions = [];
  const removed = [];
  const documentHandlers = new Map();
  const windowHandlers = new Map();
  let id = 0;
  const document = {
    visibilityState: "visible",
    addEventListener: (name, cb) => documentHandlers.set(name, cb),
    removeEventListener: (name) => documentHandlers.delete(name),
  };
  const context = vm.createContext({
    console, document,
    window: {
      addEventListener: (name, cb) => windowHandlers.set(name, cb),
      removeEventListener: (name) => windowHandlers.delete(name),
    },
    setTimeout: (cb) => { timeouts.set(++id, cb); return id; },
    clearTimeout: (id) => timeouts.delete(id),
    setInterval: (cb) => { intervals.set(++id, cb); return id; },
    clearInterval: (id) => intervals.delete(id),
  });
  const invoker = new vm.SyntheticModule(["listen"], function () {
    this.setExport("listen", async (name, cb) => {
      handlers.set(name, cb);
      const ready = deferred();
      subscriptions.push(ready);
      if (delayedSubscriptions) await ready.promise;
      return () => { removed.push(name); handlers.delete(name); };
    });
  }, { context });
  const source = await readFile(new URL("../../src/lib/state-refresh.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, { compilerOptions: {
    target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext,
  } });
  const module = new vm.SourceTextModule(outputText, { context });
  await module.link(() => invoker);
  await module.evaluate();
  return {
    api: module.namespace, timeouts, intervals, handlers, subscriptions, removed,
    document, documentHandlers, windowHandlers,
    async flushTimers() {
      const callbacks = [...timeouts.values()];
      timeouts.clear();
      callbacks.forEach((cb) => cb());
      await settle();
    },
  };
}

test("a burst of invalidations causes one snapshot", async () => {
  const f = await fixture();
  let calls = 0;
  const refresh = f.api.createStateRefresh(async () => { calls++; });
  for (let i = 0; i < 100; i++) refresh.schedule();
  assert.equal(f.timeouts.size, 1);
  await f.flushTimers();
  assert.equal(calls, 1);
});

test("changes during a request produce a trailing snapshot without overlap", async () => {
  const f = await fixture();
  const first = deferred();
  let calls = 0;
  let active = 0;
  let maxActive = 0;
  const refresh = f.api.createStateRefresh(async () => {
    calls++;
    active++;
    maxActive = Math.max(active, maxActive);
    if (calls === 1) await first.promise;
    active--;
  });
  const initial = refresh.run();
  await settle();
  refresh.schedule();
  refresh.schedule();
  const manual = refresh.run();
  assert.equal(calls, 1);
  first.resolve();
  await Promise.all([initial, manual]);
  assert.equal(calls, 2);
  assert.equal(maxActive, 1);
});

test("a failed fetch does not stop queued changes or later refreshes", async () => {
  const f = await fixture();
  const first = deferred();
  let calls = 0;
  const errors = [];
  const refresh = f.api.createStateRefresh(async () => {
    calls++;
    if (calls === 1) { await first.promise; throw new Error("offline"); }
  }, 200, (error) => errors.push(error));
  const pending = refresh.run();
  await settle();
  refresh.schedule();
  first.resolve();
  await pending;
  assert.equal(calls, 2);
  assert.equal(errors.length, 1);
  await refresh.run();
  assert.equal(calls, 3);
});

test("initial snapshot waits for every subscription", async () => {
  const f = await fixture({ delayedSubscriptions: true });
  let calls = 0;
  const refresh = f.api.createStateRefresh(async () => { calls++; });
  const stop = f.api.watchStateChanges(refresh, ["videos-changed", "tasks-changed"]);
  await settle();
  assert.equal(calls, 0);
  // Events during setup are covered by the snapshot after all subscriptions.
  f.handlers.get("videos-changed")();
  f.handlers.get("state-resync")();
  await f.flushTimers();
  assert.equal(calls, 0);
  f.subscriptions[0].resolve();
  await settle();
  assert.equal(calls, 0);
  f.subscriptions.slice(1).forEach((subscription) => subscription.resolve());
  await settle();
  assert.equal(calls, 1);
  stop();
});

test("reconnect, visibility restoration, online and fallback repair missed events", async () => {
  const f = await fixture();
  let calls = 0;
  const refresh = f.api.createStateRefresh(async () => { calls++; });
  const stop = f.api.watchStateChanges(refresh, ["videos-changed"]);
  await settle();
  f.handlers.get("state-resync")();
  await settle();
  assert.equal(calls, 2);
  f.document.visibilityState = "hidden";
  f.handlers.get("videos-changed")();
  f.documentHandlers.get("visibilitychange")();
  [...f.intervals.values()][0]();
  await f.flushTimers();
  assert.equal(calls, 2);
  f.document.visibilityState = "visible";
  f.documentHandlers.get("visibilitychange")();
  await settle();
  assert.equal(calls, 3);
  f.windowHandlers.get("online")();
  await settle();
  assert.equal(calls, 4);
  [...f.intervals.values()][0]();
  await f.flushTimers();
  assert.equal(calls, 5);
  stop();
  assert.equal(f.handlers.size, 0);
  assert.equal(f.documentHandlers.size, 0);
  assert.equal(f.windowHandlers.size, 0);
  assert.equal(f.intervals.size, 0);
});

test("unmount during async subscription cleans up late listeners and skips fetch", async () => {
  const f = await fixture({ delayedSubscriptions: true });
  let calls = 0;
  const refresh = f.api.createStateRefresh(async () => { calls++; });
  const stop = f.api.watchStateChanges(refresh, ["videos-changed"]);
  stop();
  f.subscriptions.forEach((subscription) => subscription.resolve());
  await settle();
  assert.equal(calls, 0);
  assert.equal(f.handlers.size, 0);
  assert.equal(f.removed.length, 2);
  assert.equal(f.intervals.size, 0);
});

test("unmount discards timers and trailing refreshes while a request finishes", async () => {
  const f = await fixture();
  const first = deferred();
  let calls = 0;
  const refresh = f.api.createStateRefresh(async () => { calls++; await first.promise; });
  refresh.schedule();
  assert.equal(f.timeouts.size, 1);
  const pending = refresh.run();
  await settle();
  refresh.schedule();
  refresh.stop();
  first.resolve();
  await pending;
  await refresh.run();
  assert.equal(calls, 1);
  assert.equal(f.timeouts.size, 0);
});

async function invokerFixture({ tauri = false, invokeRoom = async () => null, nativeReady, ackRoom = async () => ({ ok: true }) } = {}) {
  const retryTimers = new Map();
  let timerId = 0;
  const socketHandlers = new Map();
  const tauriListeners = new Map();
  const nativeOptions = new Map();
  const roomCommands = [];
  const sent = [];
  const socket = {
    connected: true,
    on: (name, cb) => socketHandlers.set(name, cb),
    onAny: (cb) => socketHandlers.set("*", cb),
    emit: (name, payload) => sent.push({ name, payload: JSON.parse(JSON.stringify(payload)) }),
    timeout: (ms) => {
      assert.equal(ms, 5_000);
      return { emitWithAck: async (name, payload) => {
        sent.push({ name, payload: JSON.parse(JSON.stringify(payload)) });
        return ackRoom(payload);
      } };
    },
    disconnect() {},
  };
  const context = vm.createContext({
    console: { log() {}, info() {}, error() {} }, TextEncoder,
    setTimeout: (callback, ms) => { retryTimers.set(++timerId, { callback, ms }); return timerId; },
    clearTimeout: (id) => retryTimers.delete(id),
    window: tauri ? { __TAURI_INTERNALS__: {} } : {},
    localStorage: { getItem: () => "" },
    fetch: async () => ({ ok: true, json: async () => ({ code: 0, data: null }) }),
  });
  const dependencies = {
    "@tauri-apps/api/core": { invoke: async (name, args) => {
      if (name === "set_event_rooms") {
        roomCommands.push(JSON.parse(JSON.stringify(args.rooms)));
        return invokeRoom(args.rooms);
      }
      return null;
    } },
    "@tauri-apps/api/webviewWindow": { getCurrentWebviewWindow: () => ({ label: "preview" }) },
    "@tauri-apps/plugin-http": { fetch: async () => ({}) },
    "@tauri-apps/api/event": { listen: async (name, cb, options) => {
      tauriListeners.set(name, cb);
      nativeOptions.set(name, options);
      if (nativeReady) await nativeReady.promise;
      return () => tauriListeners.delete(name);
    } },
    "@tauri-apps/plugin-shell": { open: async () => {} },
    "@tauri-apps/plugin-deep-link": { onOpenUrl: async () => {} },
    "socket.io-client": { io: () => socket },
  };
  const source = await readFile(new URL("../../src/lib/invoker.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, { compilerOptions: {
    target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext,
  } });
  const eventSource = await readFile(new URL("../../src/lib/events.ts", import.meta.url), "utf8");
  const eventOutput = ts.transpileModule(eventSource, { compilerOptions: {
    target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext,
  } }).outputText;
  const eventModule = new vm.SourceTextModule(eventOutput, { context });
  const module = new vm.SourceTextModule(outputText, { context });
  await module.link((name) => {
    if (name === "./events") return eventModule;
    assert.ok(dependencies[name], `Unexpected dependency: ${name}`);
    return new vm.SyntheticModule(Object.keys(dependencies[name]), function () {
      for (const [key, value] of Object.entries(dependencies[name])) this.setExport(key, value);
    }, { context });
  });
  await module.evaluate();
  return { api: module.namespace, socketHandlers, tauriListeners, nativeOptions, sent, socket, roomCommands, retryTimers,
    async retryRooms() {
      assert.equal(retryTimers.size, 1);
      const [[id, { callback }]] = retryTimers;
      retryTimers.delete(id);
      callback();
      await settle();
    },
  };
}

test("Web events reach the same listen API and each connect triggers reconciliation", async () => {
  const f = await invokerFixture();
  let resyncs = 0;
  const stop = await f.api.listen("state-resync", () => { resyncs++; });
  f.socketHandlers.get("connect")();
  f.socketHandlers.get("connect")();
  assert.equal(resyncs, 2);
  stop();
  for (const event of ["videos-changed", "tasks-changed", "recorders-changed", "records-changed", "state-resync"]) {
    let calls = 0;
    const unlisten = await f.api.listen(event, ({ payload }) => {
      assert.equal(payload, null);
      calls++;
    });
    f.socketHandlers.get("*")(event, null);
    assert.equal(calls, 1);
    unlisten();
    f.socketHandlers.get("*")(event, null);
    assert.equal(calls, 1);
  }
});

test("desktop state subscriptions use native Tauri events", async () => {
  const f = await invokerFixture({ tauri: true });
  let calls = 0;
  const unlisten = await f.api.listen("videos-changed", () => { calls++; });
  assert.equal(f.socketHandlers.size, 0);
  f.tauriListeners.get("videos-changed")({ payload: null });
  assert.equal(calls, 1);
  unlisten();
  assert.equal(f.tauriListeners.size, 0);
});

test("one dispatch path handles scoped progress, legacy progress and new event types", async () => {
  const f = await invokerFixture();
  const received = [];
  await f.api.listen("progress-update:task", ({ payload }) => received.push(payload.content));
  await f.api.listen("progress-finished:task", ({ payload }) => received.push(payload.success));
  await f.api.listen("custom-event", ({ payload }) => received.push(payload));
  const receive = f.socketHandlers.get("*");
  receive("progress-update:task", { id: "task", content: "encoding" });
  receive("progress-update", { id: "task", content: "legacy" });
  receive("progress-finished:task", { id: "task", success: true });
  receive("progress-update:another-task", { id: "another-task", content: "wrong task" });
  receive("custom-event", 42);
  assert.deepEqual(received, ["encoding", "legacy", true, 42]);
});

test("failing listeners and independent duplicate registrations do not disrupt dispatch", async () => {
  const f = await invokerFixture();
  let calls = 0;
  await f.api.listen("custom-event", () => { throw new Error("broken listener"); });
  const callback = () => { calls++; };
  const first = await f.api.listen("custom-event", callback);
  const second = await f.api.listen("custom-event", callback);
  f.socketHandlers.get("*")("custom-event", {});
  assert.equal(calls, 2);
  first();
  first();
  f.socketHandlers.get("*")("custom-event", {});
  assert.equal(calls, 3);
  second();
  f.socketHandlers.get("*")("custom-event", {});
  assert.equal(calls, 3);
});

const topicName = ({ platform, room }) => `danmu:${platform}:${Buffer.from(room).toString("hex")}`;

test("room subscriptions are shared locally and route by platform as well as room", async () => {
  const f = await invokerFixture();
  const room = { platform: "youtube", room: "@same:中文" };
  const other = { ...room, platform: "twitch" };
  let firstCalls = 0;
  let secondCalls = 0;
  let otherCalls = 0;
  const first = await f.api.listen("danmu", () => { firstCalls++; }, room);
  const second = await f.api.listen("danmu", () => { secondCalls++; }, room);
  assert.deepEqual(f.sent, [{ name: "event-rooms", payload: [room] }]);
  const stopOther = await f.api.listen("danmu", () => { otherCalls++; }, other);
  assert.deepEqual(f.sent.at(-1).payload, [room, other]);
  f.socketHandlers.get("*")(topicName(room), { ...room, content: "hello" });
  assert.equal(firstCalls, 1);
  assert.equal(secondCalls, 1);
  assert.equal(otherCalls, 0);
  const sends = f.sent.length;
  first();
  first();
  assert.equal(f.sent.length, sends);
  f.socketHandlers.get("*")(topicName(room), {});
  assert.equal(firstCalls, 1);
  assert.equal(secondCalls, 2);
  second();
  assert.deepEqual(f.sent.at(-1).payload, [other]);
  stopOther();
  await settle();
  assert.deepEqual(f.sent.at(-1).payload, []);
  await assert.rejects(f.api.listen("danmu", () => {}), /require a platform and room/);
});

test("reconnection sends only current room subscriptions, including changes while offline", async () => {
  const f = await invokerFixture();
  const room = { platform: "bilibili", room: "1" };
  const stop = await f.api.listen("danmu", () => {}, room);
  f.socket.connected = false;
  const sent = f.sent.length;
  stop();
  const current = { ...room, room: "2" };
  let ready = false;
  const pending = f.api.listen("danmu", () => {}, current).then((stop) => { ready = true; return stop; });
  await settle();
  assert.equal(ready, false);
  assert.equal(f.sent.length, sent);
  f.socket.connected = true;
  f.socketHandlers.get("connect")();
  await pending;
  assert.deepEqual(f.sent.at(-1), { name: "event-rooms", payload: [current] });
  f.socketHandlers.get("connect")();
  assert.deepEqual(f.sent.at(-1).payload, [current]);
});

test("Web preview listeners wait for acknowledgement and share pending readiness", async () => {
  const ack = deferred();
  const f = await invokerFixture({ ackRoom: () => ack.promise });
  const room = { platform: "bilibili", room: "1" };
  let ready = 0;
  const first = f.api.listen("danmu", () => {}, room).then((stop) => { ready++; return stop; });
  const second = f.api.listen("danmu", () => {}, room).then((stop) => { ready++; return stop; });
  await settle();
  assert.equal(ready, 0);
  assert.equal(f.sent.length, 1);
  ack.resolve({ ok: true });
  const stops = await Promise.all([first, second]);
  assert.equal(ready, 2);
  stops.forEach((stop) => stop());
});

test("Web server rejection removes only the rejected listener and reconciles the set", async () => {
  const f = await invokerFixture({ ackRoom: async (rooms) => rooms.some((room) => room.room === "invalid")
    ? { ok: false, error: "Invalid room topic" } : { ok: true } });
  const valid = { platform: "bilibili", room: "1" };
  let calls = 0;
  const stop = await f.api.listen("danmu", () => { calls++; }, valid);
  const invalid = { ...valid, room: "invalid" };
  await assert.rejects(f.api.listen("danmu", () => { calls++; }, invalid), /Invalid room topic/);
  assert.deepEqual(f.sent.at(-1).payload, [valid]);
  f.socketHandlers.get("*")(topicName(invalid), {});
  assert.equal(calls, 0);
  f.socketHandlers.get("*")(topicName(valid), {});
  assert.equal(calls, 1);
  stop();
});

test("Web ack timeout fails setup and permits a later retry", async () => {
  let fail = true;
  const f = await invokerFixture({ ackRoom: async () => {
    if (fail) { fail = false; throw new Error("operation has timed out"); }
    return { ok: true };
  } });
  const room = { platform: "bilibili", room: "1" };
  let calls = 0;
  await assert.rejects(f.api.listen("danmu", () => { calls++; }, room), /timed out/);
  f.socketHandlers.get("*")(topicName(room), {});
  assert.equal(calls, 0);
  const stop = await f.api.listen("danmu", () => { calls++; }, room);
  f.socketHandlers.get("*")(topicName(room), {});
  assert.equal(calls, 1);
  stop();
});

test("closing a preview while offline cancels setup and reconnect sends no stale topic", async () => {
  const f = await invokerFixture();
  f.socket.connected = false;
  const controller = new AbortController();
  const room = { platform: "bilibili", room: "1" };
  let calls = 0;
  const pending = f.api.listen("danmu", () => { calls++; }, room, controller.signal);
  controller.abort();
  await assert.rejects(pending, /aborted/);
  f.socketHandlers.get("*")(topicName(room), {});
  assert.equal(calls, 0);
  assert.equal(f.sent.length, 0);
  f.socket.connected = true;
  f.socketHandlers.get("connect")();
  await settle();
  assert.deepEqual(f.sent.at(-1).payload, []);
});

test("cancelling pending ACK removes the topic even if the ACK arrives later", async () => {
  const held = deferred();
  const f = await invokerFixture({ ackRoom: (rooms) => rooms.length ? held.promise : Promise.resolve({ ok: true }) });
  const controller = new AbortController();
  const room = { platform: "bilibili", room: "1" };
  let calls = 0;
  const pending = f.api.listen("danmu", () => { calls++; }, room, controller.signal);
  controller.abort();
  await assert.rejects(pending, /aborted/);
  held.resolve({ ok: true });
  await settle();
  assert.deepEqual(f.sent.at(-1).payload, []);
  f.socketHandlers.get("*")(topicName(room), {});
  assert.equal(calls, 0);
});

for (const oldFailure of ["before reconnect", "before new ACK", "after new ACK"]) {
  test(`reconnect preserves pending shared listeners when obsolete ACK fails ${oldFailure}`, async () => {
    const oldAck = deferred();
    const newAck = deferred();
    let attempts = 0;
    const f = await invokerFixture({ ackRoom: (rooms) => {
      if (!rooms.length) return Promise.resolve({ ok: true });
      return ++attempts === 1 ? oldAck.promise : newAck.promise;
    } });
    const room = { platform: "bilibili", room: "1" };
    let calls = 0, ready = 0;
    const first = f.api.listen("danmu", () => { calls++; }, room).then((stop) => { ready++; return stop; });
    const second = f.api.listen("danmu", () => { calls++; }, room).then((stop) => { ready++; return stop; });
    f.socket.connected = false;
    f.socketHandlers.get("disconnect")();
    if (oldFailure === "before reconnect") {
      oldAck.reject(new Error("socket has been disconnected"));
      await settle();
      assert.equal(ready, 0);
    }
    f.socket.connected = true;
    f.socketHandlers.get("connect")();
    assert.deepEqual(f.sent.map(({ payload }) => payload), [[room], [room]]);
    if (oldFailure === "before new ACK") {
      oldAck.reject(new Error("obsolete ACK timeout"));
      await settle();
      assert.equal(ready, 0);
    }
    newAck.resolve({ ok: true });
    const [stop, stopOther] = await Promise.all([first, second]);
    if (oldFailure === "after new ACK") oldAck.reject(new Error("obsolete ACK timeout"));
    await settle();
    assert.equal(f.sent.length, 2);
    f.socketHandlers.get("*")(topicName(room), {});
    assert.equal(calls, 2);
    stop();
    assert.equal(f.sent.length, 2);
    stopOther();
    await settle();
    assert.deepEqual(f.sent.at(-1).payload, []);
  });
}

test("serial room updates use the latest set and omit cancelled queued topics", async () => {
  const ack = deferred();
  let attempts = 0;
  const f = await invokerFixture({ ackRoom: () => ++attempts === 1 ? ack.promise : Promise.resolve({ ok: true }) });
  const room = { platform: "bilibili", room: "1" };
  const other = { ...room, room: "2" };
  const first = f.api.listen("danmu", () => {}, room);
  const controller = new AbortController();
  const second = f.api.listen("danmu", () => {}, other, controller.signal);
  assert.equal(f.sent.length, 1);
  controller.abort();
  await assert.rejects(second, /aborted/);
  ack.resolve({ ok: true });
  const stop = await first;
  assert.deepEqual(f.sent.map(({ payload }) => payload), [[room], [room]]);
  stop();
});

test("multiple reconnects ignore obsolete success and error ACKs", async () => {
  const acks = [deferred(), deferred(), deferred()];
  let attempts = 0;
  const f = await invokerFixture({ ackRoom: (rooms) => rooms.length ? acks[attempts++].promise : Promise.resolve({ ok: true }) });
  const room = { platform: "bilibili", room: "1" };
  let ready = false;
  const pending = f.api.listen("danmu", () => {}, room).then((stop) => { ready = true; return stop; });
  for (let i = 0; i < 2; i++) {
    f.socket.connected = false;
    f.socketHandlers.get("disconnect")();
    f.socket.connected = true;
    f.socketHandlers.get("connect")();
  }
  acks[0].resolve({ ok: true });
  acks[1].reject(new Error("obsolete failure"));
  await settle();
  assert.equal(ready, false);
  acks[2].resolve({ ok: true });
  const stop = await pending;
  assert.equal(f.sent.length, 3);
  stop();
});

for (const failure of ["timeout", "rejection"]) {
  test(`established shared listeners recover after reconnect ACK ${failure}`, async () => {
    let attempts = 0, serverRooms = [];
    const f = await invokerFixture({ ackRoom: async (rooms) => {
      if (++attempts === 2) {
        if (failure === "timeout") throw new Error("ACK timed out");
        return { ok: false, error: "Temporary subscription failure" };
      }
      serverRooms = JSON.parse(JSON.stringify(rooms));
      return { ok: true };
    } });
    const room = { platform: "bilibili", room: "1" };
    let calls = 0;
    const first = await f.api.listen("danmu", () => { calls++; }, room);
    const second = await f.api.listen("danmu", () => { calls++; }, room);
    f.socket.connected = false;
    f.socketHandlers.get("disconnect")();
    serverRooms = [];
    f.socket.connected = true;
    f.socketHandlers.get("connect")();
    await settle();
    assert.deepEqual(serverRooms, []);
    assert.equal(f.retryTimers.size, 1, "A failed reconnect must schedule room recovery");
    assert.equal([...f.retryTimers.values()][0].ms, 1_000);
    await f.retryRooms();
    assert.deepEqual(serverRooms, [room]);
    assert.deepEqual(f.sent.map(({ payload }) => payload), [[room], [room], [room]]);
    assert.equal(f.retryTimers.size, 0);
    f.socketHandlers.get("*")(topicName(serverRooms[0]), {});
    assert.equal(calls, 2);
    first(); second();
    await settle();
    assert.deepEqual(serverRooms, []);
  });
}

test("room recovery backs off to 30 seconds and disconnect cancels the retry", async () => {
  let fail = false;
  const f = await invokerFixture({ ackRoom: async () => {
    if (fail) throw new Error("ACK timeout");
    return { ok: true };
  } });
  const room = { platform: "bilibili", room: "1" };
  const stop = await f.api.listen("danmu", () => {}, room);
  fail = true;
  f.socketHandlers.get("connect")();
  await settle();
  for (const delay of [1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000]) {
    assert.equal([...f.retryTimers.values()][0].ms, delay);
    await f.retryRooms();
  }
  f.socket.connected = false;
  f.socketHandlers.get("disconnect")();
  assert.equal(f.retryTimers.size, 0);
  const sent = f.sent.length;
  fail = false;
  f.socket.connected = true;
  f.socketHandlers.get("connect")();
  await settle();
  assert.equal(f.sent.length, sent + 1);
  assert.equal(f.retryTimers.size, 0);
  // A new failure after recovery starts at the initial delay again.
  fail = true;
  f.socketHandlers.get("connect")();
  await settle();
  assert.equal([...f.retryTimers.values()][0].ms, 1_000);
  stop();
  await settle();
  assert.equal(f.retryTimers.size, 0);
});

test("unmounting the last established listener cancels reconnect recovery", async () => {
  let attempts = 0;
  const f = await invokerFixture({ ackRoom: async (rooms) =>
    ++attempts > 1 && rooms.length ? { ok: false, error: "Unavailable" } : { ok: true } });
  const room = { platform: "bilibili", room: "1" };
  const first = await f.api.listen("danmu", () => {}, room);
  const second = await f.api.listen("danmu", () => {}, room);
  f.socketHandlers.get("connect")();
  await settle();
  first();
  assert.equal(f.retryTimers.size, 1);
  second();
  await settle();
  assert.equal(f.retryTimers.size, 0);
  assert.deepEqual(f.sent.at(-1).payload, []);
});

test("subscription changes cancel scheduled recovery and synchronize the latest set", async () => {
  let fail = false;
  const f = await invokerFixture({ ackRoom: async () => fail
    ? { ok: false, error: "Unavailable" } : { ok: true } });
  const room = { platform: "bilibili", room: "1" };
  const stop = await f.api.listen("danmu", () => {}, room);
  fail = true;
  f.socketHandlers.get("connect")();
  await settle();
  assert.equal(f.retryTimers.size, 1);
  fail = false;
  const other = { ...room, room: "2" };
  const stopOther = await f.api.listen("danmu", () => {}, other);
  assert.equal(f.retryTimers.size, 0);
  assert.deepEqual(f.sent.at(-1).payload, [room, other]);
  stop(); stopOther();
  await settle();
  assert.deepEqual(f.sent.at(-1).payload, []);
});

test("a late recovery failure after unmount only reconciles the empty set", async () => {
  const recovery = deferred();
  let attempts = 0;
  const f = await invokerFixture({ ackRoom: async (rooms) => {
    if (!rooms.length) return { ok: true };
    if (++attempts === 2) throw new Error("Reconnect ACK timeout");
    if (attempts === 3) return recovery.promise;
    return { ok: true };
  } });
  const room = { platform: "bilibili", room: "1" };
  let calls = 0;
  const stop = await f.api.listen("danmu", () => { calls++; }, room);
  f.socketHandlers.get("connect")();
  await settle();
  await f.retryRooms();
  stop();
  assert.equal(f.sent.length, 3);
  recovery.reject(new Error("Late ACK timeout"));
  await settle();
  assert.deepEqual(f.sent.at(-1).payload, []);
  assert.equal(f.retryTimers.size, 0);
  f.socketHandlers.get("*")(topicName(room), {});
  assert.equal(calls, 0);
});

test("preview topic limit also applies offline without rejecting shared listeners", async () => {
  const f = await invokerFixture();
  const stops = [];
  for (let i = 0; i < 32; i++) {
    stops.push(await f.api.listen("danmu", () => {}, { platform: "bilibili", room: String(i) }));
  }
  f.socket.connected = false;
  const sent = f.sent.length;
  const extra = { platform: "bilibili", room: "extra" };
  let calls = 0;
  await assert.rejects(f.api.listen("danmu", () => { calls++; }, extra), /At most 32/);
  assert.equal(f.sent.length, sent);
  f.socketHandlers.get("*")(topicName(extra), {});
  assert.equal(calls, 0);
  const shared = await f.api.listen("danmu", () => {}, { platform: "bilibili", room: "0" });
  shared();
  stops.forEach((stop) => stop());
  f.socket.connected = true;
  f.socketHandlers.get("connect")();
  await settle();
  assert.deepEqual(f.sent.at(-1).payload, []);
});

test("desktop room events target the current window and commands preserve subscribe/unsubscribe order", async () => {
  const held = deferred();
  const f = await invokerFixture({ tauri: true, invokeRoom: async (rooms) => {
    if (rooms.length && rooms[0].room === "1") await held.promise;
  } });
  const room = { platform: "bilibili", room: "1" };
  const pending = f.api.listen("danmu", () => {}, room);
  await settle();
  assert.equal(f.tauriListeners.size, 1);
  assert.deepEqual(JSON.parse(JSON.stringify(f.nativeOptions.get(topicName(room)))), {
    target: { kind: "WebviewWindow", label: "preview" },
  });
  const other = { ...room, room: "2" };
  const otherPending = f.api.listen("danmu", () => {}, other);
  held.resolve();
  const [stop, stopOther] = await Promise.all([pending, otherPending]);
  stop();
  stopOther();
  await settle();
  assert.deepEqual(f.roomCommands, [[], [room], [room, other], [other], []]);
  assert.equal(f.tauriListeners.size, 0);
});

test("desktop subscription failure removes the listener and permits later subscriptions", async () => {
  let fail = true;
  const f = await invokerFixture({ tauri: true, invokeRoom: async (rooms) => {
    if (rooms.length && fail) { fail = false; throw new Error("window unavailable"); }
  } });
  const room = { platform: "youtube", room: "@test" };
  await assert.rejects(f.api.listen("danmu", () => {}, room), /window unavailable/);
  await settle();
  assert.equal(f.tauriListeners.size, 0);
  const stop = await f.api.listen("danmu", () => {}, room);
  stop();
  await settle();
  assert.deepEqual(f.roomCommands.at(-1), []);
});

test("native listener setup finishes before desktop preview forwarding begins", async () => {
  const nativeReady = deferred();
  const f = await invokerFixture({ tauri: true, nativeReady });
  const room = { platform: "bilibili", room: "1" };
  const pending = f.api.listen("danmu", () => {}, room);
  await settle();
  assert.deepEqual(f.roomCommands, [[]]);
  nativeReady.resolve();
  const stop = await pending;
  assert.deepEqual(f.roomCommands, [[], [room]]);
  stop();
  await settle();
  assert.deepEqual(f.roomCommands.at(-1), []);
});
