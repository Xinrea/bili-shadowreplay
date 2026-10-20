import type {
  OverlayItem,
  OverlayTrack,
  ProjectTimeline,
  SubtitleStyle,
  TimelineClip,
  TimelineSubtitle,
  VideoOverlay,
} from "./interface";

// Project (切片) timelines, mirroring src-tauri/src/project.rs. Clips play back
// to back on one main track; subtitles are anchored to a clip in material time
// so they follow the picture when clips move or are trimmed. Overlays (texts,
// stickers and videos) sit on tracks above the main one and are anchored the
// same way, by where they start. Every edit returns a new timeline so the
// editor can keep an undo history of snapshots.
//
// With the project's frame rate, as export renders it, clips last whole
// frames: trims and splits land on frames, so an edit moves the clips after
// it by whole frames, and export finds their segments rendered before.

export const TIMELINE_VERSION = 1;
export const MIN_CLIP_DURATION = 0.1;
export const MIN_SUBTITLE_DURATION = 0.1;
export const MIN_OVERLAY_DURATION = 0.1;
/** Overlaps this short, from rounding, do not count. */
const EPSILON = 1e-6;

export interface PlacedClip {
  clip: TimelineClip;
  index: number;
  /** Project time where the clip starts. */
  offset: number;
  duration: number;
}

export interface PlacedSubtitle {
  subtitle: TimelineSubtitle;
  clip: PlacedClip;
  /** Project time of the visible part. */
  start: number;
  end: number;
}

export interface Cue {
  start: number;
  end: number;
  text: string;
}

export function newId(): string {
  const random = globalThis.crypto?.randomUUID?.() ?? `${Math.random().toString(36).slice(2)}${Date.now().toString(36)}`;
  return random.replace(/-/g, "").slice(0, 12);
}

export function emptyTimeline(style: SubtitleStyle | null = null): ProjectTimeline {
  return { version: TIMELINE_VERSION, clips: [], subtitles: [], tracks: [], subtitle_style: style, danmu: false };
}

export function parseTimeline(json: string): ProjectTimeline {
  try {
    const value = json ? JSON.parse(json) : {};
    return {
      version: value.version ?? TIMELINE_VERSION,
      clips: Array.isArray(value.clips) ? value.clips : [],
      subtitles: Array.isArray(value.subtitles) ? value.subtitles : [],
      tracks: Array.isArray(value.tracks)
        ? value.tracks.map((track: OverlayTrack) => ({ id: track.id, items: Array.isArray(track.items) ? track.items : [] }))
        : [],
      subtitle_style: value.subtitle_style ?? null,
      danmu: Boolean(value.danmu),
    };
  } catch {
    return emptyTimeline();
  }
}

/** Materials the timeline shows, on the main track or in video overlays. */
export function timelineMaterials(timeline: ProjectTimeline): Set<number> {
  const ids = new Set(timeline.clips.map((clip) => clip.material_id));
  for (const track of timeline.tracks) {
    for (const item of track.items) if (item.kind === "video") ids.add(item.material_id);
  }
  return ids;
}

export function clipDuration(clip: TimelineClip): number {
  return Math.max(0, clip.end - clip.start);
}

export function placeClips(timeline: ProjectTimeline): PlacedClip[] {
  let offset = 0;
  return timeline.clips.map((clip, index) => {
    const duration = clipDuration(clip);
    const placed = { clip, index, offset, duration };
    offset += duration;
    return placed;
  });
}

export function timelineDuration(timeline: ProjectTimeline): number {
  return timeline.clips.reduce((total, clip) => total + clipDuration(clip), 0);
}

/** The clip shown at project time `time`; at a cut, the later one. */
export function clipAt(timeline: ProjectTimeline, time: number): PlacedClip | null {
  const placed = placeClips(timeline);
  for (const item of placed) {
    if (time < item.offset + item.duration) return item;
  }
  return placed[placed.length - 1] ?? null;
}

export function toProjectTime(placed: PlacedClip, sourceTime: number): number {
  return placed.offset + sourceTime - placed.clip.start;
}

export function toSourceTime(placed: PlacedClip, projectTime: number): number {
  return placed.clip.start + projectTime - placed.offset;
}

function visibleSpan(subtitle: TimelineSubtitle, placed: PlacedClip): { start: number; end: number } | null {
  const start = Math.max(subtitle.start, placed.clip.start);
  const end = Math.min(subtitle.end, placed.clip.end);
  if (end - start <= 0) return null;
  return { start: toProjectTime(placed, start), end: toProjectTime(placed, end) };
}

/** Visible subtitles in project time, ordered by start. */
export function visibleSubtitles(timeline: ProjectTimeline): PlacedSubtitle[] {
  const clips = new Map(placeClips(timeline).map((placed) => [placed.clip.id, placed]));
  const result: PlacedSubtitle[] = [];
  for (const subtitle of timeline.subtitles) {
    const clip = clips.get(subtitle.clip_id);
    if (!clip) continue;
    const span = visibleSpan(subtitle, clip);
    if (span) result.push({ subtitle, clip, ...span });
  }
  return result.sort((a, b) => a.start - b.start || a.clip.index - b.clip.index);
}

/**
 * `time` moved into the middle of the frame before the first frame at or
 * after it, at `fps`: it shows on the same frames, and rounded to the
 * centisecond of an ASS document it cannot move onto another one, so moved by
 * whole frames it shows on as many frames later. Without a frame rate, `time`.
 */
export function midFrame(time: number, fps: number): number {
  if (!(fps > 0)) return time;
  return Math.max(0, (Math.ceil(time * fps - EPSILON) - 0.5) / fps);
}

/** Cues in project time for `buildSubtitleAss`, put between the frames at `fps`. */
export function projectCues(timeline: ProjectTimeline, fps = 0) {
  return visibleSubtitles(timeline).map(({ start, end, subtitle }, index) => ({
    startTime: midFrame(start, fps),
    endTime: midFrame(end, fps),
    text: subtitle.text,
    layerOrder: index,
  }));
}

/**
 * A length this close to whole frames is them: material lengths are rounded,
 * as is the frame rate, so a file of 360 frames at 30000/1001 FPS lasts
 * 359.9996 frames at 29.97.
 */
const FRAME_TOLERANCE = 0.01;

/** Frames at `fps` of `duration`, short of what does not make a whole frame. */
function wholeFrames(duration: number, fps: number): number {
  return Math.floor(duration * fps + FRAME_TOLERANCE);
}

/**
 * `timeline` with every clip shortened to whole frames at `fps`, by less than
 * one; `timeline` itself when they all are, or without a frame rate.
 */
export function snapClips(timeline: ProjectTimeline, fps: number): ProjectTimeline {
  if (!(fps > 0)) return timeline;
  let changed = false;
  const clips = timeline.clips.map((clip) => {
    const frames = wholeFrames(clipDuration(clip), fps);
    if (Math.abs(clipDuration(clip) * fps - frames) < EPSILON || frames < 1) return clip;
    changed = true;
    return { ...clip, end: clip.start + frames / fps };
  });
  return changed ? { ...timeline, clips } : timeline;
}

function replaceClip(timeline: ProjectTimeline, clip: TimelineClip): ProjectTimeline {
  return { ...timeline, clips: timeline.clips.map((item) => (item.id === clip.id ? clip : item)) };
}

/** Insert `clips` at `index`, each shortened to whole frames at `fps`. */
export function insertClips(
  timeline: ProjectTimeline,
  clips: Omit<TimelineClip, "id">[],
  index = timeline.clips.length,
  fps = 0,
): ProjectTimeline {
  const added = clips
    .map((clip) => (fps > 0 ? { ...clip, end: clip.start + wholeFrames(clip.end - clip.start, fps) / fps } : clip))
    .filter((clip) => clip.end - clip.start >= MIN_CLIP_DURATION)
    .map((clip) => ({ ...clip, id: newId() }));
  const next = [...timeline.clips];
  next.splice(Math.max(0, Math.min(index, next.length)), 0, ...added);
  return { ...timeline, clips: next };
}

/** Remove a clip together with the subtitles and overlays anchored to it. */
export function removeClip(timeline: ProjectTimeline, clipId: string): ProjectTimeline {
  return {
    ...timeline,
    clips: timeline.clips.filter((clip) => clip.id !== clipId),
    subtitles: timeline.subtitles.filter((subtitle) => subtitle.clip_id !== clipId),
    tracks: withoutOverlays(timeline.tracks, (item) => item.clip_id === clipId),
  };
}

/**
 * Bring the backend's own edits of a saved timeline into one being edited.
 * Between `base`, the saved timeline the editor last knew, and `remote`, the
 * saved one now, the backend may switch danmaku, remove the clips and video
 * overlays of a deleted material and fill clips that had none with recognized
 * subtitles; everything else stays as edited in `local`. Returns `local`
 * itself when nothing applies.
 */
export function mergeRemoteTimeline(
  local: ProjectTimeline,
  base: ProjectTimeline,
  remote: ProjectTimeline,
): ProjectTimeline {
  let next = local;
  if (remote.danmu !== local.danmu) next = { ...next, danmu: remote.danmu };

  const remaining = timelineMaterials(remote);
  const removed = new Set([...timelineMaterials(base)].filter((id) => !remaining.has(id)));
  if ([...timelineMaterials(next)].some((id) => removed.has(id))) {
    const clips = next.clips.filter((clip) => !removed.has(clip.material_id));
    const kept = new Set(clips.map((clip) => clip.id));
    next = {
      ...next,
      clips,
      subtitles: next.subtitles.filter((subtitle) => kept.has(subtitle.clip_id)),
      tracks: withoutOverlays(
        next.tracks,
        (item) => !kept.has(item.clip_id) || (item.kind === "video" && removed.has(item.material_id)),
      ),
    };
  }

  const hadSubtitles = new Set(base.subtitles.map((subtitle) => subtitle.clip_id));
  const hasSubtitles = new Set(next.subtitles.map((subtitle) => subtitle.clip_id));
  const clipIds = new Set(next.clips.map((clip) => clip.id));
  const filled = remote.subtitles.filter(
    (subtitle) =>
      clipIds.has(subtitle.clip_id) && !hadSubtitles.has(subtitle.clip_id) && !hasSubtitles.has(subtitle.clip_id),
  );
  if (filled.length > 0) next = { ...next, subtitles: [...next.subtitles, ...filled] };
  return next;
}

/**
 * `current` with subtitle `id` as it is in `base`: how the timeline was before
 * a drag of that subtitle, keeping changes merged in while dragging.
 */
export function withSubtitleFrom(current: ProjectTimeline, base: ProjectTimeline, id: string): ProjectTimeline {
  const original = base.subtitles.find((subtitle) => subtitle.id === id);
  if (!original) return current;
  return { ...current, subtitles: current.subtitles.map((subtitle) => (subtitle.id === id ? original : subtitle)) };
}

/** Move a clip so that it ends up at `toIndex` of the main track. */
export function moveClip(timeline: ProjectTimeline, clipId: string, toIndex: number): ProjectTimeline {
  const from = timeline.clips.findIndex((clip) => clip.id === clipId);
  if (from < 0) return timeline;
  const clips = [...timeline.clips];
  const [clip] = clips.splice(from, 1);
  clips.splice(Math.max(0, Math.min(toIndex, clips.length)), 0, clip);
  return { ...timeline, clips };
}

/**
 * Split the clip under project time `time`, at the frame nearest to it at
 * `fps`. Subtitles after the cut move to the new clip; one crossing the cut is
 * kept on both sides so it still reads as one. Overlays starting after the cut
 * are anchored to the new clip.
 */
export function splitClip(
  timeline: ProjectTimeline,
  time: number,
  fps = 0,
): { timeline: ProjectTimeline; clipId: string | null } {
  const placed = placeClips(timeline).find(
    (item) => time - item.offset >= MIN_CLIP_DURATION && item.offset + item.duration - time >= MIN_CLIP_DURATION,
  );
  if (!placed) return { timeline, clipId: null };
  const into = fps > 0 ? Math.round((time - placed.offset) * fps) / fps : time - placed.offset;
  if (into < MIN_CLIP_DURATION || placed.duration - into < MIN_CLIP_DURATION) return { timeline, clipId: null };
  const cut = placed.clip.start + into;
  const left = { ...placed.clip, end: cut };
  const right = { id: newId(), material_id: placed.clip.material_id, start: cut, end: placed.clip.end };
  const clips = [...timeline.clips];
  clips.splice(placed.index, 1, left, right);
  const subtitles: TimelineSubtitle[] = [];
  for (const subtitle of timeline.subtitles) {
    if (subtitle.clip_id !== placed.clip.id) {
      subtitles.push(subtitle);
    } else if (subtitle.start >= cut) {
      subtitles.push({ ...subtitle, clip_id: right.id });
    } else {
      subtitles.push(subtitle);
      if (subtitle.end > cut) subtitles.push({ ...subtitle, id: newId(), clip_id: right.id });
    }
  }
  const after = (item: OverlayItem) => item.clip_id === placed.clip.id && item.start >= cut;
  const tracks = timeline.tracks.map((track) =>
    track.items.some(after)
      ? { ...track, items: track.items.map((item) => (after(item) ? { ...item, clip_id: right.id } : item)) }
      : track,
  );
  return { timeline: { ...timeline, clips, subtitles, tracks }, clipId: right.id };
}

/**
 * Move one edge of a clip to `sourceTime`, keeping it within the material.
 * With a frame rate `fps`, the clip then lasts the whole frames nearest.
 */
export function trimClip(
  timeline: ProjectTimeline,
  clipId: string,
  edge: "start" | "end",
  sourceTime: number,
  materialDuration: number,
  fps = 0,
): ProjectTimeline {
  const clip = timeline.clips.find((item) => item.id === clipId);
  if (!clip) return timeline;
  const shortest = fps > 0 ? Math.ceil(MIN_CLIP_DURATION * fps - EPSILON) / fps : MIN_CLIP_DURATION;
  // The length in whole frames nearest to `length`, but no more than `longest`.
  const frames = (length: number, longest: number) =>
    fps > 0 ? Math.min(Math.round(length * fps), wholeFrames(longest, fps)) / fps : Math.min(length, longest);
  if (edge === "start") {
    const length = frames(Math.max(clip.end - sourceTime, shortest), clip.end);
    return replaceClip(timeline, { ...clip, start: Math.max(0, clip.end - length) });
  }
  const limit = materialDuration > 0 ? materialDuration : Number.POSITIVE_INFINITY;
  const length = frames(Math.max(sourceTime - clip.start, shortest), limit - clip.start);
  return replaceClip(timeline, { ...clip, end: clip.start + length });
}

/** Replace a clip's subtitles with material-time cues. */
export function setClipSubtitles(timeline: ProjectTimeline, clipId: string, cues: Cue[]): ProjectTimeline {
  return {
    ...timeline,
    subtitles: [
      ...timeline.subtitles.filter((subtitle) => subtitle.clip_id !== clipId),
      ...cues
        .filter((cue) => cue.end > cue.start)
        .map((cue) => ({ id: newId(), clip_id: clipId, start: cue.start, end: cue.end, text: cue.text })),
    ],
  };
}

function anchor(timeline: ProjectTimeline, start: number, end: number) {
  const total = timelineDuration(timeline);
  const placed = clipAt(timeline, Math.max(0, Math.min(start, total - MIN_SUBTITLE_DURATION)));
  if (!placed) return null;
  const sourceStart = toSourceTime(placed, Math.max(start, placed.offset));
  const sourceEnd = Math.max(sourceStart + MIN_SUBTITLE_DURATION, sourceStart + (end - start));
  return { clip_id: placed.clip.id, start: sourceStart, end: sourceEnd };
}

/** Add a subtitle at project times, anchored to the clip where it starts. */
export function addSubtitle(
  timeline: ProjectTimeline,
  start: number,
  end: number,
  text = "",
): { timeline: ProjectTimeline; id: string | null } {
  const anchored = anchor(timeline, start, end);
  if (!anchored) return { timeline, id: null };
  const id = newId();
  return { timeline: { ...timeline, subtitles: [...timeline.subtitles, { id, text, ...anchored }] }, id };
}

/** Move a subtitle to start at project time `start`, re-anchoring it to the clip there. */
export function moveSubtitle(timeline: ProjectTimeline, id: string, start: number, end?: number): ProjectTimeline {
  const subtitle = timeline.subtitles.find((item) => item.id === id);
  if (!subtitle) return timeline;
  const length = end === undefined ? subtitle.end - subtitle.start : end - start;
  const anchored = anchor(timeline, Math.max(0, start), Math.max(0, start) + length);
  if (!anchored) return timeline;
  return {
    ...timeline,
    subtitles: timeline.subtitles.map((item) => (item.id === id ? { ...item, ...anchored } : item)),
  };
}

/** Move one edge of a subtitle to project time `time` within its clip. */
export function resizeSubtitle(
  timeline: ProjectTimeline,
  id: string,
  edge: "start" | "end",
  time: number,
): ProjectTimeline {
  const subtitle = timeline.subtitles.find((item) => item.id === id);
  const placed = placeClips(timeline).find((item) => item.clip.id === subtitle?.clip_id);
  if (!subtitle || !placed) return timeline;
  const clamped = Math.max(placed.offset, Math.min(time, placed.offset + placed.duration));
  const source = toSourceTime(placed, clamped);
  const next =
    edge === "start"
      ? { ...subtitle, start: Math.min(source, subtitle.end - MIN_SUBTITLE_DURATION) }
      : { ...subtitle, end: Math.max(source, subtitle.start + MIN_SUBTITLE_DURATION) };
  return { ...timeline, subtitles: timeline.subtitles.map((item) => (item.id === id ? next : item)) };
}

export function updateSubtitleText(timeline: ProjectTimeline, id: string, text: string): ProjectTimeline {
  return {
    ...timeline,
    subtitles: timeline.subtitles.map((item) => (item.id === id ? { ...item, text } : item)),
  };
}

export function removeSubtitle(timeline: ProjectTimeline, id: string): ProjectTimeline {
  return { ...timeline, subtitles: timeline.subtitles.filter((item) => item.id !== id) };
}

// ---- Overlays ---------------------------------------------------------------

export interface PlacedOverlay {
  item: OverlayItem;
  /** Index of its track, the bottom one first. */
  track: number;
  /** The visible part, in project time. */
  start: number;
  end: number;
  /**
   * How much of the item is hidden before `start`, cut off with the head of
   * its clip; a video overlay skips as much of its material.
   */
  skip: number;
}

/** Where an item goes: onto an existing track, or onto a new one inserted at that index. */
export type OverlayTarget = { track: number } | { insert: number };

type Content<T> = T extends OverlayItem ? Omit<T, "id" | "clip_id" | "start" | "duration"> : never;
/** An overlay without its place on the timeline. */
export type OverlayContent = Content<OverlayItem>;

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function overlaps(a: { start: number; end: number }, b: { start: number; end: number }) {
  return a.start < b.end - EPSILON && b.start < a.end - EPSILON;
}

function overlaySpan(item: OverlayItem, clip: PlacedClip | undefined, total: number) {
  // Anchored past the out point, it was trimmed away with the end of its clip.
  if (!clip || item.start >= clip.clip.end) return null;
  const from = toProjectTime(clip, item.start);
  const start = Math.max(from, clip.offset);
  const end = Math.min(from + item.duration, total);
  if (end - start <= EPSILON) return null;
  return { start, end, skip: start - from };
}

/** Visible overlays, the bottom track first and each track's items in order. */
export function placeOverlays(timeline: ProjectTimeline): PlacedOverlay[] {
  const clips = new Map(placeClips(timeline).map((placed) => [placed.clip.id, placed]));
  const total = timelineDuration(timeline);
  const result: PlacedOverlay[] = [];
  timeline.tracks.forEach((track, index) => {
    for (const item of track.items) {
      const span = overlaySpan(item, clips.get(item.clip_id), total);
      if (span) result.push({ item, track: index, ...span });
    }
  });
  return result;
}

export function overlayCount(timeline: ProjectTimeline): number {
  return timeline.tracks.reduce((total, track) => total + track.items.length, 0);
}

export function findOverlay(timeline: ProjectTimeline, id: string | null): OverlayItem | null {
  if (!id) return null;
  for (const track of timeline.tracks) {
    const item = track.items.find((entry) => entry.id === id);
    if (item) return item;
  }
  return null;
}

/** Whether `start..end` is free on `track`, apart from item `ignore`. */
export function trackIsFree(
  placed: PlacedOverlay[],
  track: number,
  start: number,
  end: number,
  ignore: string | null = null,
): boolean {
  return !placed.some((other) => other.track === track && other.item.id !== ignore && overlaps(other, { start, end }));
}

/** `target`, or a new track just above it when it is taken over `start..end`. */
function freeTarget(placed: PlacedOverlay[], target: OverlayTarget, start: number, end: number, ignore: string | null) {
  if ("track" in target && !trackIsFree(placed, target.track, start, end, ignore)) return { insert: target.track + 1 };
  return target;
}

/** The lowest track free over `start..end`, else a new one on top. */
function lowestFree(placed: PlacedOverlay[], trackCount: number, start: number, end: number): OverlayTarget {
  for (let track = 0; track < trackCount; track++) {
    if (trackIsFree(placed, track, start, end)) return { track };
  }
  return { insert: trackCount };
}

/** The anchor of an item starting at project time `time`. */
function anchorAt(timeline: ProjectTimeline, time: number): { clip_id: string; start: number } | null {
  const placed = clipAt(timeline, time);
  if (!placed) return null;
  return { clip_id: placed.clip.id, start: toSourceTime(placed, Math.max(time, placed.offset)) };
}

/**
 * The item of `placed` anchored to start at project time `start` and run for
 * `length`; a video then shows its material from `shift` past what it showed.
 */
function reanchor(timeline: ProjectTimeline, placed: PlacedOverlay, start: number, length: number, shift = 0) {
  const anchored = anchorAt(timeline, start);
  if (!anchored) return null;
  const item = { ...placed.item, ...anchored, duration: length } as OverlayItem;
  if (item.kind === "video") item.source_start = (placed.item as VideoOverlay).source_start + placed.skip + shift;
  return item;
}

function putOverlay(tracks: OverlayTrack[], item: OverlayItem, target: OverlayTarget): OverlayTrack[] {
  if ("track" in target && tracks[target.track]) {
    return tracks.map((track, index) => (index === target.track ? { ...track, items: [...track.items, item] } : track));
  }
  const next = [...tracks];
  next.splice(clamp("insert" in target ? target.insert : tracks.length, 0, tracks.length), 0, { id: newId(), items: [item] });
  return next;
}

/** `tracks` without the items `drop` picks, and without tracks left empty. */
function withoutOverlays(tracks: OverlayTrack[], drop: (item: OverlayItem) => boolean): OverlayTrack[] {
  return tracks
    .map((track) => (track.items.some(drop) ? { ...track, items: track.items.filter((item) => !drop(item)) } : track))
    .filter((track) => track.items.length > 0);
}

function replaceOverlay(timeline: ProjectTimeline, item: OverlayItem): ProjectTimeline {
  return {
    ...timeline,
    tracks: timeline.tracks.map((track) =>
      track.items.some((entry) => entry.id === item.id)
        ? { ...track, items: track.items.map((entry) => (entry.id === item.id ? item : entry)) }
        : track,
    ),
  };
}

/**
 * Add an item over project time `start..start + duration`, cut short at the
 * end of the project, on `target` or else the lowest track free there.
 */
export function addOverlay(
  timeline: ProjectTimeline,
  content: OverlayContent,
  start: number,
  duration: number,
  target?: OverlayTarget,
): { timeline: ProjectTimeline; id: string | null } {
  const total = timelineDuration(timeline);
  const from = clamp(start, 0, total - MIN_OVERLAY_DURATION);
  const length = Math.min(duration, total - from);
  const anchored = length >= MIN_OVERLAY_DURATION ? anchorAt(timeline, from) : null;
  if (!anchored) return { timeline, id: null };
  const id = newId();
  const item = { ...content, id, ...anchored, duration: length } as OverlayItem;
  const placed = placeOverlays(timeline);
  const to = target
    ? freeTarget(placed, target, from, from + length, null)
    : lowestFree(placed, timeline.tracks.length, from, from + length);
  return { timeline: { ...timeline, tracks: putOverlay(timeline.tracks, item, to) }, id };
}

/** Change an item, such as its element, in place. */
export function editOverlay(
  timeline: ProjectTimeline,
  id: string,
  update: (item: OverlayItem) => OverlayItem,
): ProjectTimeline {
  const item = findOverlay(timeline, id);
  return item ? replaceOverlay(timeline, update(item)) : timeline;
}

export function removeOverlay(timeline: ProjectTimeline, id: string): ProjectTimeline {
  return { ...timeline, tracks: withoutOverlays(timeline.tracks, (item) => item.id === id) };
}

/**
 * Move the visible part of an item to start at project time `start` on
 * `target`, re-anchoring it to the clip there. A track taken at the new place
 * gets a new track above it instead.
 */
export function moveOverlay(
  timeline: ProjectTimeline,
  id: string,
  start: number,
  target: OverlayTarget,
): ProjectTimeline {
  const placed = placeOverlays(timeline);
  const current = placed.find((entry) => entry.item.id === id);
  if (!current) return timeline;
  const length = current.end - current.start;
  const from = clamp(start, 0, timelineDuration(timeline) - length);
  const item = reanchor(timeline, current, from, length);
  if (!item) return timeline;
  const to = freeTarget(placed, target, from, from + length, id);
  const tracks = putOverlay(
    timeline.tracks.map((track) => ({ ...track, items: track.items.filter((entry) => entry.id !== id) })),
    item,
    to,
  );
  return { ...timeline, tracks: tracks.filter((track) => track.items.length > 0) };
}

/**
 * Move one edge of an item to project time `time`, short of its neighbours on
 * the track and the end of the project. A video overlay's edges stay within
 * its material, `materialDuration` long.
 */
export function trimOverlay(
  timeline: ProjectTimeline,
  id: string,
  edge: "start" | "end",
  time: number,
  materialDuration = 0,
): ProjectTimeline {
  const placed = placeOverlays(timeline);
  const current = placed.find((entry) => entry.item.id === id);
  if (!current) return timeline;
  const neighbours = placed.filter((entry) => entry.track === current.track && entry.item.id !== id);
  const video = current.item.kind === "video" ? current.item : null;
  // Material time shown at the visible start.
  const source = video ? video.source_start + current.skip : 0;
  let item: OverlayItem | null;
  if (edge === "start") {
    const before = Math.max(0, ...neighbours.filter((entry) => entry.end <= current.start + EPSILON).map((entry) => entry.end));
    const earliest = video ? current.start - source : 0;
    const start = clamp(time, Math.max(before, earliest), current.end - MIN_OVERLAY_DURATION);
    item = reanchor(timeline, current, start, current.end - start, start - current.start);
  } else {
    const after = Math.min(
      timelineDuration(timeline),
      ...neighbours.filter((entry) => entry.start >= current.end - EPSILON).map((entry) => entry.start),
    );
    const latest = video && materialDuration > 0 ? current.start + materialDuration - source : Number.POSITIVE_INFINITY;
    const end = clamp(time, current.start + MIN_OVERLAY_DURATION, Math.min(after, latest));
    item = reanchor(timeline, current, current.start, end - current.start);
  }
  return item ? replaceOverlay(timeline, item) : timeline;
}

/** Split an item at project time `time`; the part after the cut becomes a new item beside it. */
export function splitOverlay(
  timeline: ProjectTimeline,
  id: string,
  time: number,
): { timeline: ProjectTimeline; id: string | null } {
  const current = placeOverlays(timeline).find((entry) => entry.item.id === id);
  if (!current || time - current.start < MIN_OVERLAY_DURATION || current.end - time < MIN_OVERLAY_DURATION) {
    return { timeline, id: null };
  }
  const anchored = anchorAt(timeline, time);
  if (!anchored) return { timeline, id: null };
  // Where the whole item starts, including any part hidden with its clip's head.
  const from = current.start - current.skip;
  const left = { ...current.item, duration: time - from } as OverlayItem;
  const right = {
    ...current.item,
    element: { ...current.item.element },
    id: newId(),
    ...anchored,
    duration: from + current.item.duration - time,
  } as OverlayItem;
  if (right.kind === "video") right.source_start = (current.item as VideoOverlay).source_start + time - from;
  const tracks = timeline.tracks.map((track) => {
    const index = track.items.findIndex((entry) => entry.id === id);
    if (index < 0) return track;
    const items = [...track.items];
    items.splice(index, 1, left, right);
    return { ...track, items };
  });
  return { timeline: { ...timeline, tracks }, id: right.id };
}

/** Where moving an item one layer up or down puts it, or null at the top or bottom. */
function shiftTarget(timeline: ProjectTimeline, placed: PlacedOverlay[], id: string, step: 1 | -1) {
  const current = placed.find((entry) => entry.item.id === id);
  if (!current) return null;
  const neighbour = current.track + step;
  if (neighbour < 0 || neighbour >= timeline.tracks.length) {
    if (timeline.tracks[current.track].items.length === 1) return null;
    return { current, target: { insert: step > 0 ? timeline.tracks.length : 0 } };
  }
  if (trackIsFree(placed, neighbour, current.start, current.end, id)) return { current, target: { track: neighbour } };
  // Between its track and the neighbouring one.
  return { current, target: { insert: step > 0 ? neighbour : current.track } };
}

export function canShiftOverlay(timeline: ProjectTimeline, id: string, step: 1 | -1): boolean {
  return shiftTarget(timeline, placeOverlays(timeline), id, step) !== null;
}

/** Move an item onto the track above or below, or a new one between when that is taken. */
export function shiftOverlay(timeline: ProjectTimeline, id: string, step: 1 | -1): ProjectTimeline {
  const shift = shiftTarget(timeline, placeOverlays(timeline), id, step);
  return shift ? moveOverlay(timeline, id, shift.current.start, shift.target) : timeline;
}

/** A copy of an item, changed by `update`, on a new track just above it. */
export function duplicateOverlay(
  timeline: ProjectTimeline,
  id: string,
  update: (item: OverlayItem) => OverlayItem = (item) => item,
): { timeline: ProjectTimeline; id: string | null } {
  const current = placeOverlays(timeline).find((entry) => entry.item.id === id);
  if (!current) return { timeline, id: null };
  const copy = { ...update(current.item), id: newId() };
  return { timeline: { ...timeline, tracks: putOverlay(timeline.tracks, copy, { insert: current.track + 1 }) }, id: copy.id };
}

/**
 * Lift items that overlap others on their track, as moving or trimming clips
 * can leave them, onto new tracks just above, and drop empty tracks. Returns
 * `timeline` itself when nothing changes.
 */
export function settleOverlays(timeline: ProjectTimeline): ProjectTimeline {
  const spans = new Map(placeOverlays(timeline).map((entry) => [entry.item.id, entry]));
  const startOf = (item: OverlayItem) => spans.get(item.id)?.start ?? Number.NEGATIVE_INFINITY;
  let changed = false;
  const tracks: OverlayTrack[] = [];
  for (const track of timeline.tracks) {
    if (track.items.length === 0) changed = true;
    let current = track;
    while (current.items.length > 0) {
      const kept: PlacedOverlay[] = [];
      const lifted = new Set<string>();
      for (const item of [...current.items].sort((a, b) => startOf(a) - startOf(b))) {
        const span = spans.get(item.id);
        // Hidden items take no time.
        if (!span) continue;
        if (kept.some((other) => overlaps(other, span))) lifted.add(item.id);
        else kept.push(span);
      }
      if (lifted.size === 0) {
        tracks.push(current);
        break;
      }
      changed = true;
      tracks.push({ ...current, items: current.items.filter((item) => !lifted.has(item.id)) });
      current = { id: newId(), items: current.items.filter((item) => lifted.has(item.id)) };
    }
  }
  return changed ? { ...timeline, tracks } : timeline;
}

/**
 * `current` with overlay `id` as it is in `base`: how the timeline was before
 * a drag of that overlay, keeping changes merged in while dragging.
 */
export function withOverlayFrom(current: ProjectTimeline, base: ProjectTimeline, id: string): ProjectTimeline {
  const original = findOverlay(base, id);
  return original ? editOverlay(current, id, () => original) : current;
}

function parseSrtTime(value: string): number {
  const match = /(\d+):(\d+):(\d+)[,.](\d+)/.exec(value);
  if (!match) return Number.NaN;
  const [, hours, minutes, seconds, fraction] = match;
  return Number(hours) * 3600 + Number(minutes) * 60 + Number(seconds) + Number(fraction.padEnd(3, "0").slice(0, 3)) / 1000;
}

export function parseSrt(srt: string): Cue[] {
  return srt
    .replace(/\r\n?/g, "\n")
    .split(/\n\s*\n/)
    .map((block) => {
      const lines = block.split("\n").filter((line) => line.trim());
      const timing = lines.findIndex((line) => line.includes("-->"));
      if (timing < 0) return null;
      const [start, end] = lines[timing].split(/\s*-->\s*/).map(parseSrtTime);
      return { start, end, text: lines.slice(timing + 1).join("\n") };
    })
    .filter((cue): cue is Cue => cue !== null && Number.isFinite(cue.start) && cue.end > cue.start);
}
