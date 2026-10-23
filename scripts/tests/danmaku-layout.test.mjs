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
  space.clear();
  assert.equal(space.place(3, 700, 47, player).top, 0);
});

/**
 * Pairs of placed messages whose boxes ever intersect on screen, as CSS moves
 * them, by more than 1 px: the duration is rounded up to 0.01 s, so CSS moves
 * a hair slower than the layout reckons (as in Bilibili's).
 */
function intersections(placed) {
  const found = [];
  const x = (item, t) => item.flight.entryX - (item.flight.travel * (t - item.at)) / item.flight.duration;
  for (let i = 0; i < placed.length; i++) {
    for (let j = i + 1; j < placed.length; j++) {
      const [a, b] = [placed[i], placed[j]];
      if (a.flight.top >= b.flight.top + b.height || b.flight.top >= a.flight.top + a.height) continue;
      const from = Math.max(a.at, b.at);
      const to = Math.min(a.at + a.flight.duration, b.at + b.flight.duration);
      for (let t = from; t <= to; t += 0.05) {
        if (x(a, t) < x(b, t) + b.width - 1 && x(b, t) < x(a, t) + a.width - 1) {
          found.push([i, j, Math.round(t * 100) / 100]);
          break;
        }
      }
    }
  }
  return found;
}

test("on a full player a super chat finds room later, and nothing crosses it", async () => {
  const api = await loadModule();
  const space = api.createDanmakuSpace();
  const placed = [];
  const place = (at, width, height) => {
    const flight = space.place(at, width, height, player);
    if (flight) placed.push({ at, width, height, flight });
    return flight;
  };
  for (let i = 0; i < 22; i++) place(i * 0.1, 700, 47);
  // Taller than a row and wider, so faster: no room yet.
  let at = 2.2;
  while (!place(at, 1200, 110)) {
    place(at + 0.05, at * 10 % 2 < 1 ? 300 : 80, 47);
    at += 0.1;
    assert.ok(at < 20);
  }
  for (let i = 0; i < 40; i++) place(at + 0.1 + i * 0.1, i % 2 ? 300 : 80, 47);
  assert.deepEqual(intersections(placed), []);
});

test("a forced super chat flies over the top without taking up space", async () => {
  const api = await loadModule();
  const space = api.createDanmakuSpace();
  // Rows just entered, one after another, fill the player's entry.
  for (let i = 0; i < 22; i++) assert.ok(space.place(0, 700, 47, player));
  assert.equal(space.place(0, 420, 110, player), null);
  assert.equal(space.place(0, 420, 110, player, true).top, 0);
  // The rows are still all taken by the messages under it, not by it.
  assert.equal(space.place(0, 420, 110, player), null);
});

test("messages of mixed sizes never cross", async () => {
  const api = await loadModule();
  // A fixed pseudo-random stream: rows, emoticons and super chat cards.
  let seed = 7;
  const random = () => (seed = (seed * 48271) % 2147483647) / 2147483647;
  for (let run = 0; run < 20; run++) {
    const space = api.createDanmakuSpace();
    const placed = [];
    let at = 0;
    for (let i = 0; i < 150; i++) {
      at += random() * 0.3;
      const kind = random();
      const [width, height] =
        kind < 0.7 ? [40 + random() * 700, 47] : kind < 0.85 ? [48 + random() * 100, 48] : [200 + random() * 220, 60 + random() * 80];
      const flight = space.place(at, width, height, player);
      if (flight) placed.push({ at, width, height, flight });
    }
    assert.deepEqual(intersections(placed), [], `run ${run}`);
  }
});

test("a caught-up super chat without room at its time keeps out of the others' way", async () => {
  const api = await loadModule();
  const container = () => player;
  const rows = Array.from({ length: 140 }, (_, i) => [i * 0.1, 360 + ((i * 37) % 320)]);
  // Caught up at t=14: in one frame, each at its own time, in order.
  const catchUp = (withCard) => {
    const space = api.createDanmakuSpace();
    const clock = () => 14;
    // Due with the 22nd row, when rows fill all but 57 px.
    const card = api.createPlacer(space, clock, container, { at: rows[21][0], keep: true });
    const tops = [];
    for (const [i, [at, width]] of rows.entries()) {
      if (withCard && i === 21) assert.equal(card(420, 110), null);
      const flight = api.createPlacer(space, clock, container, { at })(width, 47);
      tops.push(flight && Math.round(flight.top));
    }
    return { space, card, tops };
  };
  const without = catchUp(false);
  const withCard = catchUp(true);
  assert.deepEqual(withCard.tops, without.tops);
  // Next frame its wait since its own time is long over: it flies over the
  // top from when the wait ended, without taking up space, even with room.
  const flight = withCard.card(420, 110);
  assert.equal(flight.top, 0);
  assert.ok(Math.abs(flight.elapsed - (14 - rows[21][0] - 5)) < 1e-9, flight.elapsed);
  const next = (space) => api.createPlacer(space, () => 14, container, { at: 14 })(420, 110);
  assert.deepEqual(next(withCard.space), next(without.space));
});

test("a super chat waits for room from its own time", async () => {
  const api = await loadModule();
  // A full player: room only when forced, over the top.
  const calls = [];
  const space = {
    place(at, width, height, container, force = false) {
      calls.push([at, force]);
      return force ? { top: 0, entryX: 1971, travel: 2441, duration: 13.67 } : null;
    },
  };
  let now = 12;
  const card = api.createPlacer(space, () => now, () => player, { at: 10, keep: true });
  // First at its own time only, then entering now, forced 5 s after its time.
  assert.equal(card(420, 110), null);
  now = 14.9;
  assert.equal(card(420, 110), null);
  now = 15;
  assert.equal(card(420, 110).top, 0);
  assert.deepEqual(calls, [[10, false], [14.9, false], [15, true]]);
  // A live one waits from its first try.
  calls.length = 0;
  const live = api.createPlacer(space, () => now, () => player, { keep: true });
  assert.equal(live(420, 110), null);
  now = 19.9;
  assert.equal(live(420, 110), null);
  now = 20;
  assert.equal(live(420, 110).elapsed, 0);
  assert.deepEqual(calls, [[15, false], [19.9, false], [20, true]]);
  // Caught up long after its time: as far into the flight over the top as
  // the wait's end is ago, which the overlay drops once the flight is over.
  calls.length = 0;
  now = 40;
  const late = api.createPlacer(space, () => now, () => player, { at: 12, keep: true });
  assert.equal(late(420, 110), null);
  now = 40.02;
  const over = late(420, 110);
  assert.ok(Math.abs(over.elapsed - 23.02) < 1e-9, over.elapsed);
  assert.ok(over.elapsed >= over.duration);
  assert.deepEqual(calls, [[12, false], [17, true]]);
  // Others are placed at their time or dropped.
  calls.length = 0;
  assert.equal(api.createPlacer(space, () => now, () => player, { at: 18 })(400, 47), null);
  assert.deepEqual(calls, [[18, false]]);
});

test("the queue runs on any clock", async () => {
  const api = await loadModule();
  const burst = api.paceTimes(Array.from({ length: 40 }, () => 1000));
  // 10 timers and 21 waiting; the 9 oldest waiting were dropped.
  assert.equal(burst.filter((time) => time !== null).length, 31);
  assert.deepEqual(burst.slice(0, 11), [1000, 1100, 1200, 1300, 1400, 1500, 1600, 1700, 1800, 1900, null]);
  assert.equal(burst[19], 1900);
  // Unsorted input keeps its order; a quiet stream is drawn on time.
  assert.deepEqual(api.paceTimes([22_000, 20_000, 20_500]), [22_000, 20_000, 20_500]);
  assert.deepEqual(api.paceTimes([]), []);
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
