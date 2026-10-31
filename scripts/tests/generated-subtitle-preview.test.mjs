import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function load(name) {
  const source = await readFile(new URL(`../../src/lib/${name}.ts`, import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const module = new vm.SourceTextModule(outputText, { context: vm.createContext({ TextEncoder }) });
  await module.link(() => { throw new Error("Preview helpers must have no runtime dependencies"); });
  await module.evaluate();
  return module.namespace;
}
const api = await load("generated-subtitle-preview");
const ass = await load("subtitle-ass");
const build = (cues) => ass.buildSubtitleAss(cues, ass.DEFAULT_SUBTITLE_STYLE);
const cue = (startTime, endTime, text) => ({ startTime, endTime, text });
const dialogues = (text) => text?.split("\n").filter((line) => line.startsWith("Dialogue:")) ?? [];
const flush = () => new Promise((resolve) => setImmediate(resolve));

function harness() {
  const requests = [];
  const states = [];
  const queue = new api.GeneratedSubtitlePreviewQueue((request) => new Promise((resolve, reject) => {
    requests.push({ request, resolve, reject });
  }), (state) => states.push(state));
  return { queue, requests, states, latest: () => states.at(-1) };
}

test("generated selector preserves headers, dialogue bytes, overlapping order and exact boundaries", () => {
  const source = build([
    cue(1, 3, "<b>你好, 👩🏽‍💻</b>\nsecond"),
    cue(1, 2, "👨‍👩‍👧‍👦"),
    cue(2, 4, "🇨🇳, end"),
  ]);
  const selector = api.parseGeneratedSubtitlePreview(source);
  const rows = dialogues(source);
  assert.equal(selector.at(0), null);
  assert.deepEqual(dialogues(selector.at(1)), rows.slice(0, 2));
  assert.equal(selector.at(1), selector.at(1.99999));
  assert.deepEqual(dialogues(selector.at(2)), [rows[0], rows[2]]);
  assert.deepEqual(dialogues(selector.at(3)), [rows[2]]);
  assert.equal(selector.at(4), null);
  assert.deepEqual(dialogues(selector.at(1.5)), rows.slice(0, 2), "backwards seek");
  const header = (text) => text.split("\n").filter((line) => !line.startsWith("Dialogue:")).join("\n");
  assert.equal(header(selector.at(1)), header(source));
});

test("selection uses generated centisecond times and never submits a movie-sized track", () => {
  const source = build(Array.from({ length: 20_000 }, (_, index) => cue(index, index + 1, `字幕 ${index}`)));
  const selector = api.parseGeneratedSubtitlePreview(source);
  assert.ok(source.length > 1_000_000);
  assert.equal(dialogues(selector.at(123.5)).length, 1);
  assert.ok(selector.at(123.5).length < 2000);
  const fractional = api.parseGeneratedSubtitlePreview(build([cue(0.289, 0.301, "fractional")]));
  assert.equal(fractional.at(0.289), null);
  assert.equal(dialogues(fractional.at(0.29)).length, 1);
  assert.equal(fractional.at(0.30), null);
});

test("empty generated documents clear, invalid input fails visibly rather than becoming external ASS", () => {
  assert.equal(api.parseGeneratedSubtitlePreview("").at(1), null);
  assert.equal(api.parseGeneratedSubtitlePreview(build([])).at(1), null);
  assert.throws(() => api.parseGeneratedSubtitlePreview("external ASS"));
  const selector = api.parseGeneratedSubtitlePreview(build([cue(0, 1, "test")]));
  for (const time of [NaN, Infinity, -1]) assert.throws(() => selector.at(time));
  assert.throws(() => api.parseGeneratedSubtitlePreview(build([cue(0, 1, "test")]).replace("0:00:01.00", "bad")));
});

test("queue renders once per static active set; seeks use bounded cached PNGs", async () => {
  const h = harness();
  h.queue.update("A", 1, "Arial");
  for (let frame = 0; frame < 100; frame++) h.queue.update("A", 1 + frame / 100, "Arial");
  assert.equal(h.requests.length, 1);
  h.requests[0].resolve("png-a");
  await flush();
  assert.equal(h.latest().image, "png-a");
  h.queue.update(null, 2, "Arial");
  assert.equal(h.latest().image, "");
  h.queue.update("A", 1, "Arial");
  assert.equal(h.latest().image, "png-a");
  assert.equal(h.requests.length, 1);
  h.queue.update("A", 1, "Other Font");
  assert.equal(h.requests.length, 2);
});

test("queue coalesces edits and obsolete replies cannot paint over latest or empty cues", async () => {
  const h = harness();
  h.queue.update("A", 1, "Arial");
  h.queue.update("B", 2, "Arial");
  h.queue.update("C", 3, "Arial");
  assert.equal(h.requests.length, 1);
  h.requests[0].resolve("stale-a");
  await flush();
  assert.equal(h.requests.length, 2);
  assert.equal(h.requests[1].request.assContent, "C");
  assert.equal(h.latest().image, "");
  h.queue.update(null, 4, "Arial");
  h.requests[1].resolve("stale-c");
  await flush();
  assert.equal(h.latest().image, "");
  assert.equal(h.latest().loading, false);
});

test("native errors are visible and retry starts a fresh request, disposal ignores pending replies", async () => {
  const h = harness();
  h.queue.update("A", 1, "Arial");
  h.requests[0].reject("native rendering failed");
  await flush();
  assert.match(h.latest().error, /native rendering failed/);
  h.queue.update("A", 1, "Arial", true);
  assert.equal(h.requests.length, 2);
  const count = h.states.length;
  h.queue.dispose();
  h.requests[1].resolve("late");
  await flush();
  assert.equal(h.states.length, count);
});

test("queue enforces UTF-8 document and retained PNG cache bounds", async () => {
  const h = harness();
  h.queue.update("字".repeat(90_000), 1, "Arial");
  assert.equal(h.requests.length, 0);
  assert.match(h.latest().error, /256 KiB/);
  for (let index = 0; index < 9; index++) {
    h.queue.update(String(index), 1, "Arial");
    h.requests.at(-1).resolve(`png-${index}`);
    await flush();
  }
  h.queue.update("0", 1, "Arial");
  assert.equal(h.requests.length, 10, "oldest entry evicted after eight documents");
  h.requests.at(-1).resolve("x".repeat(5 * 1024 * 1024));
  await flush();
  h.queue.update(null, 1, "Arial");
  h.queue.update("0", 1, "Arial");
  assert.equal(h.requests.length, 11, "large PNG is displayed but not retained");
});

test("generated preview and export callers share the native subtitle document", async () => {
  const root = new URL("../../src/lib/components/", import.meta.url);
  const project = await readFile(new URL("project/ProjectEditor.svelte", root), "utf8");
  const style = await readFile(new URL("SubtitleStyleEditor.svelte", root), "utf8");
  const upload = await readFile(new URL("upload/UploadEditor.svelte", root), "utf8");
  assert.match(project, /<NativeSubtitlePreview/);
  assert.match(style, /<NativeSubtitlePreview/);
  assert.match(project, /subtitleAss: exportedSubtitles \? exportedAss : null/);
  assert.match(upload, /subtitleAss, overlays/);
});
