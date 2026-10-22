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

test("deleting a middle material brings overlays together, and settling lifts them apart", () => {
  // x on clip a runs on over b; once b goes, it meets y on clip c.
  const clips = [clip("a", 1, 0, 4), clip("b", 2, 0, 3), clip("c", 3, 0, 10)];
  const local = withTracks(timeline(clips), track("t1", text("x", "a", 2, 4), text("y", "c", 1, 2)));
  const merged = api.mergeRemoteTimeline(local, local, timeline([clips[0], clips[2]]));
  assert.deepEqual(spans(merged), [["x", 0, 2, 6, 0], ["y", 0, 5, 7, 0]]);
  const settled = api.settleOverlays(merged);
  assert.deepEqual(plain(settled.tracks.map((tr) => tr.items.map((i) => i.id))), [["x"], ["y"]]);
});

test("an overlay drag keeps changes merged in while dragging, also when undone", () => {
  const clips = [clip("a", 1, 0, 10), clip("b", 2, 0, 10)];
  const atDragStart = withTracks(timeline(clips), track("t1", text("x", "a", 1, 2)), track("t2", pip("v", "b", 0, 3, 2)));
  // Material 2 is deleted mid-drag; the editor merges that into the live timeline.
  const merged = api.mergeRemoteTimeline(atDragStart, atDragStart, timeline([clips[0]]));
  const trimmed = api.trimOverlay(api.withOverlayFrom(merged, atDragStart, "x"), "x", "end", 5);
  assert.deepEqual(plain(trimmed.tracks.map((tr) => tr.items.map((i) => [i.id, i.duration]))), [[["x", 4]]]);
  // The undo step puts back only the dragged overlay, not the deleted video.
  const before = plain(api.withOverlayFrom(trimmed, atDragStart, "x"));
  assert.deepEqual(before.tracks.map((tr) => tr.items.map((i) => [i.id, i.duration])), [[["x", 2]]]);
  assert.equal(api.withOverlayFrom(merged, atDragStart, "v"), merged);
});

/** Frames at `fps` each clip lasts, as export cuts them: by where they fall in the project. */
function exportFrames(t, fps) {
  return api.placeClips(t).map((p) => Math.round((p.offset + p.duration) * fps) - Math.round(p.offset * fps));
}

const isWhole = (duration, fps) => Math.abs(duration * fps - Math.round(duration * fps)) < 1e-6;

test("with a frame rate, trims and splits leave clips of whole frames within their material", () => {
  for (const fps of [30, 29.97, 60]) {
    const t = timeline([clip("a", 1, 10, 10 + 300 / fps), clip("b", 2, 0, 5)]);
    const trimmed = api.trimClip(t, "a", "end", 13.4567, 30, fps).clips[0];
    assert.ok(isWhole(trimmed.end - trimmed.start, fps), `${fps}: ${trimmed.end - trimmed.start}`);
    assert.ok(Math.abs(trimmed.end - 13.4567) <= 0.5 / fps + 1e-9);
    const started = api.trimClip(t, "a", "start", 11.2345, 30, fps).clips[0];
    assert.ok(isWhole(started.end - started.start, fps));
    // Within the material, and no shorter than the shortest clip.
    const longest = api.trimClip(t, "a", "end", 99, 30.01, fps).clips[0];
    assert.ok(longest.end <= 30.01 && isWhole(longest.end - longest.start, fps));
    assert.ok(api.trimClip(t, "a", "start", -5, 30, fps).clips[0].start >= 0);
    // Never past the material: with less than a frame of it left, all of it.
    const sliver = api.trimClip(timeline([clip("s", 1, 29.995, 30)]), "s", "end", 31, 30, fps).clips[0];
    assert.ok(sliver.end <= 30 && sliver.end > sliver.start);
    const near = api.trimClip(timeline([clip("s", 1, 0, 0.003)]), "s", "start", -1, 30, fps).clips[0];
    assert.ok(near.start >= 0 && near.end - near.start > 0);
    const shortest = api.trimClip(t, "a", "end", 0, 30, fps).clips[0];
    assert.ok(shortest.end - shortest.start >= api.MIN_CLIP_DURATION - 1e-9 && isWhole(shortest.end - shortest.start, fps));
    const { timeline: split } = api.splitClip(t, 4.321, fps);
    assert.ok(split.clips.slice(0, 2).every((c) => isWhole(c.end - c.start, fps)));
  }
});

test("a trim moves the clips after it by whole frames, which export cuts the same", () => {
  const fps = 29.97;
  const t = api.insertClips(api.emptyTimeline(), [1, 2, 3, 4].map((m) => ({ material_id: m, start: 0.37 * m, end: 4.71 * m })), 0, fps);
  assert.ok(t.clips.every((c) => isWhole(c.end - c.start, fps)));
  const before = exportFrames(t, fps);
  const trimmed = api.trimClip(t, t.clips[1].id, "end", t.clips[1].end - 0.8123, 0, fps);
  const after = exportFrames(trimmed, fps);
  assert.notEqual(after[1], before[1]);
  assert.deepEqual(plain([after[0], ...after.slice(2)]), plain([before[0], ...before.slice(2)]));
  // Every clip after it moved by whole frames.
  const offsets = (timeline) => api.placeClips(timeline).map((p) => p.offset * fps);
  const moved = offsets(trimmed).map((offset, index) => offset - offsets(t)[index]);
  assert.ok(moved.slice(2).every((frames) => isWhole(frames / fps, fps) && Math.abs(frames - moved[2]) < 1e-6));
});

test("deleting or moving a clip of whole frames keeps the others' frames", () => {
  const fps = 30;
  const t = api.snapClips(timeline([clip("a", 1, 0.5, 3.217), clip("b", 2, 1, 4.4), clip("c", 3, 0, 2.05), clip("d", 4, 2, 6)]), fps);
  const frames = (timeline) => Object.fromEntries(api.placeClips(timeline).map((p, i) => [p.clip.id, exportFrames(timeline, fps)[i]]));
  const before = frames(t);
  const removed = frames(api.removeClip(t, "a"));
  assert.deepEqual(plain(removed), plain({ b: before.b, c: before.c, d: before.d }));
  assert.deepEqual(plain(frames(api.moveClip(t, "d", 0))), plain(before));
  assert.ok(api.placeClips(api.removeClip(t, "a")).every((p) => isWhole(p.offset, fps)));
});

test("snapping shortens clips to whole frames by less than one, once", () => {
  const t = timeline([clip("a", 1, 0.5, 3.21), clip("b", 2, 0, 2)]);
  const snapped = api.snapClips(t, 30);
  assert.deepEqual(plain(snapped.clips.map((c) => Math.round((c.end - c.start) * 30))), [81, 60]);
  assert.ok(t.clips[0].end - snapped.clips[0].end < 1 / 30);
  assert.equal(snapped.clips[1], t.clips[1]);
  assert.equal(api.snapClips(snapped, 30), snapped);
  assert.equal(api.snapClips(t, 0), t);
  // A file of 360 frames at 30000/1001 FPS, rounded to 12.012 s and 29.97 FPS,
  // keeps all of them.
  const ntsc = api.snapClips(timeline([clip("n", 1, 0, 12.012)]), 29.97);
  assert.equal(Math.round((ntsc.clips[0].end - ntsc.clips[0].start) * 29.97 * 1e6) / 1e6, 360);
  const inserted = api.insertClips(api.emptyTimeline(), [{ material_id: 1, start: 0, end: 12.012 }], 0, 29.97).clips[0];
  assert.equal(Math.round((inserted.end - inserted.start) * 29.97), 360);
  const full = api.trimClip(timeline([clip("n", 1, 0, 6)]), "n", "end", 99, 12.012, 29.97).clips[0];
  assert.equal(Math.round(full.end * 29.97), 360);
});

test("subtitle cues sit between frames and move by whole frames with their clip", () => {
  const fps = 60;
  // An ASS document has centiseconds; the frame a time shows first, from it.
  const firstFrame = (time) => Math.ceil((Math.round(time * 100) / 100) * fps - 1e-6);
  const frames = (lead) => {
    const t = timeline([clip("a", 1, 0, lead), clip("b", 2, 0, 9)], [sub("s", "b", 1.2345, 2.0123), sub("t", "b", 5.5, 7.77)]);
    return api.projectCues(t, fps).flatMap((cue) => [firstFrame(cue.startTime), firstFrame(cue.endTime)]);
  };
  const before = frames(2);
  for (let moved = 1; moved <= 12; moved++) {
    assert.deepEqual(plain(frames(2 + moved / fps)), plain(before.map((frame) => frame + moved)), `moved by ${moved} frames`);
  }
  // Each shows from the first frame at or after its time.
  assert.deepEqual(plain(before.slice(0, 2)), [Math.ceil(3.2345 * fps), Math.ceil(4.0123 * fps)]);
  assert.equal(api.midFrame(0, fps), 0);
  assert.equal(api.midFrame(1.2345, 0), 1.2345);
});

test("what plays at a time is one key that only changes at an edge", () => {
  const items = [
    { id: "a", start: 0, end: 2 },
    { id: "b", start: 1, end: 3 },
    { id: "c", start: 5, end: 6 },
  ];
  const at = (time) => api.idsAt(items, time, (item) => item.id);
  assert.equal(at(0.5), "a");
  // Overlapping, both; the same key all along, so nothing derived from it changes.
  assert.equal(at(1.2), "a\nb");
  assert.equal(at(1.9), at(1.2));
  // An end is not part of it.
  assert.equal(at(2), "b");
  assert.equal(at(4), "");
  assert.equal(at(5), "c");
});

// ---- Audio tracks -----------------------------------------------------------

const music = (start, duration, more = {}) => ({ kind: "file", audio_id: 9, source_start: 1, volume: 0.5, ...more, start, duration });

function withAudio(t, tracks) {
  return {
    ...t,
    audio_tracks: tracks.map((items, index) => ({ id: `a${index}`, items: items.map((item, at) => ({ id: `s${index}${at}`, ...item })) })),
  };
}

test("audio tracks and clip volumes are read and saved, and old timelines read as without them", () => {
  const json = JSON.stringify({
    clips: [{ id: "c", material_id: 1, start: 0, end: 9, volume: 0.25 }],
    audio_tracks: [{ id: "t", items: [{ id: "m", clip_id: "c", ...music(1, 4, { fade_in: 1.5 }) }] }],
  });
  const parsed = api.parseTimeline(json);
  assert.equal(parsed.clips[0].volume, 0.25);
  assert.deepEqual(plain(parsed.audio_tracks[0].items[0]), { id: "m", clip_id: "c", ...music(1, 4, { fade_in: 1.5 }) });
  assert.deepEqual(plain(api.parseTimeline(JSON.stringify({ clips: [] })).audio_tracks), []);
  assert.deepEqual(plain(api.emptyTimeline().audio_tracks), []);
});

test("audio is added onto the top track free, follows its clip and is heard like an overlay", () => {
  // a shows 10-20 at 0-10, b shows 0-5 at 10-15.
  let t = timeline([clip("a", 1, 10, 20), clip("b", 2, 0, 5)]);
  const first = api.addAudio(t, music(0, 0), 2, 20);
  t = first.timeline;
  // Cut short at the end of the project.
  assert.deepEqual(plain(api.placeAudio(t).map((p) => [p.item.id, p.track, p.start, p.end])), [[first.id, 0, 2, 15]]);
  const second = api.addAudio(t, { kind: "material", material_id: 3, source_start: 0, volume: 1 }, 5, 2);
  t = second.timeline;
  // The top track is taken there, so it goes on a new one below.
  assert.deepEqual(plain(api.placeAudio(t).map((p) => [p.item.id, p.track])), [[first.id, 0], [second.id, 1]]);
  assert.deepEqual(plain([...api.timelineMaterials(t)]), [1, 2, 3]);
  assert.deepEqual(plain([...api.timelineAudio(t)]), [9]);
  // Moving clip b first moves the sound that starts over a along with a.
  const moved = api.moveClip(t, "b", 0);
  assert.deepEqual(plain(api.placeAudio(moved).find((p) => p.item.id === second.id)).start, 10);
});

test("an audio item moved, trimmed or split keeps playing the same sound", () => {
  let t = withAudio(timeline([clip("a", 1, 0, 30)]), [[{ clip_id: "a", ...music(4, 10, { fade_in: 1, fade_out: 2 }) }]]);
  const id = "s00";
  // The source time heard at each project time.
  const heard = (timeline, time, item = id) => {
    const placed = api.placeAudio(timeline).find((p) => p.item.id === item && time >= p.start && time < p.end);
    return placed ? placed.item.source_start + placed.skip + time - placed.start : null;
  };
  assert.equal(heard(t, 6), 3);
  const trimmed = api.trimAudio(t, id, "start", 6, 20);
  assert.deepEqual(plain(api.placeAudio(trimmed).map((p) => [p.start, p.end])), [[6, 14]]);
  assert.equal(heard(trimmed, 6), 3);
  // No earlier than the start of its source, nor later than its end.
  assert.equal(api.placeAudio(api.trimAudio(t, id, "start", 0, 20))[0].start, 3);
  assert.equal(api.placeAudio(api.trimAudio(t, id, "end", 40, 20))[0].end, 23);
  const moved = api.moveAudio(t, id, 10, { track: 0 });
  assert.equal(heard(moved, 12), 3);
  const split = api.splitAudio(t, id, 8);
  const parts = api.placeAudio(split.timeline).map((p) => [p.start, p.end, p.item.fade_in ?? 0, p.item.fade_out ?? 0]);
  // The fade in stays before the cut and the fade out after it.
  assert.deepEqual(plain(parts), [[4, 8, 1, 0], [8, 14, 0, 2]]);
  assert.equal(api.placeAudio(split.timeline)[1].item.id, split.id);
  assert.equal(heard(split.timeline, 9, split.id), 6);
  t = api.editAudio(t, id, (item) => ({ ...item, volume: 0.1 }));
  assert.equal(api.findAudio(t, id).volume, 0.1);
  assert.equal(api.removeAudio(t, id).audio_tracks.length, 0);
});

test("audio of a deleted clip plays on over the clip after it, as the backend moves it", () => {
  // a shows 10-20 at 0-10, b shows 0-5 at 10-15.
  const t = withAudio(timeline([clip("a", 1, 10, 20), clip("b", 2, 0, 5)]), [
    [
      // Heard from 5 s into clip a.
      { clip_id: "a", ...music(15, 8) },
      // Its first 2 s are cut off with the head of clip a.
      { clip_id: "a", ...music(8, 5) },
      { clip_id: "b", ...music(1, 2) },
    ],
  ]);
  const removed = api.removeClip(t, "a");
  const items = removed.audio_tracks[0].items.map((item) => [item.clip_id, item.start, item.source_start, item.duration]);
  assert.deepEqual(plain(items), [["b", 0, 1, 8], ["b", 0, 3, 3], ["b", 1, 1, 2]]);
  // With no clip after it, the sound goes with its clip.
  assert.deepEqual(plain(api.removeClip(removed, "b").audio_tracks), []);
  // Settled onto tracks of their own, as they now overlap.
  assert.equal(api.settleTracks(removed).audio_tracks.length, 3);
});

test("splitting a clip anchors the audio after the cut to the new clip, which keeps the clip's volume", () => {
  const t = withAudio(timeline([{ ...clip("a", 1, 0, 10), volume: 0.5 }]), [[{ clip_id: "a", ...music(2, 1) }, { clip_id: "a", ...music(6, 1) }]]);
  const { timeline: split, clipId } = api.splitClip(t, 5);
  assert.deepEqual(plain(split.clips.map((c) => c.volume)), [0.5, 0.5]);
  assert.deepEqual(plain(split.audio_tracks[0].items.map((item) => item.clip_id)), ["a", clipId]);
  assert.deepEqual(plain(api.placeAudio(split).map((p) => p.start)), [2, 6]);
});

test("sounds of deleted materials and audio files leave a timeline being edited", () => {
  const base = withAudio(timeline([clip("a", 1, 0, 10), clip("b", 2, 0, 10)]), [
    [{ clip_id: "a", ...music(1, 2) }, { clip_id: "b", kind: "material", material_id: 3, source_start: 0, volume: 1, start: 1, duration: 2 }],
    [{ clip_id: "a", ...music(5, 2, { audio_id: 4 }) }],
  ]);
  // The backend deleted material 1, so clip a, and material 3 and audio file 9.
  const remote = withAudio(timeline([clip("b", 2, 0, 10)]), [[{ clip_id: "b", ...music(0, 2, { audio_id: 4 }) }]]);
  const local = api.editAudio(base, "s10", (item) => ({ ...item, volume: 1 }));
  const merged = api.mergeRemoteTimeline(local, base, remote);
  // The sound of file 4, edited here, moved onto clip b with clip a gone.
  assert.deepEqual(plain(merged.audio_tracks.map((track) => track.items.map((item) => [item.id, item.clip_id, item.volume]))), [[["s10", "b", 1]]]);
  assert.equal(api.mergeRemoteTimeline(base, base, base), base);
});

test("an audio item fades over the part heard, as export fades it", () => {
  const placed = (fade_in, fade_out) => ({ item: { ...music(0, 4), fade_in, fade_out }, track: 0, start: 2, end: 6, skip: 0 });
  const gains = (item) => [2, 2.5, 3, 4, 5.5, 6].map((time) => api.fadeGain(item, time));
  assert.deepEqual(gains(placed(1, 1)), [0, 0.5, 1, 1, 0.5, 0]);
  assert.deepEqual(gains(placed(0, 0)), [1, 1, 1, 1, 1, 1]);
  // Fades longer than the item share it: here 3 s in and 1 s out.
  assert.deepEqual(gains(placed(6, 2)), [0, 0.5 / 3, 1 / 3, 2 / 3, 0.5, 0]);
});

const kinded = (id, kind, ...items) => ({ id, kind, items });
const layout = (t) => plain(t.tracks.map((tr) => [tr.kind, tr.items.map((i) => i.id)]));

test("tracks from before kinds split into a video track with a sticker track over it", () => {
  const json = JSON.stringify({
    clips: [clip("a", 1, 0, 10)],
    tracks: [
      { id: "mixed", items: [text("x", "a", 0, 1), pip("v", "a", 2, 1), text("y", "a", 4, 1)] },
      { id: "texts", items: [text("z", "a", 0, 1)] },
      { id: "kept", kind: "video", items: [pip("w", "a", 0, 1)] },
    ],
  });
  const parsed = api.parseTimeline(json);
  assert.deepEqual(layout(parsed), [["video", ["v"]], ["sticker", ["x", "y"]], ["sticker", ["z"]], ["video", ["w"]]]);
  assert.deepEqual(plain(parsed.tracks.map((tr) => tr.id)), ["mixed", "mixeds", "texts", "kept"]);
  // Drawn in the same order as before.
  assert.deepEqual(plain(api.placeOverlays(parsed).map((p) => p.item.id)), ["v", "x", "y", "z", "w"]);
  assert.deepEqual(layout(api.parseTimeline(JSON.stringify(parsed))), layout(parsed));
});

test("overlays are added onto tracks of their kind, a new video track just over the main one", () => {
  let t = withTracks(timeline([clip("a", 1, 0, 10)]), kinded("s", "sticker", text("x", "a", 0, 2)));
  const video = { kind: "video", material_id: 9, source_start: 0, volume: 1, element: { x: 960, y: 540, width: 1920, rotation: 0 } };
  const first = api.addOverlay(t, video, 5, 1);
  t = first.timeline;
  assert.deepEqual(layout(t), [["video", [first.id]], ["sticker", ["x"]]]);
  // Free on the sticker track, but a sticker track does not take a video.
  const second = api.addOverlay(t, video, 5.5, 1);
  assert.deepEqual(layout(second.timeline), [["video", [first.id]], ["video", [second.id]], ["sticker", ["x"]]]);
  // A text skips the video tracks below for the sticker track; aimed at a
  // video track, it gets a sticker track of its own above it.
  const third = api.addOverlay(t, { kind: "text", element: { content: "t" } }, 5, 1);
  assert.deepEqual(layout(third.timeline), [["video", [first.id]], ["sticker", ["x", third.id]]]);
  const aimed = api.addOverlay(t, { kind: "text", element: { content: "t" } }, 5, 1, { track: 0 });
  assert.deepEqual(layout(aimed.timeline), [["video", [first.id]], ["sticker", [aimed.id]], ["sticker", ["x"]]]);
  assert.deepEqual(plain(api.overlayTarget(t, "sticker", { track: 0 }, 0, 1)), { insert: 1 });
  assert.deepEqual(plain(api.overlayTarget(t, "sticker", { track: 1 }, 3, 4)), { track: 1 });
  // Moved onto a video track, a text lands on a new sticker track above it.
  assert.deepEqual(layout(api.moveOverlay(t, "x", 0, { track: 0 })), [["video", [first.id]], ["sticker", ["x"]]]);
  // Lifted apart by settling, a track keeps its kind.
  const crowded = withTracks(t, kinded("v", "video", pip("p", "a", 0, 3), pip("q", "a", 1, 3)));
  assert.deepEqual(layout(api.settleOverlays(crowded)), [["video", ["p"]], ["video", ["q"]]]);
});

test("moving a layer up or down passes a track of the other kind", () => {
  const t = withTracks(
    timeline([clip("a", 1, 0, 10)]),
    kinded("v", "video", pip("p", "a", 0, 2)),
    kinded("s", "sticker", text("x", "a", 0, 2), text("y", "a", 5, 2)),
    kinded("w", "video", pip("q", "a", 5, 2)),
  );
  // The only item of its track swaps places with the sticker track.
  assert.deepEqual(layout(api.shiftOverlay(t, "p", 1)), [["sticker", ["x", "y"]], ["video", ["p"]], ["video", ["q"]]]);
  // One of two goes onto a track of its own past the neighbour.
  assert.deepEqual(layout(api.shiftOverlay(t, "y", 1)), [["video", ["p"]], ["sticker", ["x"]], ["video", ["q"]], ["sticker", ["y"]]]);
  assert.deepEqual(layout(api.shiftOverlay(t, "x", -1)), [["sticker", ["x"]], ["video", ["p"]], ["sticker", ["y"]], ["video", ["q"]]]);
  assert.equal(api.canShiftOverlay(t, "q", 1), false);
  assert.equal(api.canShiftOverlay(t, "p", -1), false);
  // A copy goes onto a new track of its kind just above.
  const copy = api.duplicateOverlay(t, "p");
  assert.deepEqual(layout(copy.timeline)[1], ["video", [copy.id]]);
});

test("a clip lifted off the main track shows on a video track and takes what is anchored to it along", () => {
  // a 0-10, b (2-8 of material 2) 10-16, c 16-26.
  const t = withAudio(
    {
      ...withTracks(
        timeline([clip("a", 1, 0, 10), { ...clip("b", 2, 2, 8), volume: 0.5 }, clip("c", 3, 0, 10)], [
          sub("sa", "a", 1, 2),
          sub("sb", "b", 3, 4),
          sub("hidden", "b", 0, 1),
        ]),
        kinded("s", "sticker", text("on-b", "b", 4, 1), text("on-c", "c", 1, 1)),
      ),
    },
    [[{ clip_id: "b", ...music(2, 3) }]],
  );
  const element = { x: 960, y: 540, width: 1920, rotation: 0 };
  const result = api.clipToOverlay(t, "b", 3, { insert: 0 }, element);
  const next = result.timeline;
  // The main track closes up: a 0-10, c 10-20.
  assert.deepEqual(plain(next.clips.map((c) => c.id)), ["a", "c"]);
  const pipSpan = api.placeOverlays(next).find((p) => p.item.id === result.id);
  assert.deepEqual(plain([pipSpan.track, pipSpan.start, pipSpan.end]), [0, 3, 9]);
  assert.deepEqual(
    plain(api.findOverlay(next, result.id)),
    { id: result.id, kind: "video", clip_id: "a", start: 3, duration: 6, material_id: 2, source_start: 2, volume: 0.5, element },
  );
  // Its subtitle and sticker show where they did over it; the hidden subtitle goes.
  assert.deepEqual(plain(api.visibleSubtitles(next).map((s) => [s.subtitle.id, s.start, s.end])), [["sa", 1, 2], ["sb", 4, 5]]);
  assert.deepEqual(spans(next).filter(([id]) => id !== result.id), [["on-b", 1, 5, 6, 0], ["on-c", 1, 11, 12, 0]]);
  assert.deepEqual(plain(api.placeAudio(next).map((p) => [p.start, p.end, p.item.source_start])), [[3, 6, 1]]);
  // The main track keeps a clip.
  assert.equal(api.canLiftClip(t, "b"), true);
  assert.equal(api.clipToOverlay(timeline([clip("a", 1, 0, 10)]), "a", 0, { insert: 0 }, element), null);
  // Nor when what stays is too short to show it over.
  const short = timeline([clip("a", 1, 0, 10), clip("b", 2, 0, 0.05)]);
  assert.equal(api.canLiftClip(short, "a"), false);
  assert.equal(api.canLiftClip(short, "b"), true);
});

test("a video overlay dropped on the main track becomes a clip of what it shows", () => {
  // v shows 21-24 of material 9 from 4 of clip a, which starts at 5: 1 s is hidden.
  const t = withTracks(timeline([clip("a", 1, 5, 15), clip("b", 2, 0, 5)]), kinded("v", "video", { ...pip("v", "a", 4, 3, 9, 20), volume: 0.3 }));
  const { timeline: next, clipId } = api.overlayToClip(t, "v", 1, 0);
  assert.deepEqual(plain(next.clips.map((c) => [c.id === clipId ? "new" : c.id, c.material_id, c.start, c.end, c.volume ?? 1])), [
    ["a", 1, 5, 15, 1],
    ["new", 9, 21, 23, 0.3],
    ["b", 2, 0, 5, 1],
  ]);
  assert.deepEqual(plain(next.tracks), []);
  assert.equal(api.overlayToClip(withTracks(t, kinded("s", "sticker", text("x", "a", 6, 1))), "x", 0, 0).clipId, null);
});
