import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

async function loadModule() {
  const source = await readFile(new URL("../../src/lib/project-timeline.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const module = new vm.SourceTextModule(outputText, { context: vm.createContext({ crypto: globalThis.crypto }) });
  await module.link(() => { throw new Error("Timeline editing must have no runtime dependencies"); });
  await module.evaluate();
  return module.namespace;
}

const api = await loadModule();
// Values created inside the VM context have its own prototypes.
const plain = (value) => JSON.parse(JSON.stringify(value));

function timeline(clips, subtitles = []) {
  return { ...api.emptyTimeline(), clips, subtitles };
}

const clip = (id, material_id, start, end) => ({ id, material_id, start, end });
const sub = (id, clip_id, start, end, text = id) => ({ id, clip_id, start, end, text });

test("clips play back to back and the later clip owns a cut", () => {
  const t = timeline([clip("a", 1, 10, 20), clip("b", 2, 0, 5)]);
  assert.deepEqual(plain(api.placeClips(t).map((p) => p.offset)), [0, 10]);
  assert.equal(api.timelineDuration(t), 15);
  assert.equal(api.clipAt(t, 9.99).clip.id, "a");
  assert.equal(api.clipAt(t, 10).clip.id, "b");
  assert.equal(api.clipAt(t, 15).clip.id, "b");
  assert.equal(api.clipAt(api.emptyTimeline(), 0), null);
});

test("subtitles follow their clip, are clipped to it and hide outside it", () => {
  const t = timeline(
    [clip("a", 1, 10, 20), clip("b", 2, 0, 5)],
    [sub("in-b", "b", 1, 2), sub("tail", "a", 18, 25), sub("hidden", "a", 1, 5), sub("lost", "x", 0, 1)],
  );
  const visible = api.visibleSubtitles(t).map(({ subtitle, start, end }) => [subtitle.id, start, end]);
  assert.deepEqual(plain(visible), [["tail", 8, 10], ["in-b", 11, 12]]);
  const moved = api.moveClip(t, "b", 0);
  assert.deepEqual(plain(api.visibleSubtitles(moved).map((s) => [s.subtitle.id, s.start])), [["in-b", 1], ["tail", 13]]);
  assert.deepEqual(plain(api.projectCues(t).map((cue) => cue.text)), ["tail", "in-b"]);
});

test("splitting moves later subtitles and duplicates the one crossing the cut", () => {
  const t = timeline([clip("a", 1, 10, 20)], [sub("before", "a", 11, 12), sub("across", "a", 14, 16), sub("after", "a", 17, 18)]);
  const { timeline: split, clipId } = api.splitClip(t, 5);
  assert.deepEqual(plain(split.clips.map((c) => [c.start, c.end])), [[10, 15], [15, 20]]);
  assert.equal(split.clips[1].id, clipId);
  const byClip = (id) => split.subtitles.filter((s) => s.clip_id === id).map((s) => s.text).sort();
  assert.deepEqual(plain(byClip("a")), ["across", "before"]);
  assert.deepEqual(plain(byClip(clipId)), ["across", "after"]);
  // The halves still read as one subtitle across the cut.
  assert.deepEqual(plain(api.visibleSubtitles(split).filter((s) => s.subtitle.text === "across").map((s) => [s.start, s.end])), [[4, 5], [5, 6]]);
  // Cuts at an edge would create an empty clip and are ignored.
  assert.equal(api.splitClip(t, 0.05).clipId, null);
  assert.equal(api.splitClip(t, 10).clipId, null);
});

test("trimming stays within the material and keeps a minimum length", () => {
  const t = timeline([clip("a", 1, 10, 20)]);
  assert.deepEqual(plain(api.trimClip(t, "a", "start", -5, 30).clips[0]), clip("a", 1, 0, 20));
  assert.deepEqual(plain(api.trimClip(t, "a", "end", 40, 30).clips[0]), clip("a", 1, 10, 30));
  assert.equal(api.trimClip(t, "a", "end", 5, 30).clips[0].end, 10 + api.MIN_CLIP_DURATION);
  assert.equal(api.trimClip(t, "a", "start", 25, 30).clips[0].start, 20 - api.MIN_CLIP_DURATION);
});

test("removing a clip removes its subtitles", () => {
  const t = timeline([clip("a", 1, 0, 5), clip("b", 1, 5, 10)], [sub("x", "a", 1, 2), sub("y", "b", 6, 7)]);
  const removed = api.removeClip(t, "a");
  assert.deepEqual(plain(removed.clips.map((c) => c.id)), ["b"]);
  assert.deepEqual(plain(removed.subtitles.map((s) => s.id)), ["y"]);
});

test("subtitle edits are made in project time and anchored to the clip below", () => {
  let t = timeline([clip("a", 1, 10, 20), clip("b", 2, 0, 5)]);
  const added = api.addSubtitle(t, 12, 13, "new");
  t = added.timeline;
  assert.deepEqual(plain(t.subtitles[0]), { id: added.id, clip_id: "b", start: 2, end: 3, text: "new" });
  t = api.moveSubtitle(t, added.id, 3);
  assert.deepEqual(plain([t.subtitles[0].clip_id, t.subtitles[0].start, t.subtitles[0].end]), ["a", 13, 14]);
  // Edges stay inside the anchored clip and keep a minimum length.
  t = api.resizeSubtitle(t, added.id, "end", 14);
  assert.equal(t.subtitles[0].end, 20);
  t = api.resizeSubtitle(t, added.id, "start", 30);
  assert.equal(t.subtitles[0].start, 20 - api.MIN_SUBTITLE_DURATION);
  t = api.updateSubtitleText(t, added.id, "edited");
  assert.equal(t.subtitles[0].text, "edited");
  assert.equal(api.removeSubtitle(t, added.id).subtitles.length, 0);
  assert.equal(api.addSubtitle(api.emptyTimeline(), 0, 1).id, null);
});

test("clip subtitles are replaced by material cues and SRT is parsed", () => {
  const cues = api.parseSrt("1\r\n00:00:01,500 --> 00:00:02,000\r\nHello\r\nthere\r\n\r\n2\n00:01:00.25 --> 00:01:01,000\nWorld\n\nbroken");
  assert.deepEqual(plain(cues), [
    { start: 1.5, end: 2, text: "Hello\nthere" },
    { start: 60.25, end: 61, text: "World" },
  ]);
  const t = api.setClipSubtitles(timeline([clip("a", 1, 0, 90)], [sub("old", "a", 0, 1)]), "a", cues);
  assert.deepEqual(plain(t.subtitles.map((s) => s.text)), ["Hello\nthere", "World"]);
});

test("invalid timeline JSON falls back to an empty timeline", () => {
  assert.deepEqual(plain(api.parseTimeline("not json")), plain(api.emptyTimeline()));
  assert.deepEqual(plain(api.parseTimeline("").clips), []);
  assert.equal(api.parseTimeline('{"clips":[{"id":"a"}],"danmu":1}').danmu, true);
});

test("the backend's edits of the saved timeline are merged into unsaved local edits", () => {
  const base = timeline([clip("a", 1, 0, 10), clip("b", 2, 0, 5)], [sub("s1", "a", 1, 2)]);
  // The backend switched danmaku on, removed deleted material 2 and filled
  // clip c with recognized subtitles; locally clip a was trimmed and clip c added.
  const remote = { ...timeline([clip("a", 1, 0, 10), clip("c", 3, 0, 4)], [sub("s1", "a", 1, 2), sub("r1", "c", 0, 1)]), danmu: true };
  const local = timeline([clip("a", 1, 0, 8), clip("b", 2, 0, 5), clip("c", 3, 0, 4)], [sub("s1", "a", 1, 2), sub("s2", "b", 1, 2)]);

  const merged = plain(api.mergeRemoteTimeline(local, base, remote));
  assert.equal(merged.danmu, true);
  assert.deepEqual(merged.clips.map((c) => [c.id, c.end]), [["a", 8], ["c", 4]]);
  assert.deepEqual(merged.subtitles.map((s) => s.id), ["s1", "r1"]);
});

test("local subtitle edits win over the backend's and nothing changes without its edits", () => {
  const base = timeline([clip("a", 1, 0, 10)]);
  const remote = timeline([clip("a", 1, 0, 10)], [sub("r1", "a", 1, 2)]);
  // Subtitles typed locally meanwhile are kept rather than mixed with recognized ones.
  const typed = timeline([clip("a", 1, 0, 10)], [sub("mine", "a", 3, 4)]);
  assert.deepEqual(plain(api.mergeRemoteTimeline(typed, base, remote).subtitles).map((s) => s.id), ["mine"]);
  // Subtitles the editor already knew about and deleted stay deleted.
  const cleared = timeline([clip("a", 1, 0, 10)]);
  assert.equal(api.mergeRemoteTimeline(cleared, remote, remote), cleared);
});
