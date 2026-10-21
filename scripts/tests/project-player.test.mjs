import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

/** Timers that run when the test says. */
const timers = new Map();
let nextTimer = 1;
function runTimers() {
  for (const [id, callback] of [...timers]) {
    timers.delete(id);
    callback();
  }
}

async function loadModule() {
  const source = await readFile(new URL("../../src/lib/project-player.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const context = vm.createContext({
    EventTarget,
    Event,
    performance,
    setTimeout: (callback) => {
      const id = nextTimer++;
      timers.set(id, callback);
      return id;
    },
    clearTimeout: (id) => timers.delete(id),
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
    // As the load algorithm: a seek going on ends without "seeked".
    this.loads = (this.loads ?? 0) + 1;
    this.source = value;
    this.readyState = 0;
    this.seeking = false;
    this.position = 0;
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

test("a seek moving on to another file ends the one going on", () => {
  const [active, standby] = [new FakeVideo(), new FakeVideo()];
  const instance = new ProjectPlayer(active, standby);
  // The next clip starts at the head of its file.
  instance.setSegments([
    { clipId: "a", url: "a.mp4", start: 100, end: 110, offset: 0 },
    { clipId: "b", url: "b.mp4", start: 0, end: 10, offset: 10 },
  ]);
  active.loaded();
  active.land();
  standby.loaded();
  const events = [];
  for (const type of ["seeking", "seeked"]) instance.addEventListener(type, () => events.push(type));
  instance.seek(9);
  // On to the cut while that seek goes on, which loads the other file.
  instance.seek(10);
  active.loaded();
  assert.equal(instance.seeking, false);
  assert.deepEqual(events, ["seeking", "seeked"]);
});

test("clearing the clips ends the seek going on", () => {
  const { instance, video, events } = player();
  instance.seek(10);
  instance.setSegments([]);
  instance.seek(0);
  assert.equal(instance.seeking, false);
  assert.deepEqual(events, ["seeking", "seeked"]);
  video.land();
  assert.deepEqual(events, ["seeking", "seeked"]);
});

test("a file replaced before its metadata came is not seeked in the next one", () => {
  const [active, standby] = [new FakeVideo(), new FakeVideo()];
  const instance = new ProjectPlayer(active, standby);
  instance.setSegments([
    { clipId: "a", url: "a.mp4", start: 100, end: 110, offset: 0 },
    { clipId: "b", url: "b.mp4", start: 0, end: 10, offset: 10 },
  ]);
  // Straight on to the next clip, before the first file's metadata.
  instance.seek(10);
  active.loaded();
  // Shown at the in point of its own clip, not that of the one before.
  assert.deepEqual(active.seeks, []);
  assert.equal(instance.seeking, false);
});

test("a clip coming back after the clips were cleared is loaded again", () => {
  const { instance, video } = player();
  const loads = video.loads;
  instance.setSegments([]);
  instance.seek(0);
  instance.setSegments([{ clipId: "a", url: "a.mp4", start: 100, end: 400, offset: 0 }]);
  instance.seek(5);
  assert.equal(video.loads, loads + 1);
  assert.equal(video.src, "a.mp4");
});

test("a seek queued behind one that never lands is dropped with it", () => {
  const { instance, video, events } = player();
  instance.seek(10);
  instance.seek(20);
  assert.equal(instance.seeking, true);
  // Its "seeked" never comes.
  runTimers();
  assert.equal(instance.seeking, false);
  assert.deepEqual(events, ["seeking", "seeked"]);
});

test("each clip's sound plays at its own volume, from where it is reached", () => {
  const [active, standby] = [new FakeVideo(), new FakeVideo()];
  active.volume = standby.volume = 1;
  const instance = new ProjectPlayer(active, standby);
  instance.setSegments([
    { clipId: "a", url: "a.mp4", start: 100, end: 110, offset: 0 },
    { clipId: "b", url: "b.mp4", start: 0, end: 10, offset: 10 },
  ]);
  instance.volume = 0.8;
  instance.setClipVolumes(new Map([["b", 0.5]]));
  assert.equal(active.volume, 0.8);
  // Seeked into clip b.
  instance.seek(12);
  assert.equal(instance.activeElement.volume, 0.4);
  instance.setClipVolumes(new Map());
  assert.equal(instance.activeElement.volume, 0.8);
});
