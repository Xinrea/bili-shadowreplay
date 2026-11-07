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

/** Settle what is waiting on promises. */
const flush = () => new Promise((resolve) => setImmediate(resolve));

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
    this.playbackRate = 1;
    this.muted = false;
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

/** The preview's sound, whose time the test sets, and which loads when it says. */
class FakeClock {
  constructor() {
    this.time = 0;
    this.running = false;
    this.prepared = [];
    this.started = [];
    this.pending = [];
  }
  prepare(time) {
    this.prepared.push(time);
    return new Promise((resolve) => this.pending.push(resolve));
  }
  /** Finish loading what it was asked to. */
  loaded() {
    for (const resolve of this.pending.splice(0)) resolve();
  }
  start(time) {
    this.running = true;
    this.time = time;
    this.started.push(time);
  }
  stop() {
    this.running = false;
  }
}

function player() {
  const [first, second] = [new FakeVideo(), new FakeVideo()];
  const instance = new ProjectPlayer(first, second, new FakeClock());
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
  const instance = new ProjectPlayer(active, standby, new FakeClock());
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
  const instance = new ProjectPlayer(active, standby, new FakeClock());
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
  const instance = new ProjectPlayer(active, standby, new FakeClock());
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

/** A player of two clips, playing from 0 on its clock. */
async function playing() {
  const [active, standby] = [new FakeVideo(), new FakeVideo()];
  const clock = new FakeClock();
  const instance = new ProjectPlayer(active, standby, clock);
  instance.setSegments([
    { clipId: "a", url: "a.mp4", start: 100, end: 110, offset: 0 },
    { clipId: "b", url: "b.mp4", start: 5, end: 15, offset: 10 },
  ]);
  active.loaded();
  active.land();
  standby.loaded();
  standby.land();
  active.seeks.length = standby.seeks.length = 0;
  const events = [];
  for (const type of ["play", "waiting", "playing", "pause", "ended", "seeking"]) {
    instance.addEventListener(type, () => events.push(type));
  }
  const started = instance.play();
  await flush();
  return { instance, active, standby, clock, events, started };
}

/** Move the clock on and let the player follow it, as an animation frame does. */
function tick(clock, video, time) {
  clock.time = time;
  video.dispatchEvent(new Event("timeupdate"));
}

test("playing waits for the sound and the picture there, then the videos follow the clock, muted", async () => {
  const { instance, active, standby, clock, events, started } = await playing();
  assert.equal(active.muted && standby.muted, true);
  assert.deepEqual(events, ["play", "waiting"]);
  assert.deepEqual(clock.prepared, [0]);
  assert.deepEqual(clock.started, []);
  clock.loaded();
  await started;
  assert.deepEqual(clock.started, [0]);
  assert.deepEqual(events, ["play", "waiting", "playing"]);
  tick(clock, active, 2);
  assert.equal(instance.currentTime, 2);
  assert.equal(active.paused, false);
  // Far off, it is seeked to where the clock will be once the seek lands.
  assert.ok(active.seeks.at(-1) > 102 && active.seeks.at(-1) < 103.6);
});

test("a video behind the clock or ahead of it plays faster or slower until in step", async () => {
  const { active, clock, started } = await playing();
  clock.loaded();
  await started;
  active.position = 103.85;
  tick(clock, active, 4);
  assert.equal(active.playbackRate, 1.1);
  active.position = 104.2;
  tick(clock, active, 4.15);
  assert.equal(active.playbackRate, 0.95);
  active.position = 104.205;
  tick(clock, active, 4.2);
  assert.equal(active.playbackRate, 1);
  assert.deepEqual(active.seeks, []);
});

test("pausing stops the clock and shows the frame where it stopped", async () => {
  const { instance, active, clock, events, started } = await playing();
  clock.loaded();
  await started;
  active.position = 107.2;
  tick(clock, active, 7.25);
  active.position = 107.1;
  clock.time = 7.3;
  instance.pause();
  assert.equal(clock.running, false);
  assert.equal(instance.currentTime, 7.3);
  assert.equal(active.paused, true);
  assert.equal(active.seeks.at(-1), 107.3);
  assert.equal(instance.seeking, true);
  assert.deepEqual(events.slice(-2), ["seeking", "pause"]);
});

test("at a cut the standby element, holding the next clip, shows it on the clock's time", async () => {
  const { instance, active, standby, clock, started } = await playing();
  clock.loaded();
  await started;
  active.position = 109.9;
  tick(clock, active, 9.9);
  tick(clock, active, 10.25);
  assert.equal(instance.activeElement, standby);
  assert.equal(instance.activeClipId, "b");
  assert.equal(active.paused, true);
  assert.equal(standby.paused, false);
  // Behind by 0.25 s, as it starts later than the clock: it catches up.
  assert.equal(standby.playbackRate, 1.2);
  assert.deepEqual(standby.seeks, []);
});

test("a seek while playing starts the clock again once it is loaded there", async () => {
  const { instance, active, standby, clock, events, started } = await playing();
  clock.loaded();
  await started;
  active.position = 103;
  tick(clock, active, 3);
  instance.seek(12);
  assert.equal(clock.running, false);
  assert.deepEqual(clock.prepared, [0, 12]);
  assert.equal(events.filter((event) => event === "waiting").length, 2);
  // Not before the picture there is loaded.
  clock.loaded();
  await flush();
  assert.deepEqual(clock.started, [0]);
  const shown = instance.activeElement;
  shown.loaded();
  assert.deepEqual(shown.seeks, [7]);
  shown.land();
  runTimers();
  await flush();
  assert.deepEqual(clock.started, [0, 12]);
});

test("the clock reaching the end of the project ends playing there", async () => {
  const { instance, active, clock, events, started } = await playing();
  clock.loaded();
  await started;
  tick(clock, active, 20.4);
  assert.equal(instance.currentTime, 20);
  assert.equal(instance.paused, true);
  assert.equal(clock.running, false);
  assert.equal(events.at(-1), "ended");
});

test("past the last clip the clock plays on to the end of what runs on, holding the clip's last frame", async () => {
  const { instance, standby, clock, events, started } = await playing();
  instance.setLength(25);
  assert.equal(instance.duration, 25);
  clock.loaded();
  await started;
  tick(clock, standby, 12);
  tick(clock, instance.activeElement, 22);
  assert.equal(instance.currentTime, 22);
  assert.equal(instance.paused, false);
  // Seeked there, the clip is shown at its out point, not past it.
  instance.pause();
  assert.equal(instance.activeElement.seeks.at(-1), 15);
  instance.seek(24);
  assert.equal(instance.currentTime, 24);
  // Shorter again, it goes back to the new end.
  instance.setLength(0);
  assert.equal(instance.currentTime, 20);
  assert.ok(!events.includes("ended"));
});
