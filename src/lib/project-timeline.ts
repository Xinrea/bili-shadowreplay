import type {
  AudioItem,
  AudioTrack,
  OverlayItem,
  OverlayTrack,
  ProjectTimeline,
  RenderSettings,
  SubtitleStyle,
  TimelineClip,
  TimelineSubtitle,
  TrackKind,
  VideoOverlay,
  VideoPlacement,
} from "./interface";

// Project (切片) timelines, mirroring src-tauri/src/project.rs. Clips play back
// to back on the main video track; subtitles, on one track above everything,
// are anchored to a clip in material time so they follow the picture when
// clips move or are trimmed. Overlays sit on tracks between the two, in any
// order: video tracks show other materials, sticker tracks texts and
// stickers. They are anchored the same way, by where they start, as are audio
// items (music, voiceover, the sound of a material) on tracks below the main
// one. They start over the main track but may run on past its end, which the
// project then lasts until. A clip moves off the main track onto a video
// track, and a video overlay onto the main track, as one kind of video track
// to another. Every edit returns a new timeline so the editor can keep an
// undo history of snapshots.
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

export function defaultRenderSettings(): RenderSettings {
  return { codec: "hevc", priority: "speed", bitrate_kbps: 10000 };
}

/** Read timelines saved before codec and encoding priority were configurable. */
export function normalizeRenderSettings(value?: Partial<RenderSettings> & { quality?: string } | null): RenderSettings {
  const defaults = defaultRenderSettings();
  const legacy = value?.codec == null && value?.priority == null;
  return {
    codec: value?.codec ?? defaults.codec,
    priority: value?.priority ?? (legacy && value?.quality === "high" ? "quality" : defaults.priority),
    bitrate_kbps: legacy && value?.bitrate_kbps === 0
      ? defaults.bitrate_kbps
      : value?.bitrate_kbps ?? defaults.bitrate_kbps,
  };
}

export function emptyTimeline(style: SubtitleStyle | null = null): ProjectTimeline {
  return {
    version: TIMELINE_VERSION,
    clips: [],
    subtitles: [],
    tracks: [],
    subtitle_style: style,
    danmu: false,
    render_settings: defaultRenderSettings(),
    audio_tracks: [],
  };
}

export function parseTimeline(json: string): ProjectTimeline {
  try {
    const value = json ? JSON.parse(json) : {};
    return {
      version: value.version ?? TIMELINE_VERSION,
      clips: Array.isArray(value.clips) ? value.clips : [],
      subtitles: Array.isArray(value.subtitles) ? value.subtitles : [],
      tracks: Array.isArray(value.tracks) ? kindTracks(value.tracks) : [],
      subtitle_style: value.subtitle_style ?? null,
      danmu: Boolean(value.danmu),
      render_settings: normalizeRenderSettings(value.render_settings),
      audio_tracks: Array.isArray(value.audio_tracks)
        ? value.audio_tracks.map((track: AudioTrack) => ({
            id: track.id,
            items: Array.isArray(track.items) ? track.items : [],
          }))
        : [],
    };
  } catch {
    return emptyTimeline();
  }
}

/** The kind of track an overlay goes on. */
export function trackKindOf(item: OverlayItem): TrackKind {
  return item.kind === "video" ? "video" : "sticker";
}

/**
 * Tracks as read, each of one kind. One from before tracks had kinds may hold
 * videos with texts and stickers: it splits into a video track with a sticker
 * track over it, which draw as it did since its items never overlap in time.
 */
function kindTracks(tracks: Partial<OverlayTrack>[]): OverlayTrack[] {
  return tracks.flatMap((track) => {
    const items = Array.isArray(track.items) ? track.items : [];
    const id = track.id ?? newId();
    const own: TrackKind | null = track.kind === "video" || track.kind === "sticker" ? track.kind : null;
    const kinds: TrackKind[] = own === "sticker" ? ["sticker", "video"] : ["video", "sticker"];
    const parts = kinds
      .map((kind) => ({ kind, items: items.filter((item) => trackKindOf(item) === kind) }))
      .filter((part) => part.items.length > 0);
    if (parts.length === 0) return [{ id, kind: own ?? "sticker", items }];
    return parts.map((part, index) => ({ id: index === 0 ? id : `${id}${part.kind[0]}`, ...part }));
  });
}

/** Materials the timeline shows, on the main track or in video overlays, or plays on audio tracks. */
export function timelineMaterials(timeline: ProjectTimeline): Set<number> {
  const ids = new Set(timeline.clips.map((clip) => clip.material_id));
  for (const track of timeline.tracks) {
    for (const item of track.items) if (item.kind === "video") ids.add(item.material_id);
  }
  for (const track of timeline.audio_tracks ?? []) {
    for (const item of track.items) if (item.kind === "material") ids.add(item.material_id);
  }
  return ids;
}

/** Files of the audio library the timeline plays. */
export function timelineAudio(timeline: ProjectTimeline): Set<number> {
  const ids = new Set<number>();
  for (const track of timeline.audio_tracks ?? []) {
    for (const item of track.items) if (item.kind === "file") ids.add(item.audio_id);
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

/** How long the clips of the main track last together. */
export function mainDuration(timeline: ProjectTimeline): number {
  return timeline.clips.reduce((total, clip) => total + clipDuration(clip), 0);
}

/** How long the project lasts: until the end of the main track, or of what runs on past it. */
export function timelineDuration(timeline: ProjectTimeline): number {
  let total = mainDuration(timeline);
  for (const lane of ["tracks", "audio_tracks"] as const) {
    for (const placed of placeItems(timeline, lane)) total = Math.max(total, placed.end);
  }
  return total;
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

/**
 * The ids of `items` shown at project time `time`, as one key that changes
 * only when they do: derived from it, the thousands of rows and blocks that
 * mark what plays update when the playhead crosses an edge, not every frame.
 */
export function idsAt<T extends { start: number; end: number }>(
  items: readonly T[],
  time: number,
  id: (item: T) => string,
): string {
  let key = "";
  for (const item of items) {
    if (time >= item.start && time < item.end) key += (key ? "\n" : "") + id(item);
  }
  return key;
}

/**
 * Cues in project time for `buildSubtitleAss`, put between the frames at
 * `fps`, the project's frame rate: every document burned must use it, or its
 * subtitles fall on other frames than the editor's.
 */
export function projectCues(timeline: ProjectTimeline, fps: number) {
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

/** What the project's frame rate depends on, for `get_project_frame_rate`. */
export function frameRateQuery(timeline: ProjectTimeline): { materialIds: number[]; danmu: boolean } {
  const materialIds = [...new Set(timeline.clips.map((clip) => clip.material_id))].sort((a, b) => a - b);
  return { materialIds, danmu: timeline.danmu };
}

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
  index: number,
  fps: number,
): ProjectTimeline {
  const added = clips
    .map((clip) => (fps > 0 ? { ...clip, end: clip.start + wholeFrames(clip.end - clip.start, fps) / fps } : clip))
    .filter((clip) => clip.end - clip.start >= MIN_CLIP_DURATION)
    .map((clip) => ({ ...clip, id: newId() }));
  const next = [...timeline.clips];
  next.splice(Math.max(0, Math.min(index, next.length)), 0, ...added);
  return { ...timeline, clips: next };
}

/**
 * Remove a clip together with the subtitles and overlays anchored to it. The
 * audio anchored to it plays on over the clip after it.
 */
export function removeClip(timeline: ProjectTimeline, clipId: string): ProjectTimeline {
  return {
    ...timeline,
    clips: timeline.clips.filter((clip) => clip.id !== clipId),
    subtitles: timeline.subtitles.filter((subtitle) => subtitle.clip_id !== clipId),
    tracks: withoutItems(timeline.tracks, (item) => item.clip_id === clipId),
    audio_tracks: reanchorAudio(timeline, new Set([clipId])),
  };
}

/**
 * Bring the backend's own edits of a saved timeline into one being edited.
 * Between `base`, the saved timeline the editor last knew, and `remote`, the
 * saved one now, the backend may switch danmaku, remove the clips, video
 * overlays and sounds of a deleted material, remove the sounds of a deleted
 * file of the audio library and fill clips that had none with recognized
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
  if (JSON.stringify(remote.render_settings) !== JSON.stringify(local.render_settings)) {
    next = { ...next, render_settings: remote.render_settings };
  }

  const remaining = timelineMaterials(remote);
  const removed = new Set([...timelineMaterials(base)].filter((id) => !remaining.has(id)));
  if ([...timelineMaterials(next)].some((id) => removed.has(id))) {
    const clips = next.clips.filter((clip) => !removed.has(clip.material_id));
    const kept = new Set(clips.map((clip) => clip.id));
    const gone = new Set(next.clips.filter((clip) => !kept.has(clip.id)).map((clip) => clip.id));
    next = {
      ...next,
      clips,
      subtitles: next.subtitles.filter((subtitle) => kept.has(subtitle.clip_id)),
      tracks: withoutItems(
        next.tracks,
        (item) => !kept.has(item.clip_id) || (item.kind === "video" && removed.has(item.material_id)),
      ),
      audio_tracks: withoutItems(
        reanchorAudio(next, gone),
        (item) => item.kind === "material" && removed.has(item.material_id),
      ),
    };
  }

  const remainingAudio = timelineAudio(remote);
  const removedAudio = new Set([...timelineAudio(base)].filter((id) => !remainingAudio.has(id)));
  if ([...timelineAudio(next)].some((id) => removedAudio.has(id))) {
    next = {
      ...next,
      audio_tracks: withoutItems(next.audio_tracks, (item) => item.kind === "file" && removedAudio.has(item.audio_id)),
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
 * kept on both sides so it still reads as one. Overlays and audio starting
 * after the cut are anchored to the new clip.
 */
export function splitClip(
  timeline: ProjectTimeline,
  time: number,
  fps: number,
): { timeline: ProjectTimeline; clipId: string | null } {
  const placed = placeClips(timeline).find(
    (item) => time - item.offset >= MIN_CLIP_DURATION && item.offset + item.duration - time >= MIN_CLIP_DURATION,
  );
  if (!placed) return { timeline, clipId: null };
  const into = fps > 0 ? Math.round((time - placed.offset) * fps) / fps : time - placed.offset;
  if (into < MIN_CLIP_DURATION || placed.duration - into < MIN_CLIP_DURATION) return { timeline, clipId: null };
  const cut = placed.clip.start + into;
  const left = { ...placed.clip, end: cut };
  const right = { ...placed.clip, id: newId(), start: cut };
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
  const after = (item: TrackItem) => item.clip_id === placed.clip.id && item.start >= cut;
  const moved = <R extends Track<TrackItem>>(tracks: R[]): R[] =>
    tracks.map((track) =>
      track.items.some(after)
        ? { ...track, items: track.items.map((item) => (after(item) ? { ...item, clip_id: right.id } : item)) }
        : track,
    );
  const tracks = moved(timeline.tracks);
  const audio_tracks = moved(timeline.audio_tracks ?? []);
  return { timeline: { ...timeline, clips, subtitles, tracks, audio_tracks }, clipId: right.id };
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
  fps: number,
): ProjectTimeline {
  const clip = timeline.clips.find((item) => item.id === clipId);
  if (!clip) return timeline;
  const shortest = fps > 0 ? Math.ceil(MIN_CLIP_DURATION * fps - EPSILON) / fps : MIN_CLIP_DURATION;
  // The length in whole frames nearest to `length`, at least one, but no more
  // than `longest`: with less than a frame of material left, all of it.
  const frames = (length: number, longest: number) => {
    const most = fps > 0 ? wholeFrames(longest, fps) : 0;
    if (most < 1) return Math.min(length, longest);
    return Math.max(1, Math.min(Math.round(length * fps), most)) / fps;
  };
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
  const total = mainDuration(timeline);
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

// ---- Tracks: overlays and audio --------------------------------------------
//
// Overlays sit on tracks above the main one and audio items on tracks below
// it. Both are anchored by where they start and kept apart on their tracks the
// same way, by the functions here taking a `Lane`.

/** What sits on a track above or below the main one. */
export type TrackItem = OverlayItem | AudioItem;

/** The tracks of overlays, above the main one, or those of audio, below it. */
type Lane = "tracks" | "audio_tracks";

interface Track<T extends TrackItem> {
  id: string;
  /** Of an overlay track; audio tracks have none. */
  kind?: TrackKind;
  items: T[];
}

export interface Placed<T extends TrackItem> {
  item: T;
  /** Index of its track: of overlays the bottom one first, of audio the top one. */
  track: number;
  /** The visible part, in project time. */
  start: number;
  end: number;
  /**
   * How much of the item is hidden before `start`, cut off with the head of
   * its clip; a video overlay or an audio item skips as much of its source.
   */
  skip: number;
}

export type PlacedOverlay = Placed<OverlayItem>;
export type PlacedAudio = Placed<AudioItem>;

/** Where an item goes: onto an existing track, or onto a new one inserted at that index. */
export type OverlayTarget = { track: number } | { insert: number };

type Content<T> = T extends TrackItem ? Omit<T, "id" | "clip_id" | "start" | "duration"> : never;
/** An overlay without its place on the timeline. */
export type OverlayContent = Content<OverlayItem>;
/** An audio item without its place on the timeline. */
export type AudioContent = Content<AudioItem>;

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function overlaps(a: { start: number; end: number }, b: { start: number; end: number }) {
  return a.start < b.end - EPSILON && b.start < a.end - EPSILON;
}

function tracksOf(timeline: ProjectTimeline, lane: Lane): Track<TrackItem>[] {
  return (timeline[lane] ?? []) as Track<TrackItem>[];
}

function withTracks(timeline: ProjectTimeline, lane: Lane, tracks: Track<TrackItem>[]): ProjectTimeline {
  return { ...timeline, [lane]: tracks };
}

/** Whether the item plays its source from a time of its own: a video overlay or an audio item. */
function sourced(item: TrackItem): item is VideoOverlay | AudioItem {
  return "source_start" in item;
}

function overlaySpan(item: TrackItem, clip: PlacedClip | undefined) {
  // Anchored past the out point, it was trimmed away with the end of its clip.
  if (!clip || item.start >= clip.clip.end) return null;
  const from = toProjectTime(clip, item.start);
  const start = Math.max(from, clip.offset);
  const end = from + item.duration;
  if (end - start <= EPSILON) return null;
  return { start, end, skip: start - from };
}

function placeItems(timeline: ProjectTimeline, lane: Lane): Placed<TrackItem>[] {
  const clips = new Map(placeClips(timeline).map((placed) => [placed.clip.id, placed]));
  const result: Placed<TrackItem>[] = [];
  tracksOf(timeline, lane).forEach((track, index) => {
    for (const item of track.items) {
      const span = overlaySpan(item, clips.get(item.clip_id));
      if (span) result.push({ item, track: index, ...span });
    }
  });
  return result;
}

/** Visible overlays, the bottom track first and each track's items in order. */
export function placeOverlays(timeline: ProjectTimeline): PlacedOverlay[] {
  return placeItems(timeline, "tracks") as PlacedOverlay[];
}

/** Audio items heard, the top track first and each track's items in order. */
export function placeAudio(timeline: ProjectTimeline): PlacedAudio[] {
  return placeItems(timeline, "audio_tracks") as PlacedAudio[];
}

export function overlayCount(timeline: ProjectTimeline): number {
  return timeline.tracks.reduce((total, track) => total + track.items.length, 0);
}

function findItem(timeline: ProjectTimeline, lane: Lane, id: string | null): TrackItem | null {
  if (!id) return null;
  for (const track of tracksOf(timeline, lane)) {
    const item = track.items.find((entry) => entry.id === id);
    if (item) return item;
  }
  return null;
}

export function findOverlay(timeline: ProjectTimeline, id: string | null): OverlayItem | null {
  return findItem(timeline, "tracks", id) as OverlayItem | null;
}

export function findAudio(timeline: ProjectTimeline, id: string | null): AudioItem | null {
  return findItem(timeline, "audio_tracks", id) as AudioItem | null;
}

/** The kind of track `item` goes on in `lane`; audio items have none. */
function kindIn(lane: Lane, item: TrackItem): TrackKind | undefined {
  return lane === "tracks" ? trackKindOf(item as OverlayItem) : undefined;
}

/** Whether `track` holds items going on tracks of `kind`; any audio track holds any audio item. */
function holds(track: Track<TrackItem> | undefined, kind: TrackKind | undefined): boolean {
  if (!track || !kind) return true;
  return track.kind ? track.kind === kind : track.items.every((item) => trackKindOf(item as OverlayItem) === kind);
}

/** Whether `start..end` is free on `track`, apart from item `ignore`. */
export function trackIsFree(
  placed: Placed<TrackItem>[],
  track: number,
  start: number,
  end: number,
  ignore: string | null = null,
): boolean {
  return !placed.some((other) => other.track === track && other.item.id !== ignore && overlaps(other, { start, end }));
}

/**
 * `target`, or a new track just after it when it is of another kind than
 * `kind` or taken over `start..end`.
 */
function freeTarget(
  tracks: Track<TrackItem>[],
  placed: Placed<TrackItem>[],
  kind: TrackKind | undefined,
  target: OverlayTarget,
  start: number,
  end: number,
  ignore: string | null,
): OverlayTarget {
  if (!("track" in target)) return target;
  const free = holds(tracks[target.track], kind) && trackIsFree(placed, target.track, start, end, ignore);
  return free ? target : { insert: target.track + 1 };
}

/**
 * Where an overlay of `kind` over `start..end` goes when aimed at `target`:
 * there, or a new track just above a track of the other kind or one taken
 * there, apart from item `ignore`.
 */
export function overlayTarget(
  timeline: ProjectTimeline,
  kind: TrackKind,
  target: OverlayTarget,
  start: number,
  end: number,
  ignore: string | null = null,
): OverlayTarget {
  return freeTarget(timeline.tracks, placeOverlays(timeline), kind, target, start, end, ignore);
}

/**
 * The first track of `kind` free over `start..end`, else a new one just after
 * the last of that kind. With none, a new video track goes just over the main
 * one and any other new track after the last.
 */
function lowestFree(
  tracks: Track<TrackItem>[],
  placed: Placed<TrackItem>[],
  kind: TrackKind | undefined,
  start: number,
  end: number,
): OverlayTarget {
  let last = -1;
  for (let track = 0; track < tracks.length; track++) {
    if (!holds(tracks[track], kind)) continue;
    if (trackIsFree(placed, track, start, end)) return { track };
    last = track;
  }
  if (last >= 0) return { insert: last + 1 };
  return { insert: kind === "video" ? 0 : tracks.length };
}

/** The latest project time an item can start at: it starts over the main track. */
function latestStart(timeline: ProjectTimeline): number {
  return mainDuration(timeline) - MIN_OVERLAY_DURATION;
}

/** The anchor of an item starting at project time `time`. */
function anchorAt(timeline: ProjectTimeline, time: number): { clip_id: string; start: number } | null {
  const placed = clipAt(timeline, time);
  if (!placed) return null;
  return { clip_id: placed.clip.id, start: toSourceTime(placed, Math.max(time, placed.offset)) };
}

/**
 * The item of `placed` anchored to start at project time `start` and run for
 * `length`; one playing a source then plays it from `shift` past where it did.
 */
function reanchor(timeline: ProjectTimeline, placed: Placed<TrackItem>, start: number, length: number, shift = 0) {
  const anchored = anchorAt(timeline, start);
  if (!anchored) return null;
  const item = { ...placed.item, ...anchored, duration: length } as TrackItem;
  if (sourced(item) && sourced(placed.item)) item.source_start = placed.item.source_start + placed.skip + shift;
  return item;
}

/** `tracks` with `item` on `target`; a new track is of `kind`. */
function putItem<R extends Track<TrackItem>>(
  tracks: R[],
  item: R["items"][number],
  target: OverlayTarget,
  kind: TrackKind | undefined,
): R[] {
  if ("track" in target && tracks[target.track]) {
    return tracks.map((track, index) => (index === target.track ? { ...track, items: [...track.items, item] } : track));
  }
  const next = [...tracks];
  const track = { id: newId(), ...(kind ? { kind } : {}), items: [item] } as R;
  next.splice(clamp("insert" in target ? target.insert : tracks.length, 0, tracks.length), 0, track);
  return next;
}

/** `tracks` without the items `drop` picks, and without tracks left empty. */
function withoutItems<R extends Track<TrackItem>>(tracks: R[], drop: (item: R["items"][number]) => boolean): R[] {
  return tracks
    .map((track) => (track.items.some(drop) ? { ...track, items: track.items.filter((item) => !drop(item)) } : track))
    .filter((track) => track.items.length > 0);
}

function replaceItem(timeline: ProjectTimeline, lane: Lane, item: TrackItem): ProjectTimeline {
  return withTracks(
    timeline,
    lane,
    tracksOf(timeline, lane).map((track) =>
      track.items.some((entry) => entry.id === item.id)
        ? { ...track, items: track.items.map((entry) => (entry.id === item.id ? item : entry)) }
        : track,
    ),
  );
}

function addItem(
  timeline: ProjectTimeline,
  lane: Lane,
  content: Content<TrackItem>,
  start: number,
  duration: number,
  target?: OverlayTarget,
): { timeline: ProjectTimeline; id: string | null } {
  const from = clamp(start, 0, latestStart(timeline));
  const length = duration;
  const anchored = from >= 0 && length >= MIN_OVERLAY_DURATION ? anchorAt(timeline, from) : null;
  if (!anchored) return { timeline, id: null };
  const id = newId();
  const item = { ...content, id, ...anchored, duration: length } as TrackItem;
  const placed = placeItems(timeline, lane);
  const tracks = tracksOf(timeline, lane);
  const kind = kindIn(lane, item);
  const to = target
    ? freeTarget(tracks, placed, kind, target, from, from + length, null)
    : lowestFree(tracks, placed, kind, from, from + length);
  return { timeline: withTracks(timeline, lane, putItem(tracks, item, to, kind)), id };
}

/**
 * Add an item over project time `start..start + duration`, starting over the
 * main track and maybe running on past it, on `target` or else the lowest
 * track of its kind free there.
 */
export function addOverlay(
  timeline: ProjectTimeline,
  content: OverlayContent,
  start: number,
  duration: number,
  target?: OverlayTarget,
): { timeline: ProjectTimeline; id: string | null } {
  return addItem(timeline, "tracks", content, start, duration, target);
}

/** Add an audio item, as `addOverlay` adds an overlay, onto the top track free there. */
export function addAudio(
  timeline: ProjectTimeline,
  content: AudioContent,
  start: number,
  duration: number,
  target?: OverlayTarget,
): { timeline: ProjectTimeline; id: string | null } {
  return addItem(timeline, "audio_tracks", content, start, duration, target);
}

function editItem(
  timeline: ProjectTimeline,
  lane: Lane,
  id: string,
  update: (item: TrackItem) => TrackItem,
): ProjectTimeline {
  const item = findItem(timeline, lane, id);
  return item ? replaceItem(timeline, lane, update(item)) : timeline;
}

/** Change an item, such as its element, in place. */
export function editOverlay(
  timeline: ProjectTimeline,
  id: string,
  update: (item: OverlayItem) => OverlayItem,
): ProjectTimeline {
  return editItem(timeline, "tracks", id, update as (item: TrackItem) => TrackItem);
}

/** Change an audio item, such as its volume, in place. */
export function editAudio(
  timeline: ProjectTimeline,
  id: string,
  update: (item: AudioItem) => AudioItem,
): ProjectTimeline {
  return editItem(timeline, "audio_tracks", id, update as (item: TrackItem) => TrackItem);
}

export function removeOverlay(timeline: ProjectTimeline, id: string): ProjectTimeline {
  return { ...timeline, tracks: withoutItems(timeline.tracks, (item) => item.id === id) };
}

export function removeAudio(timeline: ProjectTimeline, id: string): ProjectTimeline {
  return { ...timeline, audio_tracks: withoutItems(timeline.audio_tracks ?? [], (item) => item.id === id) };
}

function moveItem(timeline: ProjectTimeline, lane: Lane, id: string, start: number, target: OverlayTarget): ProjectTimeline {
  const placed = placeItems(timeline, lane);
  const current = placed.find((entry) => entry.item.id === id);
  if (!current) return timeline;
  const length = current.end - current.start;
  const from = clamp(start, 0, latestStart(timeline));
  const item = reanchor(timeline, current, from, length);
  if (!item) return timeline;
  const kind = kindIn(lane, item);
  const to = freeTarget(tracksOf(timeline, lane), placed, kind, target, from, from + length, id);
  const tracks = putItem(
    tracksOf(timeline, lane).map((track) => ({ ...track, items: track.items.filter((entry) => entry.id !== id) })),
    item,
    to,
    kind,
  );
  return withTracks(
    timeline,
    lane,
    tracks.filter((track) => track.items.length > 0),
  );
}

/**
 * Move the visible part of an item to start at project time `start` on
 * `target`, re-anchoring it to the clip there. A track of the other kind, or
 * one taken at the new place, gets a new track above it instead.
 */
export function moveOverlay(
  timeline: ProjectTimeline,
  id: string,
  start: number,
  target: OverlayTarget,
): ProjectTimeline {
  return moveItem(timeline, "tracks", id, start, target);
}

/** Move the part heard of an audio item, as `moveOverlay` moves an overlay; a taken track gets a new one below it. */
export function moveAudio(timeline: ProjectTimeline, id: string, start: number, target: OverlayTarget): ProjectTimeline {
  return moveItem(timeline, "audio_tracks", id, start, target);
}

function trimItem(
  timeline: ProjectTimeline,
  lane: Lane,
  id: string,
  edge: "start" | "end",
  time: number,
  sourceDuration: number,
): ProjectTimeline {
  const placed = placeItems(timeline, lane);
  const current = placed.find((entry) => entry.item.id === id);
  if (!current) return timeline;
  const neighbours = placed.filter((entry) => entry.track === current.track && entry.item.id !== id);
  const playing = sourced(current.item) ? current.item : null;
  // Source time played at the visible start.
  const source = playing ? playing.source_start + current.skip : 0;
  let item: TrackItem | null;
  if (edge === "start") {
    const before = Math.max(0, ...neighbours.filter((entry) => entry.end <= current.start + EPSILON).map((entry) => entry.end));
    const earliest = playing ? current.start - source : 0;
    const start = clamp(time, Math.max(before, earliest), Math.min(current.end, mainDuration(timeline)) - MIN_OVERLAY_DURATION);
    item = reanchor(timeline, current, start, current.end - start, start - current.start);
  } else {
    const after = Math.min(...neighbours.filter((entry) => entry.start >= current.end - EPSILON).map((entry) => entry.start));
    const latest = playing && sourceDuration > 0 ? current.start + sourceDuration - source : Number.POSITIVE_INFINITY;
    const end = clamp(time, current.start + MIN_OVERLAY_DURATION, Math.min(after, latest));
    item = reanchor(timeline, current, current.start, end - current.start);
  }
  return item ? replaceItem(timeline, lane, item) : timeline;
}

/**
 * Move one edge of an item to project time `time`, short of its neighbours on
 * the track; its start stays over the main track. A video overlay's edges
 * stay within its material, `materialDuration` long.
 */
export function trimOverlay(
  timeline: ProjectTimeline,
  id: string,
  edge: "start" | "end",
  time: number,
  materialDuration = 0,
): ProjectTimeline {
  return trimItem(timeline, "tracks", id, edge, time, materialDuration);
}

/** Move one edge of an audio item, as `trimOverlay` does, within its source, `sourceDuration` long. */
export function trimAudio(
  timeline: ProjectTimeline,
  id: string,
  edge: "start" | "end",
  time: number,
  sourceDuration = 0,
): ProjectTimeline {
  return trimItem(timeline, "audio_tracks", id, edge, time, sourceDuration);
}

function splitItem(
  timeline: ProjectTimeline,
  lane: Lane,
  id: string,
  time: number,
): { timeline: ProjectTimeline; left: TrackItem | null; right: TrackItem | null } {
  const none = { timeline, left: null, right: null };
  const current = placeItems(timeline, lane).find((entry) => entry.item.id === id);
  if (!current || time - current.start < MIN_OVERLAY_DURATION || current.end - time < MIN_OVERLAY_DURATION) return none;
  // The part after the cut starts over the main track too.
  if (time > latestStart(timeline)) return none;
  const anchored = anchorAt(timeline, time);
  if (!anchored) return none;
  // Where the whole item starts, including any part hidden with its clip's head.
  const from = current.start - current.skip;
  const left = { ...current.item, duration: time - from } as TrackItem;
  const right = { ...current.item, id: newId(), ...anchored, duration: from + current.item.duration - time } as TrackItem;
  if ("element" in right) right.element = { ...right.element } as typeof right.element;
  if (sourced(right) && sourced(current.item)) right.source_start = current.item.source_start + time - from;
  const tracks = tracksOf(timeline, lane).map((track) => {
    const index = track.items.findIndex((entry) => entry.id === id);
    if (index < 0) return track;
    const items = [...track.items];
    items.splice(index, 1, left, right);
    return { ...track, items };
  });
  return { timeline: withTracks(timeline, lane, tracks), left, right };
}

/** Split an item at project time `time`; the part after the cut becomes a new item beside it. */
export function splitOverlay(
  timeline: ProjectTimeline,
  id: string,
  time: number,
): { timeline: ProjectTimeline; id: string | null } {
  const split = splitItem(timeline, "tracks", id, time);
  return { timeline: split.timeline, id: split.right?.id ?? null };
}

/**
 * Split an audio item at project time `time`, as `splitOverlay` does: the
 * part before the cut keeps its fade in, the part after it its fade out.
 */
export function splitAudio(
  timeline: ProjectTimeline,
  id: string,
  time: number,
): { timeline: ProjectTimeline; id: string | null } {
  const split = splitItem(timeline, "audio_tracks", id, time);
  if (!split.left || !split.right) return { timeline, id: null };
  const left = { ...(split.left as AudioItem) };
  const right = { ...(split.right as AudioItem) };
  delete left.fade_out;
  delete right.fade_in;
  const next = replaceItem(replaceItem(split.timeline, "audio_tracks", left), "audio_tracks", right);
  return { timeline: next, id: right.id };
}

/** Where moving an item one layer up or down puts it, or null at the top or bottom. */
function shiftTarget(timeline: ProjectTimeline, placed: PlacedOverlay[], id: string, step: 1 | -1) {
  const current = placed.find((entry) => entry.item.id === id);
  if (!current) return null;
  const tracks = timeline.tracks;
  const neighbour = current.track + step;
  if (neighbour < 0 || neighbour >= tracks.length) {
    if (tracks[current.track].items.length === 1) return null;
    return { current, target: { insert: step > 0 ? tracks.length : 0 } };
  }
  // Past a track of the other kind, onto a new one beyond it.
  if (!holds(tracks[neighbour], trackKindOf(current.item))) {
    return { current, target: { insert: step > 0 ? neighbour + 1 : neighbour } };
  }
  if (trackIsFree(placed, neighbour, current.start, current.end, id)) return { current, target: { track: neighbour } };
  // Between its track and the neighbouring one.
  return { current, target: { insert: step > 0 ? neighbour : current.track } };
}

export function canShiftOverlay(timeline: ProjectTimeline, id: string, step: 1 | -1): boolean {
  return shiftTarget(timeline, placeOverlays(timeline), id, step) !== null;
}

/**
 * Move an item onto the track above or below, or a new one between when that
 * is taken, or past it when it is of the other kind.
 */
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
  const tracks = putItem(timeline.tracks, copy, { insert: current.track + 1 }, trackKindOf(copy));
  return { timeline: { ...timeline, tracks }, id: copy.id };
}

// ---- Between the main track and the video tracks --------------------------

/**
 * Whether clip `clipId` can leave the main track: what stays there must be
 * long enough to show an overlay over.
 */
export function canLiftClip(timeline: ProjectTimeline, clipId: string): boolean {
  const clip = timeline.clips.find((item) => item.id === clipId);
  return clip !== undefined && mainDuration(timeline) - clipDuration(clip) >= MIN_OVERLAY_DURATION;
}

/**
 * Move clip `clipId` off the main track onto `target` as a video overlay
 * starting at project time `start`, placed on the picture at `element` and as
 * loud as the clip was. The clips after it close up. What was anchored to it,
 * subtitles, overlays and audio, moves along with it, re-anchored to the
 * clips below where it lands; parts that were hidden, or that would start
 * past the end of the main track, go. Null when it is the only clip.
 */
export function clipToOverlay(
  timeline: ProjectTimeline,
  clipId: string,
  start: number,
  target: OverlayTarget,
  element: VideoPlacement,
): { timeline: ProjectTimeline; id: string } | null {
  const moved = placeClips(timeline).find((entry) => entry.clip.id === clipId);
  if (!moved || !canLiftClip(timeline, clipId)) return null;
  const rest: ProjectTimeline = { ...timeline, clips: timeline.clips.filter((clip) => clip.id !== clipId) };
  const total = mainDuration(rest);
  if (total < MIN_OVERLAY_DURATION) return null;
  const from = clamp(start, 0, total - MIN_OVERLAY_DURATION);
  const shift = from - moved.offset;

  const subtitles: TimelineSubtitle[] = timeline.subtitles.filter((subtitle) => subtitle.clip_id !== clipId);
  for (const entry of visibleSubtitles(timeline)) {
    if (entry.clip.clip.id !== clipId || entry.start + shift >= total) continue;
    const anchored = anchor(rest, entry.start + shift, entry.end + shift);
    if (anchored) subtitles.push({ ...entry.subtitle, ...anchored });
  }
  const carried = (lane: Lane) => {
    const items = new Map<string, TrackItem | null>();
    for (const entry of placeItems(timeline, lane)) {
      if (entry.item.clip_id !== clipId) continue;
      const at = entry.start + shift;
      items.set(entry.item.id, at < total ? reanchor(rest, entry, at, entry.item.duration - entry.skip) : null);
    }
    // Those hidden, never placed, go too.
    return withoutItems(
      tracksOf(timeline, lane).map((track) => ({ ...track, items: track.items.map((item) => items.get(item.id) ?? item) })),
      (item) => item.clip_id === clipId,
    );
  };
  let next: ProjectTimeline = {
    ...rest,
    subtitles,
    tracks: carried("tracks") as OverlayTrack[],
    audio_tracks: carried("audio_tracks") as AudioTrack[],
  };

  const anchored = anchorAt(next, from);
  if (!anchored) return null;
  const video: VideoOverlay = {
    id: newId(),
    kind: "video",
    ...anchored,
    duration: moved.duration,
    material_id: moved.clip.material_id,
    source_start: moved.clip.start,
    volume: moved.clip.volume ?? 1,
    element,
  };
  const to = overlayTarget(next, "video", target, from, from + moved.duration);
  next = { ...next, tracks: putItem(next.tracks, video, to, "video") };
  return { timeline: next, id: video.id };
}

/**
 * Move video overlay `id` onto the main track at `index`, as a clip of what
 * it shows from where it is first seen and as loud; its place on the picture
 * and opacity go. With a frame rate `fps`, the clip lasts whole frames.
 */
export function overlayToClip(
  timeline: ProjectTimeline,
  id: string,
  index: number,
  fps: number,
): { timeline: ProjectTimeline; clipId: string | null } {
  const entry = placeOverlays(timeline).find((placed) => placed.item.id === id);
  if (entry?.item.kind !== "video") return { timeline, clipId: null };
  const item = entry.item;
  const start = item.source_start + entry.skip;
  const clip = {
    material_id: item.material_id,
    start,
    end: start + item.duration - entry.skip,
    ...(item.volume !== 1 ? { volume: item.volume } : {}),
  };
  const without = removeOverlay(timeline, id);
  const next = insertClips(without, [clip], index, fps);
  if (next.clips.length === without.clips.length) return { timeline, clipId: null };
  return { timeline: next, clipId: next.clips[clamp(index, 0, without.clips.length)].id };
}

function settleItems(timeline: ProjectTimeline, lane: Lane): ProjectTimeline {
  const spans = new Map(placeItems(timeline, lane).map((entry) => [entry.item.id, entry]));
  const startOf = (item: TrackItem) => spans.get(item.id)?.start ?? Number.NEGATIVE_INFINITY;
  let changed = false;
  const tracks: Track<TrackItem>[] = [];
  for (const track of tracksOf(timeline, lane)) {
    if (track.items.length === 0) changed = true;
    let current = track;
    while (current.items.length > 0) {
      const kept: Placed<TrackItem>[] = [];
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
      current = {
        id: newId(),
        ...(current.kind ? { kind: current.kind } : {}),
        items: current.items.filter((item) => lifted.has(item.id)),
      };
    }
  }
  return changed ? withTracks(timeline, lane, tracks) : timeline;
}

/**
 * Lift items that overlap others on their track, as moving or trimming clips
 * can leave them, onto new tracks just above, and drop empty tracks. Returns
 * `timeline` itself when nothing changes.
 */
export function settleOverlays(timeline: ProjectTimeline): ProjectTimeline {
  return settleItems(timeline, "tracks");
}

/** Settle the overlays, and the audio items as well onto new tracks just below. */
export function settleTracks(timeline: ProjectTimeline): ProjectTimeline {
  return settleItems(settleOverlays(timeline), "audio_tracks");
}

export type TimelineSelection = {
  clips: string[];
  subtitles: string[];
  overlays: string[];
  audio: string[];
};

/**
 * Commit a group drag against the current timeline, not a drag-start snapshot.
 * `mainIndex` counts only unselected clips. Main clips move together in their
 * original order; their anchors follow naturally. Other selected, visible items
 * move by one common delta (the first selected clip's actual offset on reorder).
 * A secondary-only drag clamps that delta collectively; a reorder that would
 * require clamping is rejected instead of distorting the group.
 *
 * Like moveSubtitle, subtitles remain bound to their destination clip: a cue
 * crossing an internal cut is clipped there. Its full visible duration is kept
 * within the main end. Track media retain their visible source trims and may
 * extend past the main end. Collisions are settled only after every move.
 */
export function moveTimelineSelection(
  timeline: ProjectTimeline,
  selection: TimelineSelection,
  delta: number,
  mainIndex?: number,
): ProjectTimeline {
  if (!Number.isFinite(delta) || delta === 0) return timeline;
  const clipIds = new Set(selection.clips);
  const selected = placeClips(timeline).filter(({ clip }) => clipIds.has(clip.id));
  let next = timeline;
  let shift = delta;
  if (selected.length > 0) {
    if (mainIndex === undefined || !Number.isFinite(mainIndex)) return timeline;
    const clips = timeline.clips.filter((clip) => !clipIds.has(clip.id));
    clips.splice(clamp(Math.trunc(mainIndex), 0, clips.length), 0, ...selected.map(({ clip }) => clip));
    if (clips.every((clip, index) => clip === timeline.clips[index])) return timeline;
    next = { ...timeline, clips };
    shift = placeClips(next).find(({ clip }) => clip.id === selected[0].clip.id)!.offset - selected[0].offset;
  }

  // Read all spans before changing any anchors. In particular, do not let a
  // selected neighbour's old position cause a spurious track collision.
  const carried = new Set(selected.map(({ clip }) => clip.id));
  const subtitleIds = new Set(selection.subtitles);
  const subtitles = visibleSubtitles(timeline).filter(({ subtitle }) =>
    subtitleIds.has(subtitle.id) && !carried.has(subtitle.clip_id));
  const entries = (["tracks", "audio_tracks"] as const).map((lane) => {
    const ids = new Set(lane === "tracks" ? selection.overlays : selection.audio);
    return { lane, placed: placeItems(timeline, lane).filter(({ item }) => ids.has(item.id) && !carried.has(item.clip_id)) };
  });
  if (selected.length === 0 && subtitles.length === 0 && entries.every(({ placed }) => placed.length === 0)) return timeline;

  const total = mainDuration(next);
  let minShift = Number.NEGATIVE_INFINITY;
  let maxShift = Number.POSITIVE_INFINITY;
  for (const { start, end } of subtitles) {
    minShift = Math.max(minShift, -start);
    maxShift = Math.min(maxShift, total - Math.max(MIN_SUBTITLE_DURATION, end - start) - start);
  }
  for (const { placed } of entries) {
    for (const { start } of placed) {
      minShift = Math.max(minShift, -start);
      maxShift = Math.min(maxShift, total - MIN_OVERLAY_DURATION - start);
    }
  }
  if (minShift > maxShift) return timeline;
  if (selected.length > 0) {
    if (shift < minShift - EPSILON || shift > maxShift + EPSILON) return timeline;
  } else {
    shift = clamp(shift, minShift, maxShift);
    if (shift === 0) return timeline;
  }

  if (shift !== 0 || next !== timeline) {
    const movedSubtitles = new Map(subtitles.map(({ subtitle, start, end }) => {
      const anchored = anchorAt(next, start + shift)!;
      return [subtitle.id, { ...subtitle, ...anchored, end: anchored.start + end - start }] as const;
    }));
    if (movedSubtitles.size > 0) {
      next = { ...next, subtitles: next.subtitles.map((subtitle) => movedSubtitles.get(subtitle.id) ?? subtitle) };
    }
    for (const { lane, placed } of entries) {
      if (placed.length === 0) continue;
      const moved = new Map(placed.map((entry) => [
        entry.item.id, reanchor(next, entry, entry.start + shift, entry.end - entry.start)!,
      ]));
      next = withTracks(next, lane, tracksOf(next, lane).map((track) =>
        track.items.some((item) => moved.has(item.id))
          ? { ...track, items: track.items.map((item) => moved.get(item.id) ?? item) }
          : track));
    }
  }
  return settleTracks(next);
}

/**
 * `current` with overlay `id` as it is in `base`: how the timeline was before
 * a drag of that overlay, keeping changes merged in while dragging.
 */
export function withOverlayFrom(current: ProjectTimeline, base: ProjectTimeline, id: string): ProjectTimeline {
  const original = findOverlay(base, id);
  return original ? editOverlay(current, id, () => original) : current;
}

/** `current` with audio item `id` as it is in `base`, as `withOverlayFrom` has an overlay. */
export function withAudioFrom(current: ProjectTimeline, base: ProjectTimeline, id: string): ProjectTimeline {
  const original = findAudio(base, id);
  return original ? editAudio(current, id, () => original) : current;
}

/**
 * How loud an audio item's fades leave it at project time `time`, 0 to 1, as
 * export fades it: over the part heard, and together no longer than it.
 */
export function fadeGain(placed: PlacedAudio, time: number): number {
  const length = placed.end - placed.start;
  let fadeIn = Math.max(0, placed.item.fade_in ?? 0);
  let fadeOut = Math.max(0, placed.item.fade_out ?? 0);
  if (fadeIn + fadeOut > length) {
    const scale = length / (fadeIn + fadeOut);
    fadeIn *= scale;
    fadeOut *= scale;
  }
  let gain = 1;
  if (fadeIn > 0) gain = Math.min(gain, (time - placed.start) / fadeIn);
  if (fadeOut > 0) gain = Math.min(gain, (placed.end - time) / fadeOut);
  return clamp(gain, 0, 1);
}

/**
 * The audio tracks with the items anchored to the clips of `removed`, which
 * are about to go, moved onto the first clip after them that stays: where
 * those clips were, playing on from where each was first heard. Those with no
 * clip after them, or not heard at all, go. The backend does the same.
 */
function reanchorAudio(timeline: ProjectTimeline, removed: Set<string>): AudioTrack[] {
  const placed = placeClips(timeline);
  const index = new Map(placed.map((entry, at) => [entry.clip.id, at]));
  const moved = (item: AudioItem): AudioItem | null => {
    const at = index.get(item.clip_id);
    if (at === undefined || !removed.has(item.clip_id)) return item;
    const clip = placed[at];
    const skip = Math.max(clip.offset - toProjectTime(clip, item.start), 0);
    const heard = item.start < clip.clip.end && item.duration - skip > EPSILON;
    const next = placed.slice(at + 1).find((entry) => !removed.has(entry.clip.id));
    if (!heard || !next) return null;
    return {
      ...item,
      clip_id: next.clip.id,
      start: next.clip.start,
      source_start: item.source_start + skip,
      duration: item.duration - skip,
    };
  };
  return (timeline.audio_tracks ?? [])
    .map((track) =>
      track.items.some((item) => removed.has(item.clip_id))
        ? { ...track, items: track.items.map(moved).filter((item): item is AudioItem => item !== null) }
        : track,
    )
    .filter((track) => track.items.length > 0);
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
