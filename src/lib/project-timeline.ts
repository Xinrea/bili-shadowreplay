import type {
  ProjectTimeline,
  SubtitleStyle,
  TimelineClip,
  TimelineSubtitle,
} from "./interface";

// Project (切片) timelines, mirroring src-tauri/src/project.rs. Clips play back
// to back on one main track; subtitles are anchored to a clip in material time
// so they follow the picture when clips move or are trimmed. Every edit returns
// a new timeline so the editor can keep an undo history of snapshots.

export const TIMELINE_VERSION = 1;
export const MIN_CLIP_DURATION = 0.1;
export const MIN_SUBTITLE_DURATION = 0.1;

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
  return { version: TIMELINE_VERSION, clips: [], subtitles: [], subtitle_style: style, danmu: false };
}

export function parseTimeline(json: string): ProjectTimeline {
  try {
    const value = json ? JSON.parse(json) : {};
    return {
      version: value.version ?? TIMELINE_VERSION,
      clips: Array.isArray(value.clips) ? value.clips : [],
      subtitles: Array.isArray(value.subtitles) ? value.subtitles : [],
      subtitle_style: value.subtitle_style ?? null,
      danmu: Boolean(value.danmu),
    };
  } catch {
    return emptyTimeline();
  }
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

/** Cues in project time for `buildSubtitleAss`. */
export function projectCues(timeline: ProjectTimeline) {
  return visibleSubtitles(timeline).map(({ start, end, subtitle }, index) => ({
    startTime: start,
    endTime: end,
    text: subtitle.text,
    layerOrder: index,
  }));
}

function replaceClip(timeline: ProjectTimeline, clip: TimelineClip): ProjectTimeline {
  return { ...timeline, clips: timeline.clips.map((item) => (item.id === clip.id ? clip : item)) };
}

export function insertClips(
  timeline: ProjectTimeline,
  clips: Omit<TimelineClip, "id">[],
  index = timeline.clips.length,
): ProjectTimeline {
  const added = clips
    .filter((clip) => clip.end - clip.start >= MIN_CLIP_DURATION)
    .map((clip) => ({ ...clip, id: newId() }));
  const next = [...timeline.clips];
  next.splice(Math.max(0, Math.min(index, next.length)), 0, ...added);
  return { ...timeline, clips: next };
}

/** Remove a clip together with the subtitles anchored to it. */
export function removeClip(timeline: ProjectTimeline, clipId: string): ProjectTimeline {
  return {
    ...timeline,
    clips: timeline.clips.filter((clip) => clip.id !== clipId),
    subtitles: timeline.subtitles.filter((subtitle) => subtitle.clip_id !== clipId),
  };
}

/**
 * Bring the backend's own edits of a saved timeline into one being edited.
 * Between `base`, the saved timeline the editor last knew, and `remote`, the
 * saved one now, the backend may switch danmaku, remove the clips of a deleted
 * material and fill clips that had none with recognized subtitles; everything
 * else stays as edited in `local`. Returns `local` itself when nothing applies.
 */
export function mergeRemoteTimeline(
  local: ProjectTimeline,
  base: ProjectTimeline,
  remote: ProjectTimeline,
): ProjectTimeline {
  let next = local;
  if (remote.danmu !== local.danmu) next = { ...next, danmu: remote.danmu };

  const remaining = new Set(remote.clips.map((clip) => clip.material_id));
  const removed = new Set(base.clips.map((clip) => clip.material_id).filter((id) => !remaining.has(id)));
  if (next.clips.some((clip) => removed.has(clip.material_id))) {
    const clips = next.clips.filter((clip) => !removed.has(clip.material_id));
    const kept = new Set(clips.map((clip) => clip.id));
    next = { ...next, clips, subtitles: next.subtitles.filter((subtitle) => kept.has(subtitle.clip_id)) };
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
 * Split the clip under project time `time`. Subtitles after the cut move to the
 * new clip; one crossing the cut is kept on both sides so it still reads as one.
 */
export function splitClip(
  timeline: ProjectTimeline,
  time: number,
): { timeline: ProjectTimeline; clipId: string | null } {
  const placed = placeClips(timeline).find(
    (item) => time - item.offset >= MIN_CLIP_DURATION && item.offset + item.duration - time >= MIN_CLIP_DURATION,
  );
  if (!placed) return { timeline, clipId: null };
  const cut = toSourceTime(placed, time);
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
  return { timeline: { ...timeline, clips, subtitles }, clipId: right.id };
}

/** Move one edge of a clip to `sourceTime`, keeping it within the material. */
export function trimClip(
  timeline: ProjectTimeline,
  clipId: string,
  edge: "start" | "end",
  sourceTime: number,
  materialDuration: number,
): ProjectTimeline {
  const clip = timeline.clips.find((item) => item.id === clipId);
  if (!clip) return timeline;
  if (edge === "start") {
    const start = Math.max(0, Math.min(sourceTime, clip.end - MIN_CLIP_DURATION));
    return replaceClip(timeline, { ...clip, start });
  }
  const limit = materialDuration > 0 ? materialDuration : Number.POSITIVE_INFINITY;
  const end = Math.min(limit, Math.max(sourceTime, clip.start + MIN_CLIP_DURATION));
  return replaceClip(timeline, { ...clip, end });
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
