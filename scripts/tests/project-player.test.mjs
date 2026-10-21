import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function loadModule() {
  const source = await readFile(new URL("../../src/lib/project-player.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const context = vm.createContext({
    EventTarget,
    Event,
    performance,
    setTimeout,
    clearTimeout,
    requestAnimationFrame: () => 0,
    cancelAnimationFrame: () => {},
  });
  const module = new vm.SourceTextModule(outputText, { context });
  await module.link(() => {
    throw new Error("The player must have no runtime dependencies");
  });
  await module.evaluate();
  return module.namespace;
}

const { ProjectPlayer } = await loadModule();

/** A <video> whose seeks land when the test says. */
class FakeVideo extends EventTarget {
  constructor() {
    super();
    this.dataset = {};
    this.readyState = 0;
    this.seeking = false;
    this.paused = true;
    this.seeks = [];
    this.position = 0;
  }
  get currentTime() {
    return this.position;
  }
  set currentTime(value) {
    this.position = value;
    this.seeking = true;
    this.seeks.push(value);
  }
  set src(value) {
    this.source = value;
    this.readyState = 0;
  }
  get src() {
    return this.source;
  }
  removeAttribute() {}
  load() {}
  pause() {
    this.paused = true;
  }
  async play() {
    this.paused = false;
  }
  /** Finish loading the file. */
  loaded() {
    this.readyState = 4;
    this.dispatchEvent(new Event("loadedmetadata"));
    this.dispatchEvent(new Event("loadeddata"));
  }
  /** Land the seek going on. */
  land() {
    this.seeking = false;
    this.dispatchEvent(new Event("seeked"));
  }
}

function player() {
  const [first, second] = [new FakeVideo(), new FakeVideo()];
  const instance = new ProjectPlayer(first, second);
  instance.setSegments([{ clipId: "a", url: "a.mp4", start: 100, end: 400, offset: 0 }]);
  const events = [];
  for (const type of ["seeking", "seeked"]) instance.addEventListener(type, () => events.push(type));
  first.loaded();
  first.land();
  events.length = 0;
  first.seeks.length = 0;
  return { instance, video: first, events };
}

test("seeks asked while one goes on wait for it, and only the last is made", () => {
  const { instance, video, events } = player();
  instance.seek(10);
  assert.deepEqual(video.seeks, [110]);
  // While it decodes, the pointer moves on twice.
  instance.seek(20);
  instance.seek(30);
  assert.deepEqual(video.seeks, [110]);
  assert.equal(instance.seeking, true);
  video.land();
  // Straight on to the last, without a "seeked" in between.
  assert.deepEqual(video.seeks, [110, 130]);
  assert.equal(instance.seeking, true);
  video.land();
  assert.equal(instance.seeking, false);
  assert.deepEqual(events, ["seeking", "seeked"]);
  assert.equal(instance.currentTime, 30);
});

test("a seek asked back to where the one going on lands needs no other", () => {
  const { instance, video, events } = player();
  instance.seek(10);
  instance.seek(20);
  instance.seek(10);
  video.land();
  assert.deepEqual(video.seeks, [110]);
  assert.equal(instance.seeking, false);
  assert.deepEqual(events, ["seeking", "seeked"]);
});

test("an element that fails while seeking does not stay seeking", () => {
  const { instance, video } = player();
  instance.seek(10);
  instance.seek(20);
  video.seeking = false;
  video.dispatchEvent(new Event("error"));
  assert.deepEqual(video.seeks, [110, 120]);
  video.land();
  assert.equal(instance.seeking, false);
});

test("leaving the clips drops the seek waiting", () => {
  const { instance, video } = player();
  instance.setSegments([]);
  instance.seek(10);
  assert.equal(instance.seeking, false);
  assert.deepEqual(video.seeks, []);
});

test("the standby element preloading does not hold the player seeking", () => {
  const [active, standby] = [new FakeVideo(), new FakeVideo()];
  const instance = new ProjectPlayer(active, standby);
  instance.setSegments([
    { clipId: "a", url: "a.mp4", start: 100, end: 110, offset: 0 },
    { clipId: "b", url: "b.mp4", start: 5, end: 15, offset: 10 },
  ]);
  active.loaded();
  active.land();
  // The standby goes to the next clip's in point, and is still seeking there.
  standby.loaded();
  assert.deepEqual(standby.seeks, [5]);
  const events = [];
  for (const type of ["seeking", "seeked"]) instance.addEventListener(type, () => events.push(type));
  instance.seek(1);
  active.land();
  // Done with the seek asked, though the standby still seeks on.
  assert.equal(instance.seeking, false);
  assert.deepEqual(events, ["seeking", "seeked"]);
  standby.land();
  assert.equal(instance.seeking, false);
});

test("an error while still seeking keeps the time asked", () => {
  const { instance, video, events } = player();
  instance.seek(10);
  instance.seek(20);
  // Still seeking when the error arrives.
  video.dispatchEvent(new Event("error"));
  assert.deepEqual(video.seeks, [110]);
  assert.equal(instance.seeking, true);
  video.land();
  assert.deepEqual(video.seeks, [110, 120]);
  video.land();
  assert.equal(instance.seeking, false);
  assert.deepEqual(events, ["seeking", "seeked"]);
});
