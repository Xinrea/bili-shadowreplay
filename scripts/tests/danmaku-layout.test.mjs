import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function loadModule() {
  const source = await readFile(
    new URL("../../src/lib/danmaku-layout.ts", import.meta.url),
    "utf8",
  );
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: {
      target: ts.ScriptTarget.ES2022,
      module: ts.ModuleKind.ESNext,
    },
  });
  const context = vm.createContext({ setTimeout, clearTimeout });
  const module = new vm.SourceTextModule(outputText, { context });
  await module.link(() => {
    throw new Error("danmaku-layout.ts must be self-contained");
  });
  await module.evaluate();
  return module.namespace;
}

const player = { width: 1920, height: 1080 };

test("crossing time follows Bilibili's steps for the player's width", async () => {
  const api = await loadModule();
  assert.equal(api.baseDuration(100, 400), 4.5);
  assert.equal(api.baseDuration(0, 1920), 4.5);
  const fullHd = api.baseDuration(300, 1920);
  assert.ok(fullHd > 13 && fullHd < 13.2, fullHd);
  const window = api.baseDuration(300, 1280);
  assert.ok(window > 10 && window < 10.2, window);
  // From the first boundary on, the first step.
  const small = api.baseDuration(100, 558);
  assert.ok(small > 7 && small < 7.2, small);
  const wide = api.baseDuration(300, 3100);
  assert.ok(wide > 17 && wide < 17.2, wide);
  assert.ok(api.baseDuration(1500, 1920) > fullHd);
});

test("font size is the burned size, scaled to the player", async () => {
  const api = await loadModule();
  // The burned em of the default 36 at 1080p: close to Bilibili's 42.26 px.
  const fullHd = api.danmakuFontSize(36, 1080);
  assert.ok(Math.abs(fullHd - 40.91) < 0.01, fullHd);
  assert.ok(Math.abs(api.danmakuFontSize(36, 540) - fullHd / 2) < 1e-9);
  assert.ok(Math.abs(api.danmakuFontSize(48, 1080) - (fullHd * 48) / 36) < 1e-9);
});

test("messages enter and leave beyond the player", async () => {
  const api = await loadModule();
  const space = api.createDanmakuSpace();
  const flight = space.place(0, 200, 48, player);
  assert.equal(flight.top, 0);
  assert.equal(flight.entryX, 1971);
  assert.equal(flight.travel, 2221);
  const speed = 2120 / api.baseDuration(200, 1920);
  assert.equal(flight.duration, Math.ceil((2221 / speed) * 100) / 100);
});

test("rows fill from the top, 1 px apart, and a full player drops messages", async () => {
  const api = await loadModule();
  const space = api.createDanmakuSpace();
  const tops = [];
  for (let i = 0; i < 30; i++) {
    const flight = space.place(i * 0.1, 700, 47, player);
    if (flight) tops.push(flight.top);
  }
  // 22 rows of 47 + 1 px fit in 1081 px.
  assert.equal(tops.length, 22);
  assert.deepEqual(tops.slice(0, 3), [0, 48, 96]);
  assert.equal(space.place(3, 700, 47, player), null);
  assert.equal(space.place(3, 700, 47, player, true).top, 0);
  space.clear();
  assert.equal(space.place(3, 700, 47, player).top, 0);
});

test("a row is reused once the message ahead cannot be caught", async () => {
  const api = await loadModule();
  const close = api.createDanmakuSpace();
  close.place(0, 300, 47, player);
  assert.equal(close.place(0.5, 300, 47, player).top, 48);

  const apart = api.createDanmakuSpace();
  apart.place(0, 300, 47, player);
  assert.equal(apart.place(3, 300, 47, player).top, 0);
  // A longer, faster message must wait longer behind a short one.
  const faster = api.createDanmakuSpace();
  faster.place(0, 100, 47, player);
  assert.equal(faster.place(3, 1200, 47, player).top, 48);
  // Messages that have left free their rows.
  assert.equal(faster.place(30, 1200, 47, player).top, 0);
});

test("bursts are spread out and their oldest dropped; a quiet stream is drawn at once", async () => {
  const api = await loadModule();
  let now = 0;
  let pending = [];
  const timer = {
    set(callback, delay) {
      const id = { at: now + delay, callback };
      pending.push(id);
      return id;
    },
    clear(id) {
      pending = pending.filter((other) => other !== id);
    },
  };
  function run(until) {
    for (;;) {
      pending.sort((a, b) => a.at - b.at);
      const next = pending[0];
      if (!next || next.at > until) break;
      pending.shift();
      now = next.at;
      next.callback();
    }
    now = until;
  }
  const drawn = [];
  const pacer = api.createDanmakuPacer((item) => drawn.push([now, item]), timer);
  for (let i = 0; i < 40; i++) pacer.push(i);
  run(10_000);
  // 10 timers and 21 waiting; the 9 oldest waiting were dropped.
  assert.equal(drawn.length, 31);
  assert.deepEqual(drawn.slice(0, 11).map(([, item]) => item), [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 19]);
  assert.deepEqual(drawn.slice(0, 11).map(([at]) => at), [0, 100, 200, 300, 400, 500, 600, 700, 800, 900, 900]);

  drawn.length = 0;
  for (const at of [20_000, 20_500, 22_000]) {
    run(at);
    pacer.push(at);
  }
  run(30_000);
  assert.deepEqual(drawn, [[20_000, 20_000], [20_500, 20_500], [22_000, 22_000]]);

  for (let i = 0; i < 5; i++) pacer.push(i);
  pacer.clear();
  assert.equal(pending.length, 0);
});
