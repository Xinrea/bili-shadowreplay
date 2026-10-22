import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

/** Intervals and timeouts that run when the test says. */
const timers = new Map();
let nextTimer = 1;
function runIntervals() {
  for (const [, timer] of [...timers]) if (timer.repeat) timer.callback();
}

async function loadModule() {
  const source = await readFile(new URL("../../src/lib/preview-audio.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const schedule = (repeat) => (callback) => {
    const id = nextTimer++;
    timers.set(id, { callback, repeat });
    return id;
  };
  const context = vm.createContext({
    console,
    performance,
    setTimeout: schedule(false),
    setInterval: schedule(true),
    clearTimeout: (id) => timers.delete(id),
    clearInterval: (id) => timers.delete(id),
  });
  const module = new vm.SourceTextModule(outputText, { context });
  await module.link(() => {
    throw new Error("The preview's sound must have no runtime dependencies");
  });
  await module.evaluate();
  return module.namespace;
}

const api = await loadModule();
const flush = () => new Promise((resolve) => setImmediate(resolve));

class FakeParam {
  constructor() {
    this.value = 1;
    this.events = [];
  }
  setValueAtTime(value, time) {
    this.events.push(["set", +value.toFixed(4), +time.toFixed(4)]);
  }
  linearRampToValueAtTime(value, time) {
    this.events.push(["ramp", +value.toFixed(4), +time.toFixed(4)]);
  }
  setTargetAtTime(value, time) {
    this.events.push(["target", value, time]);
  }
}

class FakeNode {
  connect() {}
  disconnect() {}
}

/** An AudioContext whose time the test sets. */
class FakeContext {
  constructor() {
    this.currentTime = 0;
    this.outputLatency = 0;
    this.destination = new FakeNode();
    this.sources = [];
    this.gains = [];
  }
  createGain() {
    const node = Object.assign(new FakeNode(), { gain: new FakeParam() });
    this.gains.push(node);
    return node;
  }
  createBufferSource() {
    const node = Object.assign(new FakeNode(), {
      stopped: false,
      start: (when, offset, duration) => Object.assign(node, { when: +when.toFixed(4), offset: +offset.toFixed(4), duration: +duration.toFixed(4) }),
      stop: () => (node.stopped = true),
    });
    this.sources.push(node);
    return node;
  }
  async decodeAudioData(data) {
    return { duration: data.duration };
  }
  async resume() {}
}

/** A loader that answers when the test says; `null` sounds are silent. */
function loader(silent = new Set()) {
  const asked = [];
  const waiting = [];
  const load = (file, index) => {
    asked.push(`${file}#${index}`);
    return new Promise((resolve) => waiting.push(() => resolve(silent.has(index) ? null : { duration: 10 })));
  };
  return { load, asked, answer: () => waiting.splice(0).forEach((resolve) => resolve()) };
}

/** Heard over project 5-30 from file time 8, so file 8-33. */
const voice = (more = {}) => ({ id: "v", file: "a.mp4", start: 5, end: 30, from: 8, volume: 0.5, ...more });
const scheduled = (context) => context.sources.filter((node) => !node.stopped).map((node) => [node.when, node.offset, node.duration]);

async function prepared(voices = [voice()], at = 10, silent) {
  const context = new FakeContext();
  const chunks = loader(silent);
  const audio = new api.PreviewAudio(context, chunks.load);
  audio.setVoices(voices);
  const ready = audio.prepare(at);
  chunks.answer();
  await ready;
  return { context, chunks, audio };
}

test("the chunks of a span and the fades are those of export", () => {
  assert.deepEqual([...api.chunksOf(voice(), 0, 100)], [0, 1, 2, 3]);
  assert.deepEqual([...api.chunksOf(voice(), 10, 11)], [1]);
  assert.deepEqual([...api.chunksOf(voice(), 30, 40)], []);
  // Up to the end of a chunk, not into the next.
  assert.deepEqual([...api.chunksOf(voice({ from: 0, start: 0 }), 0, 10)], [0]);
  const faded = voice({ fadeIn: 2, fadeOut: 5 });
  assert.deepEqual([4, 5, 6, 7, 20, 27.5, 30].map((time) => api.fadeAt(faded, time)), [0, 0, 0.5, 1, 1, 0.5, 0]);
});

test("chunks are scheduled back to back on the context's time from where playing starts", async () => {
  const { context, chunks, audio } = await prepared();
  assert.deepEqual([...chunks.asked], ["a.mp4#1"]);
  audio.start(10);
  // File 13 at once, after the lead.
  assert.deepEqual(scheduled(context), [[0.05, 3, 7]]);
  chunks.answer();
  await flush();
  context.currentTime = 5;
  runIntervals();
  // Chunk 2 starts as chunk 1 ends, chunk 3 at the end of the voice's file.
  assert.deepEqual(scheduled(context), [[0.05, 3, 7], [7.05, 0, 10]]);
  // Loaded once it is within reach of what is heard.
  context.currentTime = 12;
  runIntervals();
  chunks.answer();
  await flush();
  context.currentTime = 15;
  runIntervals();
  assert.deepEqual(scheduled(context).at(-1), [17.05, 0, 3]);
  audio.stop();
  assert.deepEqual(scheduled(context), []);
});

test("a chunk loaded late is heard from when it comes, cut short at its head", async () => {
  const context = new FakeContext();
  const chunks = loader();
  const audio = new api.PreviewAudio(context, chunks.load);
  audio.setVoices([voice()]);
  audio.start(10);
  assert.deepEqual(scheduled(context), []);
  chunks.answer();
  await flush();
  context.currentTime = 1;
  runIntervals();
  assert.deepEqual(scheduled(context)[0], [1.005, 3.955, 6.045]);
});

test("a voice changed in volume plays on, and changed otherwise starts again", async () => {
  const { context, audio } = await prepared();
  audio.start(10);
  const [first] = context.sources;
  const volume = context.gains[1];
  audio.setVoices([voice({ volume: 0.25 })]);
  assert.equal(first.stopped, false);
  assert.deepEqual(volume.gain.events, [["target", 0.25, 0]]);
  audio.setVoices([voice({ volume: 0.25, from: 9 })]);
  assert.equal(first.stopped, true);
  // Again at once from the chunk loaded: file 14 is now heard at 10.
  assert.deepEqual(scheduled(context), [[0.05, 4, 6]]);
  // Silent ones are not loaded at all.
  audio.setVoices([voice({ volume: 0 })]);
  assert.deepEqual(scheduled(context), []);
});

test("fades ramp over the voice from where it is first heard", async () => {
  const { context, audio } = await prepared([voice({ fadeIn: 2, fadeOut: 4, start: 9 })], 10);
  audio.start(10);
  // The master's gain, then the voice's volume and its fade.
  const [, volume, fade] = context.gains;
  assert.equal(volume.gain.value, 0.5);
  // Entering at 9.95, partway into the fade in, which ends at 11; the fade out runs 26-30.
  assert.deepEqual(fade.gain.events, [
    ["set", 0.475, 0],
    ["ramp", 1, 1.05],
    ["ramp", 1, 16.05],
    ["ramp", 0, 20.05],
  ]);
});

test("a chunk without sound is silence, and asked for again only later", async () => {
  const { context, chunks, audio } = await prepared([voice()], 10, new Set([1]));
  audio.start(10);
  runIntervals();
  runIntervals();
  assert.deepEqual(scheduled(context), []);
  assert.equal(chunks.asked.filter((name) => name === "a.mp4#1").length, 1);
});

test("the clock tells the time of what is heard, and holds until the first sound is", async () => {
  const { context, audio } = await prepared();
  context.outputLatency = 0.1;
  audio.start(10);
  context.currentTime = 0.1;
  assert.equal(audio.time, 10);
  context.currentTime = 1.15;
  assert.equal(+audio.time.toFixed(6), 11);
  // Never back, as when the latency told grows.
  context.outputLatency = 0.3;
  assert.equal(+audio.time.toFixed(6), 11);
  audio.stop();
  context.currentTime = 9;
  assert.equal(+audio.time.toFixed(6), 11);
});
