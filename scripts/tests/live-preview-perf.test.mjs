import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function loadModule() {
  const source = await readFile(
    new URL("../../src/lib/live-preview-perf.ts", import.meta.url),
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
    throw new Error("live-preview-perf.ts must be self-contained");
  });
  await module.evaluate();
  return module.namespace;
}

test("danmu batcher flushes many incoming comments as a single array update", async () => {
  const api = await loadModule();
  const flushed = [];
  const batcher = api.createDanmuBatcher((batch) => flushed.push(batch), 50);

  for (let i = 0; i < 40; i++) {
    batcher.push({ ts: i, content: `d${i}` });
  }
  assert.equal(flushed.length, 0);

  batcher.flushNow();
  assert.equal(flushed.length, 1);
  assert.equal(flushed[0].length, 40);
  assert.equal(flushed[0][39].content, "d39");
  batcher.cancel();
});

test("throttled runner coalesces a burst into one immediate call and one trailing call", async () => {
  const api = await loadModule();
  const calls = [];
  const scheduled = [];
  let now = 1000;
  const run = api.createThrottledRunner(
    () => calls.push(now),
    1000,
    {
      now: () => now,
      schedule: (cb, delay) => {
        scheduled.push({ cb, delay });
        return scheduled.length;
      },
      cancel: () => {
        scheduled.length = 0;
      },
    },
  );

  run();
  run();
  run();
  assert.deepEqual(calls, [1000]);
  assert.equal(scheduled.length, 1);
  assert.equal(scheduled[0].delay, 1000);

  now = 2000;
  scheduled[0].cb();
  assert.deepEqual(calls, [1000, 2000]);
});

test("heat bar layout uses neighbor times instead of scanning the whole array", async () => {
  const api = await loadModule();
  const points = Array.from({ length: 4000 }, (_, i) => ({
    time: i * 5,
    count: i === 2000 ? 90 : 10,
    level: i === 2000 ? "core" : "normal",
  }));
  const started = Date.now();
  const bars = api.layoutHeatBars(points, 0, 20000, 90);
  const elapsed = Date.now() - started;

  assert.equal(bars.length, 4000);
  assert.ok(elapsed < 200, `layout took ${elapsed}ms`);
  assert.equal(bars[0].left, 0);
  assert.ok(bars[2000].height > bars[0].height);
  assert.equal(bars[2000].level, "core");
});

test("downsample keeps the hottest bars under a fixed DOM budget", async () => {
  const api = await loadModule();
  const points = Array.from({ length: 2000 }, (_, i) => ({
    time: i,
    count: i === 1234 ? 80 : 1,
    level: i === 1234 ? "core" : "normal",
  }));
  const sampled = api.downsampleHeatPoints(points, 200);
  assert.equal(sampled.length, 200);
  assert.ok(sampled.some((point) => point.count === 80 && point.level === "core"));
});

test("danmu statistic buckets use seconds, not milliseconds", async () => {
  const api = await loadModule();
  const points = api.bucketDanmuStatistics(
    [
      { ts: 10_000, content: "a" },
      { ts: 10_003, content: "b" },
      { ts: 16_000, content: "c" },
    ],
    { localOffsetSec: 0, globalOffsetSec: 0, gapSec: 5 },
  );
  assert.equal(points.length, 2);
  assert.equal(points[0].count, 2);
  assert.equal(points[0].ts, 10_000);
  assert.equal(points[1].count, 1);
  assert.equal(points[1].ts, 15_000);
});

test("virtual list keeps a stable total height while the window slides", async () => {
  const api = await loadModule();
  const first = api.virtualListWindow(0, 480, 28, 2000, 12);
  const scrolled = api.virtualListWindow(336, 480, 28, 2000, 12);

  assert.equal(first.totalHeight, 2000 * 28);
  assert.equal(scrolled.totalHeight, first.totalHeight);
  assert.equal(first.start, 0);
  assert.equal(scrolled.start, 0);
  assert.equal(scrolled.offsetY, 0);

  const further = api.virtualListWindow(840, 480, 28, 2000, 12);
  assert.equal(further.totalHeight, first.totalHeight);
  assert.ok(further.start > 0);
  assert.equal(further.offsetY, further.start * 28);
});
