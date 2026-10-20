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

test("a subtitle drag keeps subtitles merged in while dragging, also when undone", () => {
  const atDragStart = timeline([clip("a", 1, 0, 10), clip("b", 2, 0, 10)], [sub("s1", "a", 1, 2)]);
  // Recognition fills clip b mid-drag; the editor merges it into the live timeline.
  const merged = api.mergeRemoteTimeline(atDragStart, atDragStart, timeline(atDragStart.clips, [sub("s1", "a", 1, 2), sub("r1", "b", 0, 1)]));
  // Previews apply to the live timeline, so the release commits it with both.
  const released = api.moveSubtitle(merged, "s1", 3, 4);
  assert.deepEqual(plain(released.subtitles).map((s) => [s.id, s.start]), [["s1", 3], ["r1", 0]]);
  // The undo step is the live timeline with only the dragged subtitle put back.
  const before = plain(api.withSubtitleFrom(released, atDragStart, "s1"));
  assert.deepEqual(before.subtitles.map((s) => [s.id, s.start]), [["s1", 1], ["r1", 0]]);
});

const text = (id, clip_id, start, duration, x = 960) => ({
  id, clip_id, start, duration, kind: "text", element: { kind: "text", id: 0, x, y: 540, rotation: 0, content: id },
});
const pip = (id, clip_id, start, duration, material_id = 9, source_start = 0) => ({
  id, clip_id, start, duration, kind: "video", material_id, source_start, volume: 1,
  element: { x: 1500, y: 800, width: 640, rotation: 0 },
});
const track = (id, ...items) => ({ id, items });
const withTracks = (t, ...tracks) => ({ ...t, tracks });
const spans = (t) => plain(api.placeOverlays(t).map((p) => [p.item.id, p.track, p.start, p.end, p.skip]));

test("overlays follow the moment of their clip and run on over later clips", () => {
  // a shows 10-20 of material 1 at 0-10, b shows 0-5 of material 2 at 10-15.
  const t = withTracks(timeline([clip("a", 1, 10, 20), clip("b", 2, 0, 5)]), track("t1", text("x", "a", 18, 4), text("y", "b", 1, 1)));
  assert.deepEqual(spans(t), [["x", 0, 8, 12, 0], ["y", 0, 11, 12, 0]]);
  // Swapping the clips carries each overlay along with its clip.
  assert.deepEqual(spans(api.moveClip(t, "b", 0)), [["x", 0, 13, 15, 0], ["y", 0, 1, 2, 0]]);
  // Trimming the head of a clip past an anchor hides that part; trimming the
  // tail past it hides the whole overlay.
  const headTrimmed = withTracks(t, track("t1", text("x", "a", 9, 3)));
  assert.deepEqual(spans(headTrimmed), [["x", 0, 0, 2, 1]]);
  assert.deepEqual(spans(api.trimClip(t, "a", "end", 17, 30)), [["y", 0, 8, 9, 0]]);
  // Nothing runs past the end of the project.
  assert.deepEqual(spans(withTracks(t, track("t1", text("x", "b", 4, 9)))), [["x", 0, 14, 15, 0]]);
});

test("adding an overlay anchors it at the time given on the lowest free track", () => {
  let t = timeline([clip("a", 1, 10, 20), clip("b", 2, 0, 5)]);
  const content = { kind: "text", element: { content: "hi" } };
  const first = api.addOverlay(t, content, 12, 3);
  t = first.timeline;
  assert.deepEqual(plain(api.findOverlay(t, first.id)), { ...content, id: first.id, clip_id: "b", start: 2, duration: 3 });
  // Taken there: a new track above; free again later: back on the bottom one.
  const second = api.addOverlay(t, content, 13, 1);
  assert.equal(second.timeline.tracks.length, 2);
  assert.equal(second.timeline.tracks[1].items[0].id, second.id);
  const third = api.addOverlay(second.timeline, content, 4, 2);
  assert.equal(third.timeline.tracks[0].items[1].id, third.id);
  // Cut short at the end; nothing to anchor to without clips.
  const late = api.addOverlay(t, content, 14.5, 3);
  assert.equal(api.findOverlay(late.timeline, late.id).duration, 0.5);
  assert.equal(api.addOverlay(api.emptyTimeline(), content, 0, 1).id, null);
});

test("moving an overlay re-anchors it and lands on a new track where the target is taken", () => {
  const t = withTracks(
    timeline([clip("a", 1, 0, 10), clip("b", 2, 0, 10)]),
    track("t1", text("x", "a", 1, 2), text("y", "a", 5, 2)),
    track("t2", pip("v", "b", 0, 3)),
  );
  const moved = api.moveOverlay(t, "x", 12, { track: 0 });
  assert.deepEqual(plain([api.findOverlay(moved, "x").clip_id, api.findOverlay(moved, "x").start]), ["b", 2]);
  assert.deepEqual(spans(moved), [["y", 0, 5, 7, 0], ["x", 0, 12, 14, 0], ["v", 1, 10, 13, 0]]);
  // Onto y: x gets a track between.
  const stacked = api.moveOverlay(t, "x", 6, { track: 0 });
  assert.deepEqual(plain(stacked.tracks.map((tr) => tr.items.map((i) => i.id))), [["y"], ["x"], ["v"]]);
  // Moving the only item off a track removes the track; the project end stops it.
  const lifted = api.moveOverlay(t, "v", 30, { insert: 0 });
  assert.deepEqual(plain(lifted.tracks.map((tr) => tr.items.map((i) => i.id))), [["v"], ["x", "y"]]);
  assert.deepEqual(spans(lifted)[0], ["v", 0, 17, 20, 0]);
});

test("a video overlay moved or trimmed keeps showing the same material", () => {
  // The clip's head was trimmed past the overlay's anchor: 1 s of it is hidden.
  const t = withTracks(timeline([clip("a", 1, 5, 15)]), track("t1", pip("v", "a", 4, 3, 9, 20), text("x", "a", 9, 2)));
  assert.deepEqual(spans(t)[0], ["v", 0, 0, 2, 1]);
  const moved = api.findOverlay(api.moveOverlay(t, "v", 1, { track: 0 }), "v");
  assert.deepEqual(plain([moved.start, moved.duration, moved.source_start]), [6, 2, 21]);
  // Its start goes back no further than the material's start, its end no
  // further than the material's end or the next item on the track.
  const early = api.findOverlay(api.trimOverlay(t, "v", "start", -30, 60), "v");
  assert.deepEqual(plain([early.start, early.duration, early.source_start]), [5, 2, 21]);
  const shifted = withTracks(t, track("t1", pip("v", "a", 6, 2, 9, 21), text("x", "a", 12, 2)));
  const extended = api.findOverlay(api.trimOverlay(shifted, "v", "start", 0, 60), "v");
  assert.deepEqual(plain([extended.start, extended.duration, extended.source_start]), [5, 3, 20]);
  assert.equal(api.findOverlay(api.trimOverlay(shifted, "v", "end", 9, 60), "v").duration, 6);
  assert.equal(api.findOverlay(api.trimOverlay(shifted, "v", "end", 9, 22.5), "v").duration, 1.5);
  const shortest = api.findOverlay(api.trimOverlay(shifted, "v", "end", 0, 60), "v").duration;
  assert.ok(Math.abs(shortest - api.MIN_OVERLAY_DURATION) < 1e-9);
});

test("splitting an overlay leaves two items that play on from each other", () => {
  const t = withTracks(timeline([clip("a", 1, 0, 10)]), track("t1", pip("v", "a", 2, 6, 9, 30)));
  const { timeline: split, id } = api.splitOverlay(t, "v", 5);
  const [left, right] = split.tracks[0].items;
  assert.deepEqual(plain([left.start, left.duration, left.source_start]), [2, 3, 30]);
  assert.deepEqual(plain([right.id, right.start, right.duration, right.source_start]), [id, 5, 3, 33]);
  assert.equal(api.splitOverlay(t, "v", 2.05).id, null);
});

test("clip edits carry overlays along with their clips", () => {
  const t = withTracks(timeline([clip("a", 1, 0, 10), clip("b", 2, 0, 10)]), track("t1", text("x", "a", 2, 1), text("y", "a", 6, 1), pip("v", "b", 1, 1, 2)));
  const { timeline: split, clipId } = api.splitClip(t, 5);
  assert.deepEqual(plain(split.tracks[0].items.map((i) => [i.id, i.clip_id])), [["x", "a"], ["y", clipId], ["v", "b"]]);
  // Removing a clip removes what is anchored to it, and an emptied track.
  assert.deepEqual(plain(api.removeClip(t, "a").tracks.map((tr) => tr.items.map((i) => i.id))), [["v"]]);
  assert.deepEqual(plain(api.removeClip(api.removeClip(t, "a"), "b").tracks), []);
});

test("settling lifts overlays that clip edits made overlap onto new tracks", () => {
  // Clip a ends with x running on over clip b, where y starts.
  const t = withTracks(timeline([clip("a", 1, 0, 4), clip("b", 2, 0, 10)]), track("t1", text("x", "a", 2, 3), text("y", "b", 2, 2)), track("t2", text("z", "a", 0, 1)));
  assert.equal(api.settleOverlays(t), t);
  // Trimming a shortens it, so y comes closer and meets x.
  const trimmed = api.trimClip(t, "a", "end", 2.5, 4);
  const settled = api.settleOverlays(trimmed);
  assert.deepEqual(plain(settled.tracks.map((tr) => tr.items.map((i) => i.id))), [["x"], ["y"], ["z"]]);
  assert.equal(settled.tracks[0].id, "t1");
  assert.equal(api.settleOverlays(settled), settled);
  assert.deepEqual(plain(api.settleOverlays(withTracks(t, track("t1"), ...t.tracks)).tracks.map((tr) => tr.id)), ["t1", "t2"]);
});

test("overlays move between layers, onto new tracks where a neighbouring one is taken", () => {
  const t = withTracks(timeline([clip("a", 1, 0, 10)]), track("t1", text("x", "a", 0, 2), text("y", "a", 5, 2)), track("t2", text("z", "a", 1, 2)));
  const ids = (next) => plain(next.tracks.map((tr) => tr.items.map((i) => i.id)));
  assert.deepEqual(ids(api.shiftOverlay(t, "y", 1)), [["x"], ["z", "y"]]);
  assert.deepEqual(ids(api.shiftOverlay(t, "x", 1)), [["y"], ["x"], ["z"]]);
  assert.deepEqual(ids(api.shiftOverlay(t, "x", -1)), [["x"], ["y"], ["z"]]);
  assert.deepEqual(ids(api.shiftOverlay(t, "z", -1)), [["x", "y"], ["z"]]);
  // The only item of the top track has nowhere further up to go.
  assert.equal(api.canShiftOverlay(t, "z", 1), false);
  assert.equal(api.canShiftOverlay(t, "x", -1), true);
  const copy = api.duplicateOverlay(t, "x", (item) => ({ ...item, element: { ...item.element, x: item.element.x + 40 } }));
  assert.deepEqual(ids(copy.timeline), [["x", "y"], [copy.id], ["z"]]);
  assert.equal(api.findOverlay(copy.timeline, copy.id).element.x, 1000);
  assert.deepEqual(ids(api.removeOverlay(t, "z")), [["x", "y"]]);
});

test("overlays of a material the backend deleted are dropped when merging", () => {
  const clips = [clip("a", 1, 0, 10), clip("b", 2, 0, 5)];
  const base = withTracks(timeline(clips), track("t1", pip("only", "a", 1, 2, 3)));
  // Material 3 was only used by a video overlay; material 2's clip had a text.
  const local = withTracks(timeline(clips), track("t1", pip("only", "a", 1, 2, 3), text("x", "b", 1, 1)), track("t2", text("keep", "a", 5, 1)));
  const remote = timeline([clip("a", 1, 0, 10)]);
  const merged = api.mergeRemoteTimeline(local, base, remote);
  assert.deepEqual(plain(merged.tracks.map((tr) => tr.items.map((i) => i.id))), [["keep"]]);
  assert.deepEqual([...api.timelineMaterials(local)].sort(), [1, 2, 3]);
  assert.equal(api.mergeRemoteTimeline(local, local, local), local);
  // Old timelines without tracks parse to none.
  assert.deepEqual(plain(api.parseTimeline('{"clips":[]}').tracks), []);
});
