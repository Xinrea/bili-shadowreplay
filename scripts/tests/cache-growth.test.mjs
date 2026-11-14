import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../../src/lib/cache-growth.ts", import.meta.url), "utf8");
const module = new vm.SourceTextModule(ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText);
await module.link(() => { throw new Error("Unexpected cache growth dependency"); });
await module.evaluate();
const { createCacheGrowthEstimator } = module.namespace;

test("cache growth uses elapsed wall time and smooths batched segment writes", () => {
  const growth = createCacheGrowthEstimator();
  assert.equal(growth.sample(10_000, 0), null);
  assert.equal(growth.sample(14_000, 2000), 2000);
  assert.equal(growth.sample(14_000, 5000), 800);
  assert.equal(growth.sample(18_000, 10_000), 800);
});

test("idle recordings reach zero when old growth leaves the ten-second window", () => {
  const growth = createCacheGrowthEstimator();
  growth.sample(0, 0);
  growth.sample(8000, 2000);
  for (let time = 4000; time <= 10_000; time += 2000) growth.sample(8000, time);
  assert.equal(growth.sample(8000, 12_000), 0);
});

test("deletion resets the baseline without negative rates or rebound spikes", () => {
  const growth = createCacheGrowthEstimator();
  growth.sample(10_000, 0);
  growth.sample(14_000, 2000);
  assert.equal(growth.sample(2000, 4000), null);
  assert.equal(growth.sample(6000, 6000), 2000);
});

test("paused observation and explicit resets discard stale history", () => {
  const growth = createCacheGrowthEstimator();
  growth.sample(1000, 0);
  assert.equal(growth.sample(51_000, 60_000), null);
  assert.equal(growth.sample(55_000, 62_000), 2000);
  growth.reset();
  assert.equal(growth.sample(59_000, 64_000), null);
});

test("invalid samples and non-increasing timestamps cannot produce invalid speeds", () => {
  const growth = createCacheGrowthEstimator();
  for (const [size, time] of [[NaN, 0], [Infinity, 0], [-1, 0], [1000, NaN]]) {
    growth.sample(1000, 0);
    assert.equal(growth.sample(size, time), null);
    assert.equal(growth.sample(2000, 2000), null);
  }
  assert.equal(growth.sample(3000, 2000), null);
  assert.equal(growth.sample(4000, 1000), null);
  assert.equal(growth.sample(6000, 3000), 1000);
});
