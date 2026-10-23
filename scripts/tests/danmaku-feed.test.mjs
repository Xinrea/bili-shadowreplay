import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function loadModule() {
  const source = await readFile(new URL("../../src/lib/danmaku-feed.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const module = new vm.SourceTextModule(outputText, { context: vm.createContext({}) });
  await module.link(() => {
    throw new Error("danmaku-feed.ts must be self-contained");
  });
  await module.evaluate();
  return module.namespace;
}

const playing = (now, more = {}) => ({ now, offset: 0, paused: false, atLiveEdge: false, ...more });
const plain = (step) => ({ clear: step.clear, live: step.live, from: step.from });

test("playing on hands over each stretch once; starting catches up", async () => {
  const { createDanmakuFeed } = await loadModule();
  const feed = createDanmakuFeed();
  assert.deepEqual(plain(feed.step(playing(60_000))), { clear: true, live: false, from: 40_000 });
  assert.deepEqual(plain(feed.step(playing(60_200))), { clear: false, live: false, from: 60_000 });
  // At 5× a step moves about a second: still playing on.
  assert.deepEqual(plain(feed.step(playing(61_200))), { clear: false, live: false, from: 60_200 });
});

test("a seek, a changed offset, moving back or far ahead start over, also while paused", async () => {
  const { createDanmakuFeed } = await loadModule();
  const feed = createDanmakuFeed();
  feed.step(playing(10_000));
  // Paused, nothing more is due.
  assert.deepEqual(plain(feed.step(playing(10_000, { paused: true }))), { clear: false, live: false, from: null });
  // A seek while paused: what is on screen there, at once.
  feed.reset();
  assert.deepEqual(plain(feed.step(playing(50_000, { paused: true }))), { clear: true, live: false, from: 30_000 });
  assert.deepEqual(plain(feed.step(playing(50_000, { paused: true }))), { clear: false, live: false, from: null });
  // A changed offset while paused.
  assert.deepEqual(plain(feed.step(playing(53_000, { paused: true, offset: 3000 }))), { clear: true, live: false, from: 33_000 });
  // Back, and far ahead, while playing.
  assert.deepEqual(plain(feed.step(playing(40_000, { offset: 3000 }))), { clear: true, live: false, from: 20_000 });
  assert.deepEqual(plain(feed.step(playing(42_500, { offset: 3000 }))), { clear: true, live: false, from: 22_500 });
  assert.deepEqual(plain(feed.step(playing(44_000, { offset: 3000 }))), { clear: false, live: false, from: 42_500 });
});

test("crossing the live edge starts over both ways", async () => {
  const { createDanmakuFeed } = await loadModule();
  const feed = createDanmakuFeed();
  feed.step(playing(10_000));
  feed.step(playing(10_200));
  // Caught up with live: recorded danmaku go, live ones come as they arrive.
  assert.deepEqual(plain(feed.step(playing(10_400, { atLiveEdge: true }))), { clear: true, live: true, from: null });
  assert.deepEqual(plain(feed.step(playing(10_600, { atLiveEdge: true }))), { clear: false, live: true, from: null });
  // Paused at the live edge, live danmaku still come.
  assert.deepEqual(plain(feed.step(playing(10_600, { atLiveEdge: true, paused: true }))), { clear: false, live: true, from: null });
  // Fallen behind: live danmaku go, recorded ones catch up.
  assert.deepEqual(plain(feed.step(playing(10_800))), { clear: true, live: false, from: -9_200 });
  assert.deepEqual(plain(feed.step(playing(11_000))), { clear: false, live: false, from: 10_800 });
});

test("paused behind the live edge, live danmaku are not drawn", async () => {
  const { createDanmakuFeed } = await loadModule();
  const feed = createDanmakuFeed();
  feed.step(playing(10_000));
  // Nothing recorded is due while paused, and live ones are not drawn either.
  assert.deepEqual(plain(feed.step(playing(10_000, { paused: true }))), { clear: false, live: false, from: null });
  assert.deepEqual(plain(feed.step(playing(10_200))), { clear: false, live: false, from: 10_000 });
});
