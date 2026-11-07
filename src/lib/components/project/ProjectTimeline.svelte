<script lang="ts" module>
  export interface MaterialInfo {
    id: number;
    name: string;
    coverUrl: string;
    url: string;
    /** Relative to the output directory. */
    file: string;
    duration: number;
  }

  /** A file of the audio library, as the timeline shows it. */
  export interface AudioInfo {
    id: number;
    name: string;
    url: string;
    /** Relative to the output directory. */
    file: string;
    duration: number;
  }

  /** dataTransfer type used when dragging a material onto the timeline. */
  export const MATERIAL_DRAG_TYPE = "application/x-bsr-material";
  /** dataTransfer type used when dragging a file of the audio library onto the audio tracks. */
  export const AUDIO_DRAG_TYPE = "application/x-bsr-audio";

  /** What a sound dropped on the audio tracks plays: a file of the audio library, or a material's sound. */
  export type AudioDrop = { kind: "file"; id: number } | { kind: "material"; id: number };
</script>

<script lang="ts">
  import { AudioLines, Captions, Film, Music, Smile, Sticker, Type } from "lucide-svelte";
  import { onDestroy } from "svelte";
  import type { MaterialPeaks, ProjectTimeline, TrackKind } from "../../interface";
  import {
    audioFades,
    canLiftClip,
    moveTimelineSelection,
    type TimelineSelection,
    idsAt,
    moveAudio,
    moveClip,
    moveOverlay,
    moveSubtitle,
    overlayCount,
    overlayTarget,
    overlayToClip,
    placeAudio,
    placeOverlays,
    trimAudio,
    withAudioFrom,
    withOverlayFrom,
    withSubtitleFrom,
    mainDuration,
    placeClips,
    resizeSubtitle,
    timelineDuration,
    trackIsFree,
    trimClip,
    trimOverlay as trimOverlayEdge,
    trackKindOf,
    visibleSubtitles,
    MIN_OVERLAY_DURATION,
    type OverlayTarget,
    type PlacedAudio,
    type PlacedClip,
    type PlacedOverlay,
    type PlacedSubtitle,
  } from "../../project-timeline";
  import { formatTimecode } from "../../time-format";

  interface Props {
    timeline: ProjectTimeline;
    materials: Map<number, MaterialInfo>;
    peaks: Map<number, MaterialPeaks>;
    audioFiles: Map<number, AudioInfo>;
    /** Waveforms of the audio library's files. */
    audioPeaks: Map<number, MaterialPeaks>;
    /** Trims land on frames at this rate; 0 for none. */
    frameRate: number;
    time: number;
    /** 1 fits the whole project into view. */
    zoom: number;
    selectedClipId: string | null;
    selectedSubtitleId: string | null;
    selectedOverlayId: string | null;
    selectedAudioId: string | null;
    /** URL of each sticker reference, for the overlays' thumbnails. */
    stickerUrls: Map<string, string>;
    onSeek: (time: number) => void;
    onScrub?: (active: boolean) => void;
    /** Show an edit while it is being dragged. */
    onPreview: (timeline: ProjectTimeline) => void;
    /** Finish an edit that started from `before`. */
    onCommit: (timeline: ProjectTimeline, before: ProjectTimeline) => void;
    onSelectClip: (id: string | null) => void;
    onSelectSubtitle: (id: string | null) => void;
    onSelectOverlay: (id: string | null) => void;
    onSelectAudio: (id: string | null) => void;
    /** Add at project time: overlay existing picture, or append at the main track's end. */
    onDropMaterial: (materialId: number, time: number) => void;
    /** A material dropped on the tracks above the main one, to show over it from `time`. */
    onDropOverlay: (materialId: number, time: number, target: OverlayTarget) => void;
    /** A clip dragged off the main track onto a video track, to show there from `time`. */
    onLiftClip: (clipId: string, time: number, target: OverlayTarget) => void;
    /** A sound dropped on the audio tracks, to play from `time`. */
    onDropAudio: (source: AudioDrop, time: number, target: OverlayTarget) => void;
    onZoom: (zoom: number) => void;
  }

  let {
    timeline,
    materials,
    peaks,
    audioFiles,
    audioPeaks,
    frameRate,
    time,
    zoom,
    selectedClipId,
    selectedSubtitleId,
    selectedOverlayId,
    selectedAudioId,
    stickerUrls,
    onSeek,
    onScrub,
    onPreview,
    onCommit,
    onSelectClip,
    onSelectSubtitle,
    onSelectOverlay,
    onSelectAudio,
    onDropMaterial,
    onDropOverlay,
    onLiftClip,
    onDropAudio,
    onZoom,
  }: Props = $props();

  const RULER_HEIGHT = 24;
  const LANE_HEIGHT = 28;
  /** The main video track. */
  const MAIN_TRACK_HEIGHT = 76;
  /** The other video tracks. */
  const VIDEO_TRACK_HEIGHT = 48;
  const STICKER_TRACK_HEIGHT = 34;
  const AUDIO_TRACK_HEIGHT = 40;
  /** The row above the tracks over the main one, where a new track is made. */
  const NEW_TRACK_HEIGHT = 22;
  const EDGE_PX = 7;
  const SNAP_PX = 8;
  const DRAG_THRESHOLD_PX = 4;
  /**
   * Narrower than this, a block shows no text, which it has no room for:
   * laid out again at every zoom, thousands of truncated texts cost the most.
   */
  const LABEL_MIN_PX = 20;
  /** How long after a zoom waveforms are drawn again at their new width. */
  const WAVE_SETTLE_MS = 150;
  const TRAILING_PX = 160;
  const TICK_STEPS = [0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];

  let viewport: HTMLElement = $state();
  let viewportWidth = $state(800);
  let snapTime: number | null = $state(null);
  let materialDropTime: number | null = $state(null);
  let content: HTMLElement = $state();
  let overlayArea: HTMLElement = $state();
  let mainTrack: HTMLElement = $state();
  /** Where a material dragged over the tracks above the main one would go. */
  let overlayDrop: { time: number; target: OverlayTarget } | null = $state(null);
  let audioArea: HTMLElement = $state();
  /** Where a sound dragged over the audio tracks would go. */
  let audioDrop: { time: number; target: OverlayTarget } | null = $state(null);

  type ClipDrag = {
    kind: "move";
    clipId: string;
    pointerX: number;
    pointerY: number;
    pointerTime: number;
    /** Where the clip starts and how long it lasts. */
    offset: number;
    length: number;
    moved: boolean;
    index: number;
    /** Where it goes when dragged up off the main track, onto a video track. */
    lift: { ghost: number; target: OverlayTarget } | null;
  };
  type TrimDrag = {
    kind: "trim";
    clipId: string;
    edge: "start" | "end";
    base: ProjectTimeline;
    pointerX: number;
    /** Edge position in project px when the drag started. */
    edgeX: number;
    deltaPx: number;
  };
  type SubtitleDrag = {
    kind: "subtitle";
    id: string;
    mode: "move" | "start" | "end";
    base: ProjectTimeline;
    pointerTime: number;
    start: number;
    end: number;
    moved: boolean;
  };
  /** A drag of an overlay, or of an audio item, which are dragged the same way. */
  type OverlayDrag = {
    kind: "overlay";
    lane: "overlay" | "audio";
    /** Of the tracks an overlay goes on; null for audio. */
    trackKind: TrackKind | null;
    id: string;
    mode: "move" | "start" | "end";
    base: ProjectTimeline;
    pointerTime: number;
    pointerY: number;
    /** The visible part and track as the drag began. */
    start: number;
    end: number;
    track: number;
    moved: boolean;
    /** Where a move puts it, shown until it is dropped. */
    ghost: number;
    target: OverlayTarget;
    /** Index of the main track where a video overlay dragged onto it goes as a clip. */
    toMain: number | null;
  };
  let drag: ClipDrag | TrimDrag | SubtitleDrag | OverlayDrag | null = $state(null);
  /** An overlay or audio item being moved. */
  let moving = $derived(drag?.kind === "overlay" && drag.moved && drag.mode === "move" ? drag : null);
  /**
   * Where an overlay or audio item being moved, or a clip being lifted off
   * the main track, goes on the tracks of its lane, shown until it is dropped.
   */
  let ghost: { lane: OverlayDrag["lane"]; start: number; length: number; target: OverlayTarget } | null = $derived.by(
    () => {
      if (moving && moving.toMain === null) {
        return { lane: moving.lane, start: moving.ghost, length: moving.end - moving.start, target: moving.target };
      }
      if (drag?.kind === "move" && drag.lift) {
        return { lane: "overlay", start: drag.lift.ghost, length: drag.length, target: drag.lift.target };
      }
      return null;
    },
  );

  let duration = $derived(timelineDuration(timeline));
  // Overlays and sounds start over the main track, and may run on past it.
  let mainLength = $derived(mainDuration(timeline));
  let latestStart = $derived(mainLength - MIN_OVERLAY_DURATION);
  let pxPerSecond = $derived(
    Math.max(0.0005, ((viewportWidth - 48) / Math.max(duration, 10)) * zoom),
  );
  let contentWidth = $derived(Math.max(viewportWidth, duration * pxPerSecond + TRAILING_PX));
  let placed = $derived(placeClips(timeline));
  let subtitles = $derived(visibleSubtitles(timeline));
  let lanes = $derived(assignLanes(subtitles));
  let laneCount = $derived(Math.max(1, ...lanes.map((lane) => lane + 1)));
  let subtitleTrackHeight = $derived(laneCount * LANE_HEIGHT + 8);
  let overlays = $derived(placeOverlays(timeline));
  let audio = $derived(placeAudio(timeline));
  // What plays at the playhead, changing only as it crosses an edge: each key
  // is the same string until then, so nothing derived from it updates.
  let subtitlesKey = $derived(idsAt(subtitles, time, (item) => item.subtitle.id));
  let overlaysKey = $derived(idsAt(overlays, time, (item) => item.item.id));
  let audioKey = $derived(idsAt(audio, time, (item) => item.item.id));
  let playingSubtitles = $derived(new Set(subtitlesKey.split("\n")));
  let playingOverlays = $derived(new Set(overlaysKey.split("\n")));
  let playingAudio = $derived(new Set(audioKey.split("\n")));
  let trackCount = $derived(timeline.tracks.length);
  let hiddenOverlays = $derived(overlayCount(timeline) - overlays.length);
  // Audio tracks from the top, nearest the main track.
  let audioRows = $derived(Array.from({ length: timeline.audio_tracks.length }, (_, row) => row));
  let hiddenAudio = $derived(
    timeline.audio_tracks.reduce((total, track) => total + track.items.length, 0) - audio.length,
  );
  // Rows from the top: the highest track, drawn over the others, first.
  let trackRows = $derived(Array.from({ length: trackCount }, (_, row) => trackCount - 1 - row));
  let trackHeights = $derived(
    timeline.tracks.map((track) => (track.kind === "video" ? VIDEO_TRACK_HEIGHT : STICKER_TRACK_HEIGHT)),
  );
  let tickStep = $derived(TICK_STEPS.find((step) => step * pxPerSecond >= 84) ?? 3600);
  let ticks = $derived(
    Array.from({ length: Math.floor(contentWidth / pxPerSecond / tickStep) + 1 }, (_, i) => i * tickStep),
  );

  $effect(() => {
    if (!viewport) return;
    const observer = new ResizeObserver(() => (viewportWidth = viewport.clientWidth));
    observer.observe(viewport);
    viewportWidth = viewport.clientWidth;
    return () => observer.disconnect();
  });

  // Keep the playhead in view while it moves on its own.
  $effect(() => {
    const x = time * pxPerSecond;
    if (!viewport || drag || groupDrag || marquee) return;
    const { scrollLeft, clientWidth } = viewport;
    if (x < scrollLeft || x > scrollLeft + clientWidth - 40) {
      viewport.scrollLeft = Math.max(0, x - clientWidth * 0.2);
    }
  });

  function assignLanes(items: PlacedSubtitle[]): number[] {
    const laneEnds: number[] = [];
    return items.map((item) => {
      let lane = laneEnds.findIndex((end) => end <= item.start + 0.001);
      if (lane < 0) {
        lane = laneEnds.length;
        laneEnds.push(0);
      }
      laneEnds[lane] = item.end;
      return lane;
    });
  }

  function formatTick(seconds: number) {
    return formatTimecode(seconds, tickStep < 1 ? 1 : 0);
  }

  function timeAt(clientX: number) {
    const rect = viewport.getBoundingClientRect();
    return Math.max(0, (clientX - rect.left + viewport.scrollLeft) / pxPerSecond);
  }

  /** The nearest snap target within reach of `value`, in seconds. */
  function snap(value: number, targets: number[], enabled: boolean): number {
    if (!enabled) {
      snapTime = null;
      return value;
    }
    const threshold = SNAP_PX / pxPerSecond;
    let best: number | null = null;
    for (const target of targets) {
      if (Math.abs(target - value) <= threshold && (best === null || Math.abs(target - value) < Math.abs(best - value))) {
        best = target;
      }
    }
    snapTime = best;
    return best ?? value;
  }

  /** Snap whichever end of a `length` long span starting at `start` is closer to a target. */
  function snapSpan(start: number, length: number, targets: number[], enabled: boolean): number {
    snapTime = null;
    if (!enabled) return start;
    const threshold = SNAP_PX / pxPerSecond;
    let best: { target: number; start: number; distance: number } | null = null;
    for (const target of targets) {
      for (const [edge, aligned] of [[start, target], [start + length, target - length]]) {
        const distance = Math.abs(target - edge);
        if (distance <= threshold && (!best || distance < best.distance)) {
          best = { target, start: aligned, distance };
        }
      }
    }
    snapTime = best?.target ?? null;
    return best?.start ?? start;
  }

  function clipEdges(): number[] {
    return [...placed.map((p) => p.offset), mainLength];
  }

  // Drags follow the pointer through window listeners, so they continue
  // outside the timeline without pointer capture.

  // Ruler: scrub the playhead.
  function startScrub(event: PointerEvent) {
    if (event.button !== 0) return;
    onScrub?.(true);
    onSeek(Math.min(timeAt(event.clientX), duration));
    const move = (e: PointerEvent) => onSeek(Math.min(timeAt(e.clientX), duration));
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      onScrub?.(false);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  type SelectionKind = keyof TimelineSelection;
  const emptySelection = (): TimelineSelection => ({ clips: [], subtitles: [], overlays: [], audio: [] });
  let selection: TimelineSelection = $state(emptySelection());
  let selectionCount = $derived(Object.values(selection).reduce((sum, ids) => sum + ids.length, 0));
  let marquee: { x: number; y: number; width: number; height: number } | null = $state(null);
  let groupDrag: { pointerTime: number; pointerX: number; mainAnchor: number | null; moved: boolean; delta: number; index?: number } | null = $state(null);
  let groupPreview: ProjectTimeline | null = $state(null);
  let cleanupSelectionGesture = () => {};

  // History, deletion or remote updates may remove selected items.
  $effect(() => {
    const visible: TimelineSelection = {
      clips: placed.map((item) => item.clip.id),
      subtitles: subtitles.map((item) => item.subtitle.id),
      overlays: overlays.map((item) => item.item.id),
      audio: audio.map((item) => item.item.id),
    };
    const next = emptySelection();
    let changed = false;
    for (const kind of Object.keys(next) as SelectionKind[]) {
      const ids = new Set(visible[kind]);
      next[kind] = selection[kind].filter((id) => ids.has(id));
      changed ||= next[kind].length !== selection[kind].length;
    }
    if (changed) selection = next;
  });

  // Selecting something in the inspector or subtitle list exits a box selection.
  $effect(() => {
    if (selectionCount > 0 && (selectedClipId || selectedSubtitleId || selectedOverlayId || selectedAudioId)) {
      selection = emptySelection();
    }
  });

  function clearPrimarySelection() {
    onSelectClip(null);
    onSelectSubtitle(null);
    onSelectOverlay(null);
    onSelectAudio(null);
  }

  function selected(kind: SelectionKind, id: string) {
    return selection[kind].includes(id);
  }

  /** Include a normal single selection when extending it with a modifier. */
  function currentSelection(): TimelineSelection {
    const next: TimelineSelection = {
      clips: [...selection.clips], subtitles: [...selection.subtitles],
      overlays: [...selection.overlays], audio: [...selection.audio],
    };
    for (const [kind, primary] of [["clips", selectedClipId], ["subtitles", selectedSubtitleId], ["overlays", selectedOverlayId], ["audio", selectedAudioId]] as const) {
      if (primary && !next[kind].includes(primary)) next[kind].push(primary);
    }
    return next;
  }

  /** Empty track space starts a box; a click still seeks and clears selection. */
  function startMarquee(event: PointerEvent) {
    if (event.button !== 0 || (event.target as HTMLElement).closest("[data-item], .ruler")) return;
    event.preventDefault();
    cleanupSelectionGesture();
    const rect = content.getBoundingClientRect();
    const origin = { x: event.clientX - rect.left, y: event.clientY - rect.top };
    const previous = event.shiftKey || event.ctrlKey || event.metaKey ? currentSelection() : emptySelection();
    let moved = false;
    const move = (e: PointerEvent) => {
      const bounds = content.getBoundingClientRect();
      const x = e.clientX - bounds.left;
      const y = e.clientY - bounds.top;
      if (!moved && Math.hypot(x - origin.x, y - origin.y) < DRAG_THRESHOLD_PX) return;
      moved = true;
      clearPrimarySelection();
      const box = { x: Math.min(origin.x, x), y: Math.min(origin.y, y), width: Math.abs(x - origin.x), height: Math.abs(y - origin.y) };
      marquee = box;
      const next: TimelineSelection = { clips: [...previous.clips], subtitles: [...previous.subtitles], overlays: [...previous.overlays], audio: [...previous.audio] };
      for (const element of content.querySelectorAll<HTMLElement>("[data-item]")) {
        const item = element.getBoundingClientRect();
        const left = item.left - bounds.left;
        const top = item.top - bounds.top;
        if (left <= box.x + box.width && left + item.width >= box.x &&
            top <= box.y + box.height && top + item.height >= box.y) {
          const kind = element.dataset.kind as SelectionKind;
          const id = element.dataset.item!;
          if (!next[kind].includes(id)) next[kind].push(id);
        }
      }
      selection = next;
    };
    const finish = (e: PointerEvent) => {
      cleanupSelectionGesture();
      marquee = null;
      if (!moved && e.type !== "pointercancel") {
        selection = emptySelection();
        clearPrimarySelection();
        onSeek(Math.min(timeAt(e.clientX), duration));
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish);
    cleanupSelectionGesture = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish);
    };
  }

  /** Multiple selected items move together; single selections retain edge trimming. */
  function startSelectionDrag(event: PointerEvent, kind: SelectionKind, id: string): boolean {
    if (event.shiftKey || event.ctrlKey || event.metaKey) {
      const next = currentSelection();
      if (next[kind].includes(id)) next[kind] = next[kind].filter((value) => value !== id);
      else next[kind].push(id);
      clearPrimarySelection();
      selection = next;
      return true;
    }
    if (!selected(kind, id) || selectionCount < 2) {
      selection = emptySelection();
      return false;
    }
    cleanupSelectionGesture();
    const firstClip = placed.find((item) => selected("clips", item.clip.id));
    groupDrag = {
      pointerTime: timeAt(event.clientX), pointerX: event.clientX,
      mainAnchor: firstClip ? firstClip.offset + firstClip.duration / 2 : null,
      moved: false, delta: 0,
    };
    const move = (e: PointerEvent) => {
      const state = groupDrag;
      if (!state) return;
      if (!state.moved && Math.abs(e.clientX - state.pointerX) < DRAG_THRESHOLD_PX) return;
      const delta = timeAt(e.clientX) - state.pointerTime;
      const remaining = placed.filter((item) => !selection.clips.includes(item.clip.id));
      // Any selected item may be grabbed, including one far from the main clips.
      // Move the main group's anchor by the pointer delta, not to the pointer itself.
      const index = state.mainAnchor === null ? undefined : remaining.filter((item) => state.mainAnchor! + delta > item.offset + item.duration / 2).length;
      // Snap the group as a whole; never snap or clamp its members separately.
      const spans = [
        ...subtitles.filter((item) => selected("subtitles", item.subtitle.id)),
        ...overlays.filter((item) => selected("overlays", item.item.id)),
        ...audio.filter((item) => selected("audio", item.item.id)),
      ];
      const start = Math.min(...spans.map((item) => item.start));
      const end = Math.max(...spans.map((item) => item.end));
      const targets = [time, 0, mainLength,
        ...subtitles.filter((item) => !selected("subtitles", item.subtitle.id)).flatMap((item) => [item.start, item.end]),
        ...overlays.filter((item) => !selected("overlays", item.item.id)).flatMap((item) => [item.start, item.end]),
        ...audio.filter((item) => !selected("audio", item.item.id)).flatMap((item) => [item.start, item.end]),
        ...clipEdges()];
      const snapped = selection.clips.length || !spans.length ? delta : snapSpan(start + delta, end - start, targets, !e.shiftKey) - start;
      groupDrag = { ...state, moved: true, delta: snapped, index };
      groupPreview = moveTimelineSelection(timeline, selection, snapped, index);
    };
    const finish = (e: PointerEvent) => {
      const state = groupDrag;
      cleanupSelectionGesture();
      groupDrag = null;
      groupPreview = null;
      snapTime = null;
      if (state?.moved && e.type !== "pointercancel") {
        onCommit(moveTimelineSelection(timeline, selection, state.delta, state.index), timeline);
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish);
    cleanupSelectionGesture = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish);
    };
    return true;
  }

  let groupOffsets = $derived.by(() => {
    const offsets = new Map<string, number>();
    if (!groupPreview) return offsets;
    const clips = new Map(placeClips(groupPreview).map((item) => [item.clip.id, item.offset]));
    const subs = new Map(visibleSubtitles(groupPreview).map((item) => [item.subtitle.id, item.start]));
    const others = new Map([...placeOverlays(groupPreview), ...placeAudio(groupPreview)].map((item) => [item.item.id, item.start]));
    for (const item of placed) offsets.set(`clips:${item.clip.id}`, ((clips.get(item.clip.id) ?? item.offset) - item.offset) * pxPerSecond);
    for (const item of subtitles) offsets.set(`subtitles:${item.subtitle.id}`, ((subs.get(item.subtitle.id) ?? item.start) - item.start) * pxPerSecond);
    for (const [kind, items] of [["overlays", overlays], ["audio", audio]] as const) {
      for (const item of items) offsets.set(`${kind}:${item.item.id}`, ((others.get(item.item.id) ?? item.start) - item.start) * pxPerSecond);
    }
    return offsets;
  });

  function cancelSelection(event: KeyboardEvent | FocusEvent) {
    if (event instanceof KeyboardEvent && event.key !== "Escape") return;
    cleanupSelectionGesture();
    groupDrag = null;
    groupPreview = null;
    marquee = null;
    snapTime = null;
    selection = emptySelection();
  }

  onDestroy(() => cleanupSelectionGesture());

  // Clips: reorder by dragging the body, trim by dragging an edge.
  function startClipDrag(event: PointerEvent, item: PlacedClip) {
    if (event.button !== 0) return;
    event.stopPropagation();
    if (startSelectionDrag(event, "clips", item.clip.id)) return;
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const x = event.clientX - rect.left;
    const edgeSize = Math.min(EDGE_PX, rect.width / 3);
    const edge = x < edgeSize ? "start" : rect.width - x < edgeSize ? "end" : null;
    onSelectClip(item.clip.id);
    onSelectSubtitle(null);
    onSelectOverlay(null);
    onSelectAudio(null);
    if (edge) {
      drag = {
        kind: "trim",
        clipId: item.clip.id,
        edge,
        base: timeline,
        pointerX: event.clientX,
        edgeX: (edge === "start" ? item.offset : item.offset + item.duration) * pxPerSecond,
        deltaPx: 0,
      };
    } else {
      drag = {
        kind: "move",
        clipId: item.clip.id,
        pointerX: event.clientX,
        pointerY: event.clientY,
        pointerTime: timeAt(event.clientX),
        offset: item.offset,
        length: item.duration,
        moved: false,
        index: item.index,
        lift: null,
      };
    }
    window.addEventListener("pointermove", moveClipDrag);
    window.addEventListener("pointerup", endClipDrag);
  }

  /** Main-track index where a clip or material dropped at `clientX` goes. */
  function insertionIndex(clientX: number, excludeId: string | null = null) {
    const t = timeAt(clientX);
    let index = 0;
    for (const item of placed) {
      if (item.clip.id === excludeId) continue;
      if (t > item.offset + item.duration / 2) index++;
    }
    return index;
  }

  function insertionX(index: number, excludeId: string | null = null) {
    const others = placed.filter((item) => item.clip.id !== excludeId);
    if (others.length === 0) return 0;
    const item = others[Math.min(index, others.length - 1)];
    return (index >= others.length ? item.offset + item.duration : item.offset) * pxPerSecond;
  }

  function moveClipDrag(event: PointerEvent) {
    const state = drag;
    if (state?.kind !== "move" && state?.kind !== "trim") return;
    if (state.kind === "move") {
      if (
        !state.moved &&
        Math.abs(event.clientX - state.pointerX) < DRAG_THRESHOLD_PX &&
        Math.abs(event.clientY - state.pointerY) < DRAG_THRESHOLD_PX
      ) {
        return;
      }
      // Dragged up off the main track, it goes onto a video track; the main
      // track keeps at least one clip.
      if (event.clientY < mainTrack.getBoundingClientRect().top && canLiftClip(timeline, state.clipId)) {
        const at = state.offset + timeAt(event.clientX) - state.pointerTime;
        const snapped = snapSpan(Math.max(0, at), state.length, overlaySnapTargets(state.clipId), !event.shiftKey);
        // Starting over what is left once the clips after it close up.
        const ghost = Math.max(0, Math.min(snapped, latestStart - state.length));
        const target = overlayTarget(timeline, "video", trackTargetAt(event.clientY), ghost, ghost + state.length);
        drag = { ...state, moved: true, lift: { ghost, target } };
        return;
      }
      snapTime = null;
      drag = { ...state, moved: true, lift: null, index: insertionIndex(event.clientX, state.clipId) };
      return;
    }
    // Snap the edge to the playhead and to the other clips' edges.
    const edgeTime = (state.edgeX + event.clientX - state.pointerX) / pxPerSecond;
    const snapped = snap(edgeTime, [time, ...clipEdges()], !event.shiftKey);
    drag = { ...state, deltaPx: snapped * pxPerSecond - state.edgeX };
  }

  /** Material time the dragged edge points at. */
  function trimSource(state: TrimDrag): { clip: PlacedClip; source: number } | null {
    const item = placeClips(state.base).find((p) => p.clip.id === state.clipId);
    if (!item) return null;
    const delta = state.deltaPx / pxPerSecond;
    const source = state.edge === "start" ? item.clip.start + delta : item.clip.end + delta;
    return { clip: item, source };
  }

  function trimmed(state: TrimDrag): ProjectTimeline {
    const result = trimSource(state);
    if (!result) return state.base;
    const material = materials.get(result.clip.clip.material_id);
    return trimClip(state.base, state.clipId, state.edge, result.source, material?.duration ?? 0, frameRate);
  }

  function endClipDrag() {
    window.removeEventListener("pointermove", moveClipDrag);
    window.removeEventListener("pointerup", endClipDrag);
    const state = drag;
    drag = null;
    snapTime = null;
    if (state?.kind !== "move" && state?.kind !== "trim") return;
    if (state.kind === "move") {
      if (state.lift) {
        onLiftClip(state.clipId, state.lift.ghost, state.lift.target);
        return;
      }
      const from = timeline.clips.findIndex((clip) => clip.id === state.clipId);
      if (state.moved && state.index !== from) onCommit(moveClip(timeline, state.clipId, state.index), timeline);
      return;
    }
    if (Math.abs(state.deltaPx) < 0.5) return;
    // Applied to the timeline as it is now, which may have gained recognized
    // subtitles while dragging; the drag start only places the edge.
    const result = trimSource(state);
    if (!result) return;
    const material = materials.get(result.clip.clip.material_id);
    const next = trimClip(timeline, state.clipId, state.edge, result.source, material?.duration ?? 0, frameRate);
    onCommit(next, timeline);
    // Show the frame at the edit point.
    const item = placeClips(next).find((p) => p.clip.id === state.clipId);
    if (item) onSeek(state.edge === "start" ? item.offset : Math.max(item.offset, item.offset + item.duration - 0.04));
  }

  /** Geometry of a clip while it is being trimmed: the kept part and the change. */
  function trimOverlay(item: PlacedClip) {
    if (drag?.kind !== "trim" || drag.clipId !== item.clip.id) return null;
    const next = placeClips(trimmed(drag)).find((p) => p.clip.id === item.clip.id);
    if (!next) return null;
    const removed = next.duration < item.duration;
    const change = Math.abs(next.duration - item.duration) * pxPerSecond;
    const width = item.duration * pxPerSecond;
    // Trimming darkens the part being removed; extending shows the part added.
    const start = drag.edge === "start";
    const left = start ? (removed ? 0 : -change) : removed ? width - change : width;
    // Where the edge will be, in the clip's own coordinates.
    const tip = start ? (removed ? change : -change) : removed ? width - change : width + change;
    return {
      left,
      width: change,
      removed,
      tip,
      label: start ? "入点" : "出点",
      source: start ? next.clip.start : next.clip.end,
    };
  }

  // Subtitles: move by the body (re-anchored to the clip below), resize by an edge.
  function startSubtitleDrag(event: PointerEvent, item: PlacedSubtitle) {
    if (event.button !== 0) return;
    event.stopPropagation();
    if (startSelectionDrag(event, "subtitles", item.subtitle.id)) return;
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const x = event.clientX - rect.left;
    const edgeSize = Math.min(EDGE_PX, rect.width / 3);
    const mode = x < edgeSize ? "start" : rect.width - x < edgeSize ? "end" : "move";
    onSelectSubtitle(item.subtitle.id);
    onSelectClip(null);
    onSelectOverlay(null);
    onSelectAudio(null);
    drag = {
      kind: "subtitle",
      id: item.subtitle.id,
      mode,
      base: timeline,
      pointerTime: timeAt(event.clientX),
      start: item.start,
      end: item.end,
      moved: false,
    };
    window.addEventListener("pointermove", moveSubtitleDrag);
    window.addEventListener("pointerup", endSubtitleDrag);
  }

  function subtitleSnapTargets(excludeId: string) {
    const targets = [time, ...clipEdges()];
    for (const item of visibleSubtitles(timeline)) {
      if (item.subtitle.id !== excludeId) targets.push(item.start, item.end);
    }
    return targets;
  }

  function moveSubtitleDrag(event: PointerEvent) {
    if (drag?.kind !== "subtitle") return;
    const delta = timeAt(event.clientX) - drag.pointerTime;
    if (!drag.moved && Math.abs(delta * pxPerSecond) < DRAG_THRESHOLD_PX / 2) return;
    drag = { ...drag, moved: true };
    const targets = subtitleSnapTargets(drag.id);
    const enabled = !event.shiftKey;
    if (drag.mode === "move") {
      const length = drag.end - drag.start;
      const start = Math.max(0, snapSpan(Math.max(0, drag.start + delta), length, targets, enabled));
      // Positions are absolute, so they apply to the timeline as it is now.
      onPreview(moveSubtitle(timeline, drag.id, start, start + length));
    } else {
      const edgeTime = snap((drag.mode === "start" ? drag.start : drag.end) + delta, targets, enabled);
      onPreview(resizeSubtitle(timeline, drag.id, drag.mode, edgeTime));
    }
  }

  function endSubtitleDrag() {
    window.removeEventListener("pointermove", moveSubtitleDrag);
    window.removeEventListener("pointerup", endSubtitleDrag);
    const state = drag;
    drag = null;
    snapTime = null;
    if (state?.kind !== "subtitle") return;
    if (state.moved) onCommit(timeline, withSubtitleFrom(timeline, state.base, state.id));
    else onSeek(state.start);
  }

  // Overlays and audio items: move by the body, also onto another track of
  // theirs, trim by an edge.
  function startOverlayDrag(event: PointerEvent, item: PlacedOverlay | PlacedAudio, lane: OverlayDrag["lane"]) {
    if (event.button !== 0) return;
    event.stopPropagation();
    if (startSelectionDrag(event, lane === "audio" ? "audio" : "overlays", item.item.id)) return;
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const x = event.clientX - rect.left;
    const edgeSize = Math.min(EDGE_PX, rect.width / 3);
    const mode = x < edgeSize ? "start" : rect.width - x < edgeSize ? "end" : "move";
    if (lane === "audio") onSelectAudio(item.item.id);
    else onSelectOverlay(item.item.id);
    drag = {
      kind: "overlay",
      lane,
      trackKind: lane === "overlay" ? trackKindOf(item.item as PlacedOverlay["item"]) : null,
      id: item.item.id,
      mode,
      base: timeline,
      pointerTime: timeAt(event.clientX),
      pointerY: event.clientY,
      start: item.start,
      end: item.end,
      track: item.track,
      moved: false,
      ghost: item.start,
      target: { track: item.track },
      toMain: null,
    };
    window.addEventListener("pointermove", moveOverlayDrag);
    window.addEventListener("pointerup", endOverlayDrag);
  }

  function overlaySnapTargets(excludeId: string) {
    const targets = [time, ...clipEdges()];
    for (const item of [...overlays, ...audio]) {
      if (item.item.id !== excludeId) targets.push(item.start, item.end);
    }
    for (const item of subtitles) targets.push(item.start, item.end);
    return targets;
  }

  /** The audio track under the pointer, or a new one below them all. */
  function audioTargetAt(clientY: number): OverlayTarget {
    const row = Math.floor((clientY - audioArea.getBoundingClientRect().top) / AUDIO_TRACK_HEIGHT);
    return row >= 0 && row < audioRows.length ? { track: row } : { insert: audioRows.length };
  }

  /** Top of the gap a new audio track at `index` would fill, within the audio area. */
  function audioInsertionY(index: number) {
    return index * AUDIO_TRACK_HEIGHT;
  }

  /** The length of what an audio item plays, `0` when unknown. */
  function soundDuration(item: PlacedAudio["item"]) {
    return (item.kind === "file" ? audioFiles.get(item.audio_id) : materials.get(item.material_id))?.duration ?? 0;
  }

  /** The track above the main one under the pointer, or a new one above or below them all. */
  function trackTargetAt(clientY: number): OverlayTarget {
    let y = clientY - overlayArea.getBoundingClientRect().top - NEW_TRACK_HEIGHT;
    if (y < 0) return { insert: trackCount };
    for (const track of trackRows) {
      y -= trackHeights[track];
      if (y < 0) return { track };
    }
    return { insert: 0 };
  }

  /** Top of the gap a new track at `index` would fill, within the overlay area. */
  function insertionY(index: number) {
    let y = NEW_TRACK_HEIGHT;
    for (let track = trackCount - 1; track >= index; track--) y += trackHeights[track];
    return y;
  }

  function overMain(clientY: number) {
    const rect = mainTrack.getBoundingClientRect();
    return clientY >= rect.top && clientY < rect.bottom;
  }

  function moveOverlayDrag(event: PointerEvent) {
    const state = drag;
    if (state?.kind !== "overlay") return;
    const delta = timeAt(event.clientX) - state.pointerTime;
    if (
      !state.moved &&
      Math.abs(delta * pxPerSecond) < DRAG_THRESHOLD_PX &&
      Math.abs(event.clientY - state.pointerY) < DRAG_THRESHOLD_PX
    ) {
      return;
    }
    const targets = overlaySnapTargets(state.id);
    const enabled = !event.shiftKey;
    const sound = state.lane === "audio";
    if (state.mode === "move") {
      const length = state.end - state.start;
      const snapped = snapSpan(Math.max(0, state.start + delta), length, targets, enabled);
      const ghost = Math.max(0, Math.min(snapped, latestStart));
      // A video overlay dragged onto the main track goes there as a clip.
      if (state.trackKind === "video" && overMain(event.clientY)) {
        snapTime = null;
        drag = { ...state, moved: true, ghost, toMain: insertionIndex(event.clientX) };
        return;
      }
      let target: OverlayTarget;
      if (sound) {
        target = audioTargetAt(event.clientY);
        // Where the track is taken, an audio item gets a new one below it.
        if ("track" in target && !trackIsFree(audio, target.track, ghost, ghost + length, state.id)) {
          target = { insert: target.track + 1 };
        }
      } else {
        // Where the track is of the other kind or taken, an overlay gets a new one above it.
        target = overlayTarget(timeline, state.trackKind ?? "sticker", trackTargetAt(event.clientY), ghost, ghost + length, state.id);
      }
      drag = { ...state, moved: true, ghost, target, toMain: null };
      return;
    }
    const edgeTime = snap((state.mode === "start" ? state.start : state.end) + delta, targets, enabled);
    // From the item as the drag began, so the preview does not add up.
    if (sound) {
      const item = audio.find((entry) => entry.item.id === state.id)?.item;
      const from = withAudioFrom(timeline, state.base, state.id);
      onPreview(trimAudio(from, state.id, state.mode, edgeTime, item ? soundDuration(item) : 0));
    } else {
      const item = overlays.find((entry) => entry.item.id === state.id)?.item;
      const material = item?.kind === "video" ? materials.get(item.material_id) : undefined;
      const from = withOverlayFrom(timeline, state.base, state.id);
      onPreview(trimOverlayEdge(from, state.id, state.mode, edgeTime, material?.duration ?? 0));
    }
    drag = { ...state, moved: true };
  }

  function endOverlayDrag() {
    window.removeEventListener("pointermove", moveOverlayDrag);
    window.removeEventListener("pointerup", endOverlayDrag);
    const state = drag;
    drag = null;
    snapTime = null;
    if (state?.kind !== "overlay") return;
    if (!state.moved) {
      // A click: bring it into the preview.
      if (time < state.start || time >= state.end) onSeek(state.start);
      return;
    }
    const sound = state.lane === "audio";
    const before = (sound ? withAudioFrom : withOverlayFrom)(timeline, state.base, state.id);
    if (state.mode !== "move") {
      onCommit(timeline, before);
      return;
    }
    if (state.toMain !== null) {
      const result = overlayToClip(before, state.id, state.toMain, frameRate);
      if (!result.clipId) return;
      onCommit(result.timeline, before);
      onSelectClip(result.clipId);
      return;
    }
    const stayed = "track" in state.target && state.target.track === state.track;
    if (stayed && Math.abs(state.ghost - state.start) < 1e-6) return;
    onCommit((sound ? moveAudio : moveOverlay)(before, state.id, state.ghost, state.target), before);
  }

  function overlayDragOver(event: DragEvent) {
    if (!event.dataTransfer?.types.includes(MATERIAL_DRAG_TYPE) || mainLength < MIN_OVERLAY_DURATION) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    const at = Math.min(timeAt(event.clientX), latestStart);
    // A material shows on a video track.
    const target = overlayTarget(timeline, "video", trackTargetAt(event.clientY), at, at + MIN_OVERLAY_DURATION);
    overlayDrop = { time: at, target };
  }

  function overlayDropMaterial(event: DragEvent) {
    const id = Number(event.dataTransfer?.getData(MATERIAL_DRAG_TYPE));
    const drop = overlayDrop;
    overlayDrop = null;
    if (!Number.isFinite(id) || id <= 0 || !drop) return;
    event.preventDefault();
    onDropOverlay(id, drop.time, drop.target);
  }

  // Sounds dragged in from the audio library, or materials whose sound plays.
  function audioDragOver(event: DragEvent) {
    const types = event.dataTransfer?.types ?? [];
    if ((!types.includes(AUDIO_DRAG_TYPE) && !types.includes(MATERIAL_DRAG_TYPE)) || mainLength < MIN_OVERLAY_DURATION) {
      return;
    }
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    const at = Math.min(timeAt(event.clientX), latestStart);
    audioDrop = { time: at, target: audioTargetAt(event.clientY) };
  }

  function audioDropSound(event: DragEvent) {
    const drop = audioDrop;
    audioDrop = null;
    const file = Number(event.dataTransfer?.getData(AUDIO_DRAG_TYPE));
    const material = Number(event.dataTransfer?.getData(MATERIAL_DRAG_TYPE));
    const source: AudioDrop | null =
      file > 0 ? { kind: "file", id: file } : material > 0 ? { kind: "material", id: material } : null;
    if (!source || !drop) return;
    event.preventDefault();
    onDropAudio(source, drop.time, drop.target);
  }

  function soundLabel(item: PlacedAudio["item"]) {
    if (item.kind === "file") return audioFiles.get(item.audio_id)?.name ?? "音频已删除";
    return materials.get(item.material_id)?.name ?? "素材已删除";
  }

  function overlayLabel(item: PlacedOverlay["item"]) {
    if (item.kind === "text") return item.element.content.replace(/\n/g, " ") || "文字";
    if (item.kind === "sticker") return "贴纸";
    return materials.get(item.material_id)?.name ?? "素材已删除";
  }

  // Materials dragged in from the material bin.
  function dragOver(event: DragEvent) {
    if (!event.dataTransfer?.types.includes(MATERIAL_DRAG_TYPE)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    const at = Math.max(0, snap(timeAt(event.clientX), [time, ...clipEdges()], !event.shiftKey));
    materialDropTime = at < mainLength ? Math.min(at, latestStart) : mainLength;
  }

  function drop(event: DragEvent) {
    const id = Number(event.dataTransfer?.getData(MATERIAL_DRAG_TYPE));
    const dropTime = materialDropTime ?? timeAt(event.clientX);
    materialDropTime = null;
    snapTime = null;
    if (!Number.isFinite(id) || id <= 0) return;
    event.preventDefault();
    // Like ＋ at the playhead, but at the pointer: overlay, never cut the picture.
    onDropMaterial(id, dropTime);
  }

  function wheel(event: WheelEvent) {
    if (event.ctrlKey || event.metaKey) {
      event.preventDefault();
      // Zoom around the pointer.
      const anchor = timeAt(event.clientX);
      const nextZoom = Math.min(200, Math.max(1, zoom * (event.deltaY < 0 ? 1.15 : 1 / 1.15)));
      onZoom(nextZoom);
      requestAnimationFrame(() => {
        const rect = viewport.getBoundingClientRect();
        viewport.scrollLeft = anchor * pxPerSecond - (event.clientX - rect.left);
      });
    } else if (Math.abs(event.deltaY) > Math.abs(event.deltaX)) {
      event.preventDefault();
      viewport.scrollLeft += event.deltaY;
    }
  }

  // Waveform of `length` seconds of a source from `from`: the visible part of
  // a clip, or the part heard of an audio item.
  function waveform(
    canvas: HTMLCanvasElement,
    params: { data: MaterialPeaks | undefined; from: number; length: number; width: number },
  ) {
    let current = params;
    let drawn = params;
    // While zooming, the canvas stretches with its clip until the zoom settles.
    let settle: ReturnType<typeof setTimeout> | undefined;
    // The canvas's height as last laid out, told by an observer: reading it
    // after other canvases changed would lay the page out again for each clip.
    let height = 0;
    const observer = new ResizeObserver(([entry]) => {
      if (entry.contentRect.height === height) return;
      height = entry.contentRect.height;
      draw(current);
    });
    observer.observe(canvas);
    function draw(params: typeof current) {
      drawn = params;
      const { data, width } = params;
      const ratio = window.devicePixelRatio || 1;
      // Browsers cap canvas size; long clips at high zoom are drawn coarser.
      const pixels = Math.min(Math.max(1, Math.round(width * ratio)), 16000);
      // Until the observer tells, about what the track gives it.
      if (!height) height = 40;
      canvas.width = pixels;
      canvas.height = Math.round(height * ratio);
      const context = canvas.getContext("2d");
      if (!context) return;
      context.clearRect(0, 0, canvas.width, canvas.height);
      if (!data || data.peaks.length === 0) return;
      const from = params.from * data.rate;
      const span = params.length * data.rate;
      let max = 1;
      for (let i = Math.floor(from); i < Math.min(data.peaks.length, from + span); i++) max = Math.max(max, data.peaks[i]);
      context.fillStyle = "rgba(196, 181, 253, 0.55)";
      const middle = canvas.height / 2;
      for (let x = 0; x < pixels; x++) {
        const begin = Math.floor(from + (x / pixels) * span);
        const end = Math.max(begin + 1, Math.floor(from + ((x + 1) / pixels) * span));
        let peak = 0;
        for (let i = begin; i < end && i < data.peaks.length; i++) peak = Math.max(peak, data.peaks[i]);
        const h = Math.max(1, (peak / max) * middle * 0.95);
        context.fillRect(x, middle - h, 1, h * 2);
      }
    }
    draw(params);
    return {
      update(next: typeof params) {
        current = next;
        const same = next.data === drawn.data && next.from === drawn.from && next.length === drawn.length;
        clearTimeout(settle);
        if (same) {
          settle = setTimeout(() => draw(current), WAVE_SETTLE_MS);
        } else {
          draw(next);
        }
      },
      destroy() {
        clearTimeout(settle);
        observer.disconnect();
      },
    };
  }
</script>

<svelte:window onkeydown={cancelSelection} onblur={cancelSelection} ondragend={() => { materialDropTime = null; overlayDrop = null; audioDrop = null; snapTime = null; }} />

<div class="timeline" title="在轨道空白处拖动框选；Ctrl / Cmd / Shift 点击增减选择；拖动选中项一起移动；Esc 取消选择">
  <div class="labels">
    <div class="label ruler-label" style:height="{RULER_HEIGHT}px">时间</div>
    <div class="label" style:height="{subtitleTrackHeight}px" title="字幕轨道，始终在最上层">
      <Captions class="h-3.5 w-3.5 shrink-0 text-[#0a84ff]" />字幕
    </div>
    <div class="label new-track-label" style:height="{NEW_TRACK_HEIGHT}px">新轨道</div>
    {#each trackRows as track (track)}
      {@const kind = timeline.tracks[track].kind}
      <div
        class="label"
        style:height="{trackHeights[track]}px"
        title={kind === "video" ? "视频轨道：素材画面叠加在下层轨道之上" : "贴图轨道：文字与贴纸"}
      >
        {#if kind === "video"}
          <Film class="h-3.5 w-3.5 shrink-0 text-[#8b5cf6]" />视频
        {:else}
          <Sticker class="h-3.5 w-3.5 shrink-0 text-[#f59e0b]" />贴图
        {/if}
      </div>
    {/each}
    <div class="label main-label" style:height="{MAIN_TRACK_HEIGHT}px" title="主视频轨道：片段首尾相接，其他轨道依附于它">
      <Film class="h-3.5 w-3.5 shrink-0 text-[#8b5cf6]" />
      <span class="flex flex-col leading-tight">视频<span class="text-[10px] text-[#a78bfa]">主轨</span></span>
    </div>
    {#each audioRows as track (track)}
      <div class="label" style:height="{AUDIO_TRACK_HEIGHT}px" title="音频轨道，始终在最下层">
        <Music class="h-3.5 w-3.5 shrink-0 text-[#34d399]" />音频
      </div>
    {/each}
    <div class="label new-track-label" style:height="{NEW_TRACK_HEIGHT}px">新轨道</div>
  </div>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="viewport sidebar-scrollbar" bind:this={viewport} onwheel={wheel}>
    <!-- Clips, subtitles and overlays are placed by CSS from the scale here, so
         zooming sets one variable instead of thousands of positions. -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="content" bind:this={content} onpointerdown={startMarquee} style:width="{contentWidth}px" style:--pps={pxPerSecond}>
      {#if selectionCount > 0}
        <div class="selection-status">已选 {selectionCount} 项 · 拖动一起移动 · Esc 取消</div>
      {/if}
      {#if marquee}
        <div class="selection-box" style:left="{marquee.x}px" style:top="{marquee.y}px" style:width="{marquee.width}px" style:height="{marquee.height}px"></div>
      {/if}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="ruler" style:height="{RULER_HEIGHT}px" onpointerdown={startScrub}>
        {#each ticks as tick}
          <div class="tick" style:left="{tick * pxPerSecond}px">
            <span>{formatTick(tick)}</span>
          </div>
        {/each}
      </div>

      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="track subtitle-track" style:height="{subtitleTrackHeight}px">
        {#if subtitles.length === 0}
          <div class="track-hint">暂无字幕，可在右侧使用 AI 字幕或手动添加</div>
        {/if}
        {#each subtitles as item, index (item.subtitle.id)}
          {@const labelled = (item.end - item.start) * pxPerSecond >= LABEL_MIN_PX}
          <div
            class="subtitle"
            class:labelled
            class:selected={selectedSubtitleId === item.subtitle.id || selected("subtitles", item.subtitle.id)}
            data-kind="subtitles"
            data-item={item.subtitle.id}
            style:transform={groupPreview ? `translateX(${groupOffsets.get(`subtitles:${item.subtitle.id}`) ?? 0}px)` : undefined}
            class:active={playingSubtitles.has(item.subtitle.id)}
            role="button"
            tabindex="-1"
            style:--start={item.start}
            style:--length={item.end - item.start}
            style:top="{4 + lanes[index] * LANE_HEIGHT}px"
            title={item.subtitle.text}
            onpointerdown={(event) => startSubtitleDrag(event, item)}
          >
            {#if labelled}
              <span>{item.subtitle.text || "空字幕"}</span>
            {/if}
          </div>
        {/each}
      </div>

      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="overlay-area"
        bind:this={overlayArea}
        ondragover={overlayDragOver}
        ondragleave={(event) => {
          if (!overlayArea.contains(event.relatedTarget as Node)) overlayDrop = null;
        }}
        ondrop={overlayDropMaterial}
      >
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div class="track new-track" style:height="{NEW_TRACK_HEIGHT}px">
          {#if trackCount === 0}
            <div class="track-hint">从左侧添加文字、贴纸，或把素材、主轨片段拖到这里，叠加在画面上</div>
          {:else if hiddenOverlays > 0}
            <div class="track-hint">另有 {hiddenOverlays} 个叠加内容随片段裁剪被隐藏</div>
          {/if}
        </div>
        {#each trackRows as track (track)}
          {@const kind = timeline.tracks[track].kind}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            class="track overlay-track layer-{kind}"
            class:target={(ghost?.lane === "overlay" && "track" in ghost.target && ghost.target.track === track) ||
              (overlayDrop && "track" in overlayDrop.target && overlayDrop.target.track === track)}
            style:height="{trackHeights[track]}px"
          >
            {#each overlays.filter((entry) => entry.track === track) as item (item.item.id)}
              {@const width = (item.end - item.start) * pxPerSecond}
              {@const wide = width > 26}
              {@const wider = width > 64}
              {@const overlay = item.item}
              <div
                class="overlay {overlay.kind}"
                class:selected={selectedOverlayId === overlay.id || selected("overlays", overlay.id)}
                data-kind="overlays"
                data-item={overlay.id}
                style:transform={groupPreview ? `translateX(${groupOffsets.get(`overlays:${overlay.id}`) ?? 0}px)` : undefined}
                class:active={playingOverlays.has(item.item.id)}
                class:dragging={moving?.id === overlay.id}
                class:missing={overlay.kind === "video" && !materials.has(overlay.material_id)}
                role="button"
                tabindex="-1"
                style:--start={item.start}
                style:--length={item.end - item.start}
                title="{overlayLabel(overlay)}（{formatTimecode(item.end - item.start)}）"
                onpointerdown={(event) => startOverlayDrag(event, item, "overlay")}
              >
                {#if wide}
                  {#if overlay.kind === "text"}
                    <Type class="h-3 w-3 shrink-0" />
                  {:else if overlay.kind === "sticker"}
                    {#if stickerUrls.get(overlay.element.src)}
                      <img class="sticker-thumb" src={stickerUrls.get(overlay.element.src)} alt="" draggable="false" />
                    {:else}
                      <Smile class="h-3 w-3 shrink-0" />
                    {/if}
                  {:else if materials.get(overlay.material_id)?.coverUrl && wider}
                    <img class="video-thumb" src={materials.get(overlay.material_id).coverUrl} alt="" draggable="false" />
                  {:else}
                    <Film class="h-3 w-3 shrink-0" />
                  {/if}
                {/if}
                {#if width >= LABEL_MIN_PX}
                  <span class="overlay-name">{overlayLabel(overlay)}</span>
                {/if}
                <span class="edge start"></span>
                <span class="edge end"></span>
              </div>
            {/each}
            {#if ghost?.lane === "overlay" && "track" in ghost.target && ghost.target.track === track}
              <div
                class="overlay-ghost"
                style:left="{ghost.start * pxPerSecond}px"
                style:width="{Math.max(3, ghost.length * pxPerSecond)}px"
              ></div>
            {/if}
            {#if overlayDrop && "track" in overlayDrop.target && overlayDrop.target.track === track}
              <div class="insert-marker" style:left="{overlayDrop.time * pxPerSecond}px"></div>
            {/if}
          </div>
        {/each}
        {#if ghost?.lane === "overlay" && "insert" in ghost.target}
          <div
            class="new-track-bar"
            style:top="{insertionY(ghost.target.insert)}px"
            style:left="{ghost.start * pxPerSecond}px"
            style:width="{Math.max(3, ghost.length * pxPerSecond)}px"
          ></div>
        {/if}
        {#if overlayDrop && "insert" in overlayDrop.target}
          <div
            class="new-track-bar"
            style:top="{insertionY(overlayDrop.target.insert)}px"
            style:left="{overlayDrop.time * pxPerSecond}px"
            style:width="80px"
          ></div>
        {/if}
      </div>

      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="track video-track"
        class:target={moving?.toMain != null}
        bind:this={mainTrack}
        style:height="{MAIN_TRACK_HEIGHT}px"
        ondragover={dragOver}
        ondragleave={(event) => {
          if (!mainTrack.contains(event.relatedTarget as Node)) { materialDropTime = null; snapTime = null; }
        }}
        ondrop={drop}
      >
        {#if placed.length === 0}
          <div class="track-hint">从左侧素材库拖入或点击「＋」添加素材</div>
        {/if}
        {#each placed as item (item.clip.id)}
          {@const material = materials.get(item.clip.material_id)}
          {@const width = item.duration * pxPerSecond}
          {@const wide = width > 64}
          {@const labelled = width >= LABEL_MIN_PX}
          {@const overlay = trimOverlay(item)}
          <div
            class="clip"
            class:labelled
            class:selected={selectedClipId === item.clip.id || selected("clips", item.clip.id)}
            data-kind="clips"
            data-item={item.clip.id}
            style:transform={groupPreview ? `translateX(${groupOffsets.get(`clips:${item.clip.id}`) ?? 0}px)` : undefined}
            class:dragging={drag?.kind === "move" && drag.moved && drag.clipId === item.clip.id}
            class:missing={!material}
            role="button"
            tabindex="-1"
            style:--start={item.offset}
            style:--length={item.duration}
            title="{material?.name ?? "素材已删除"}（{formatTimecode(item.duration)}）"
            onpointerdown={(event) => startClipDrag(event, item)}
          >
            {#if labelled}
              <div class="clip-head">
                {#if material?.coverUrl && wide}
                  <img src={material.coverUrl} alt="" draggable="false" />
                {:else}
                  <Film class="h-3 w-3 shrink-0 text-violet-300" />
                {/if}
                <span class="clip-name">{material?.name ?? "素材已删除"}</span>
                <span class="clip-length">{formatTimecode(item.duration)}</span>
              </div>
            {/if}
            <canvas
              class="clip-wave"
              use:waveform={{ data: peaks.get(item.clip.material_id), from: item.clip.start, length: item.duration, width }}
            ></canvas>
            <span class="edge start"></span>
            <span class="edge end"></span>
            {#if overlay}
              <div
                class="trim-overlay"
                class:removed={overlay.removed}
                style:left="{overlay.left}px"
                style:width="{overlay.width}px"
              ></div>
              <div class="trim-tip" style:left="{overlay.tip}px">
                {overlay.label} {formatTimecode(overlay.source)}
              </div>
            {/if}
          </div>
        {/each}
        {#if drag?.kind === "move" && drag.moved && !drag.lift}
          <div class="insert-marker" style:left="{insertionX(drag.index, drag.clipId)}px"></div>
        {/if}
        {#if moving?.toMain != null}
          <div class="insert-marker" style:left="{insertionX(moving.toMain)}px"></div>
        {/if}
        {#if materialDropTime !== null}
          <div class="insert-marker" style:left="{Math.min(materialDropTime, mainLength) * pxPerSecond}px">
            <span class="drop-label">{mainLength >= MIN_OVERLAY_DURATION && materialDropTime < mainLength ? "叠加视频" : "添加到主轨"} · {formatTimecode(Math.min(materialDropTime, mainLength))}</span>
          </div>
        {/if}
      </div>

      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="audio-area"
        bind:this={audioArea}
        ondragover={audioDragOver}
        ondragleave={(event) => {
          if (!audioArea.contains(event.relatedTarget as Node)) audioDrop = null;
        }}
        ondrop={audioDropSound}
      >
        {#each audioRows as track (track)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            class="track audio-track"
            class:target={(ghost?.lane === "audio" && "track" in ghost.target && ghost.target.track === track) ||
              (audioDrop && "track" in audioDrop.target && audioDrop.target.track === track)}
            style:height="{AUDIO_TRACK_HEIGHT}px"
          >
            {#each audio.filter((entry) => entry.track === track) as item (item.item.id)}
              {@const width = (item.end - item.start) * pxPerSecond}
              {@const sound = item.item}
              {@const fade = audioFades(item)}
              <div
                class="sound"
                class:labelled={width >= LABEL_MIN_PX}
                class:of-material={sound.kind === "material"}
                class:selected={selectedAudioId === sound.id || selected("audio", sound.id)}
                data-kind="audio"
                data-item={sound.id}
                style:transform={groupPreview ? `translateX(${groupOffsets.get(`audio:${sound.id}`) ?? 0}px)` : undefined}
                class:active={playingAudio.has(sound.id)}
                class:dragging={moving?.id === sound.id}
                class:missing={sound.kind === "file" ? !audioFiles.has(sound.audio_id) : !materials.has(sound.material_id)}
                class:muted={sound.volume <= 0}
                role="button"
                tabindex="-1"
                style:--start={item.start}
                style:--length={item.end - item.start}
                title="{soundLabel(sound)}（{formatTimecode(item.end - item.start)}，音量 {Math.round(sound.volume * 100)}%）"
                onpointerdown={(event) => startOverlayDrag(event, item, "audio")}
              >
                <canvas
                  class="sound-wave"
                  use:waveform={{
                    data: sound.kind === "file" ? audioPeaks.get(sound.audio_id) : peaks.get(sound.material_id),
                    from: sound.source_start + item.skip,
                    length: item.end - item.start,
                    width,
                  }}
                ></canvas>
                {#if fade.fadeIn > 0}
                  <span class="fade in" style:width="{fade.fadeIn * pxPerSecond}px"></span>
                {/if}
                {#if fade.fadeOut > 0}
                  <span class="fade out" style:width="{fade.fadeOut * pxPerSecond}px"></span>
                {/if}
                {#if width >= LABEL_MIN_PX}
                  <span class="sound-name">
                    {#if width > 26}
                      {#if sound.kind === "file"}
                        <Music class="h-3 w-3 shrink-0" />
                      {:else}
                        <AudioLines class="h-3 w-3 shrink-0" />
                      {/if}
                    {/if}
                    <span class="truncate">{soundLabel(sound)}</span>
                  </span>
                {/if}
                <span class="edge start"></span>
                <span class="edge end"></span>
              </div>
            {/each}
            {#if ghost?.lane === "audio" && "track" in ghost.target && ghost.target.track === track}
              <div
                class="overlay-ghost"
                style:left="{ghost.start * pxPerSecond}px"
                style:width="{Math.max(3, ghost.length * pxPerSecond)}px"
              ></div>
            {/if}
            {#if audioDrop && "track" in audioDrop.target && audioDrop.target.track === track}
              <div class="insert-marker" style:left="{audioDrop.time * pxPerSecond}px"></div>
            {/if}
          </div>
        {/each}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div class="track new-track audio-new-track" style:height="{NEW_TRACK_HEIGHT}px">
          {#if audioRows.length === 0}
            <div class="track-hint">把音频库里的音乐、配音，或素材拖到这里，只用它的声音</div>
          {:else if hiddenAudio > 0}
            <div class="track-hint">另有 {hiddenAudio} 段音频随片段裁剪被隐藏</div>
          {/if}
        </div>
        {#if ghost?.lane === "audio" && "insert" in ghost.target}
          <div
            class="new-track-bar"
            style:top="{audioInsertionY(ghost.target.insert)}px"
            style:left="{ghost.start * pxPerSecond}px"
            style:width="{Math.max(3, ghost.length * pxPerSecond)}px"
          ></div>
        {/if}
        {#if audioDrop && "insert" in audioDrop.target}
          <div
            class="new-track-bar"
            style:top="{audioInsertionY(audioDrop.target.insert)}px"
            style:left="{audioDrop.time * pxPerSecond}px"
            style:width="80px"
          ></div>
        {/if}
      </div>

      {#each ticks as tick}
        <div class="grid-line" style:left="{tick * pxPerSecond}px" style:top="{RULER_HEIGHT}px"></div>
      {/each}
      {#if snapTime !== null}
        <div class="snap-line" style:left="{snapTime * pxPerSecond}px"></div>
      {/if}
      <div class="playhead" style:left="{time * pxPerSecond}px">
        <span></span>
      </div>
    </div>
  </div>
</div>

<style>
  .selection-box {
    position: absolute;
    z-index: 30;
    border: 1px solid #55b9ff;
    background: rgb(10 132 255 / 18%);
    pointer-events: none;
  }

  .selection-status {
    position: sticky;
    left: 8px;
    top: 0;
    z-index: 31;
    width: max-content;
    height: 0;
    margin-left: 8px;
    color: #8dcdff;
    font-size: 10px;
    pointer-events: none;
    transform: translateY(25px);
  }

  .drop-label {
    position: absolute;
    top: -20px;
    padding: 2px 5px;
    border-radius: 3px;
    background: #0a84ff;
    color: white;
    white-space: nowrap;
    font-size: 10px;
  }

  .timeline {
    display: flex;
    max-height: 50vh;
    overflow-y: auto;
    border-top: 1px solid rgb(31 41 55 / 50%);
    background: #1c1c1e;
    user-select: none;
  }

  .labels {
    display: flex;
    width: 72px;
    height: max-content;
    flex-shrink: 0;
    flex-direction: column;
    border-right: 1px solid rgb(31 41 55 / 70%);
    background: #242426;
    color: #9ca3af;
    font-size: 12px;
  }

  .label {
    display: flex;
    align-items: center;
    gap: 8px;
    white-space: nowrap;
    border-bottom: 1px solid rgb(31 41 55 / 70%);
    padding: 0 12px;
  }

  .ruler-label {
    font-size: 10px;
    letter-spacing: 0.05em;
  }

  .main-label {
    background: rgb(139 92 246 / 8%);
  }

  .new-track-label {
    font-size: 10px;
    letter-spacing: 0.05em;
  }

  .viewport {
    position: relative;
    flex: 1;
    height: max-content;
    overflow-x: auto;
    overflow-y: hidden;
  }

  .content {
    position: relative;
    min-width: 100%;
  }

  .ruler {
    position: relative;
    border-bottom: 1px solid rgb(31 41 55 / 70%);
    background: #202022;
    cursor: pointer;
  }

  .tick {
    position: absolute;
    top: 0;
    bottom: 0;
    border-left: 1px solid rgb(75 85 99 / 70%);
  }

  .tick span {
    position: absolute;
    top: 4px;
    left: 4px;
    color: #9ca3af;
    font-size: 10px;
    white-space: nowrap;
  }

  .grid-line {
    position: absolute;
    bottom: 0;
    border-left: 1px solid rgb(55 65 81 / 35%);
    pointer-events: none;
  }

  .track {
    position: relative;
    border-bottom: 1px solid rgb(31 41 55 / 70%);
  }

  .subtitle-track {
    background: rgb(23 37 84 / 10%);
  }

  .video-track {
    background: rgb(46 16 101 / 10%);
  }

  .video-track.target {
    background: rgb(139 92 246 / 16%);
  }

  .overlay-area {
    position: relative;
  }

  .new-track {
    background: repeating-linear-gradient(
      -45deg,
      rgb(20 184 166 / 4%) 0 6px,
      transparent 6px 12px
    );
  }

  .overlay-track {
    background: rgb(120 53 15 / 8%);
  }

  .overlay-track.layer-video {
    background: rgb(46 16 101 / 8%);
  }

  .audio-area {
    position: relative;
  }

  .audio-track {
    background: rgb(6 78 59 / 10%);
  }

  .audio-track.target {
    background: rgb(52 211 153 / 14%);
  }

  .audio-new-track {
    background: repeating-linear-gradient(
      -45deg,
      rgb(52 211 153 / 4%) 0 6px,
      transparent 6px 12px
    );
  }

  .sound {
    position: absolute;
    top: 4px;
    bottom: 4px;
    left: calc(var(--start) * var(--pps) * 1px);
    width: max(3px, calc(var(--length) * var(--pps) * 1px));
    z-index: 10;
    overflow: hidden;
    border: 1px solid rgb(52 211 153 / 50%);
    background: rgb(52 211 153 / 14%);
    color: #d1fae5;
    font-size: 10px;
    cursor: grab;
  }

  .sound.labelled {
    border-radius: 6px;
  }

  .sound.of-material {
    border-color: rgb(167 139 250 / 50%);
    background: rgb(139 92 246 / 14%);
    color: #ede9fe;
  }

  .sound.active {
    filter: brightness(1.25);
  }

  .sound.selected {
    z-index: 20;
    box-shadow: 0 0 0 1px rgb(255 255 255 / 85%);
  }

  .sound.dragging {
    opacity: 0.45;
    cursor: grabbing;
  }

  .sound.missing {
    border-color: rgb(248 113 113 / 60%);
    background: rgb(248 113 113 / 12%);
  }

  .sound.muted .sound-wave {
    opacity: 0.35;
  }

  .sound:hover .edge,
  .sound.selected .edge {
    background: rgb(255 255 255 / 25%);
  }

  .sound-wave {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }

  .sound-name {
    position: absolute;
    top: 2px;
    right: 6px;
    left: 6px;
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 4px;
    pointer-events: none;
  }

  /* The quieter part of a fade, darkened above its ramp. */
  .fade {
    position: absolute;
    top: 0;
    bottom: 0;
    pointer-events: none;
  }

  .fade.in {
    left: 0;
    background: linear-gradient(to bottom right, rgb(0 0 0 / 45%) 50%, transparent 50%);
  }

  .fade.out {
    right: 0;
    background: linear-gradient(to bottom left, rgb(0 0 0 / 45%) 50%, transparent 50%);
  }

  .overlay-track.target {
    background: rgb(245 158 11 / 12%);
  }

  .overlay-track.layer-video.target {
    background: rgb(139 92 246 / 16%);
  }

  .overlay {
    position: absolute;
    top: 4px;
    bottom: 4px;
    left: calc(var(--start) * var(--pps) * 1px);
    width: max(3px, calc(var(--length) * var(--pps) * 1px));
    z-index: 10;
    display: flex;
    align-items: center;
    gap: 5px;
    overflow: hidden;
    /* Videos look like the main track's clips. */
    border: 1px solid rgb(139 92 246 / 50%);
    border-radius: 6px;
    background: rgb(139 92 246 / 18%);
    padding: 0 6px;
    color: #ede9fe;
    font-size: 10px;
    cursor: grab;
  }

  .overlay.text {
    border-color: rgb(245 158 11 / 55%);
    background: rgb(245 158 11 / 18%);
    color: #fef3c7;
  }

  .overlay.sticker {
    border-color: rgb(236 72 153 / 55%);
    background: rgb(236 72 153 / 18%);
    color: #fce7f3;
  }

  .overlay.active {
    filter: brightness(1.25);
  }

  .overlay.selected {
    z-index: 20;
    box-shadow: 0 0 0 1px rgb(255 255 255 / 85%);
  }

  .overlay.dragging {
    opacity: 0.45;
    cursor: grabbing;
  }

  .overlay.missing {
    border-color: rgb(248 113 113 / 60%);
    background: rgb(248 113 113 / 12%);
  }

  .overlay:hover .edge,
  .overlay.selected .edge {
    background: rgb(255 255 255 / 25%);
  }

  .overlay-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sticker-thumb {
    height: 18px;
    width: 18px;
    flex-shrink: 0;
    object-fit: contain;
  }

  .video-thumb {
    height: 30px;
    width: 52px;
    flex-shrink: 0;
    border-radius: 3px;
    object-fit: cover;
  }

  .overlay-ghost {
    position: absolute;
    top: 4px;
    bottom: 4px;
    z-index: 25;
    border: 1px dashed #fbbf24;
    border-radius: 6px;
    background: rgb(251 191 36 / 12%);
    pointer-events: none;
  }

  .new-track-bar {
    position: absolute;
    z-index: 30;
    height: 4px;
    margin-top: -2px;
    border-radius: 2px;
    background: #fbbf24;
    pointer-events: none;
  }

  .track-hint {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    padding-left: 16px;
    color: #4b5563;
    font-size: 12px;
    pointer-events: none;
  }

  .subtitle {
    position: absolute;
    left: calc(var(--start) * var(--pps) * 1px);
    width: max(2px, calc(var(--length) * var(--pps) * 1px));
    z-index: 10;
    display: flex;
    height: 24px;
    align-items: center;
    overflow: hidden;
    border: 1px solid rgb(10 132 255 / 50%);
    /* Rounded only when labelled: thousands of rounded bordered blocks cost the most to paint at every zoom. */
    background: rgb(10 132 255 / 20%);
    padding: 0 6px;
    color: #dbeafe;
    font-size: 10px;
    cursor: move;
  }

  .subtitle span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .subtitle.active {
    border-color: rgb(10 132 255 / 90%);
    background: rgb(10 132 255 / 30%);
  }

  .subtitle.selected {
    z-index: 20;
    box-shadow: 0 0 0 1px rgb(255 255 255 / 80%);
  }

  .subtitle.labelled,
  .clip.labelled {
    border-radius: 6px;
  }

  /* Without its heading, a clip's waveform fills it. */
  .clip:not(.labelled) .clip-wave {
    top: 0;
    height: 100%;
  }

  .clip {
    position: absolute;
    top: 4px;
    bottom: 4px;
    left: calc(var(--start) * var(--pps) * 1px);
    width: max(3px, calc(var(--length) * var(--pps) * 1px));
    overflow: visible;
    border: 1px solid rgb(139 92 246 / 50%);
    /* As subtitles, rounded only when labelled. */
    background: rgb(139 92 246 / 15%);
    cursor: grab;
  }

  .clip.selected {
    z-index: 5;
    border-color: #c4b5fd;
    box-shadow: 0 0 0 1px #c4b5fd;
  }

  .clip.dragging {
    opacity: 0.55;
    cursor: grabbing;
  }

  .clip.missing {
    border-color: rgb(248 113 113 / 60%);
    background: rgb(248 113 113 / 12%);
  }

  .clip-head {
    display: flex;
    height: 22px;
    align-items: center;
    gap: 6px;
    overflow: hidden;
    border-radius: 5px 5px 0 0;
    background: rgb(139 92 246 / 20%);
    padding: 0 6px;
    color: #ede9fe;
    font-size: 11px;
  }

  .clip-head img {
    height: 16px;
    width: 28px;
    flex-shrink: 0;
    border-radius: 3px;
    object-fit: cover;
  }

  .clip-name {
    min-width: 0;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .clip-length {
    flex-shrink: 0;
    color: rgb(221 214 254 / 70%);
    font-family: ui-monospace, monospace;
    font-size: 10px;
  }

  .clip-wave {
    position: absolute;
    top: 22px;
    right: 0;
    bottom: 0;
    left: 0;
    width: 100%;
    height: calc(100% - 22px);
    pointer-events: none;
  }

  .edge {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 7px;
    cursor: ew-resize;
  }

  .edge.start {
    left: -1px;
    border-radius: 6px 0 0 6px;
  }

  .edge.end {
    right: -1px;
    border-radius: 0 6px 6px 0;
  }

  .clip:hover .edge,
  .clip.selected .edge {
    background: rgb(196 181 253 / 45%);
  }

  .trim-overlay {
    position: absolute;
    top: -1px;
    bottom: -1px;
    border-radius: 6px;
    background: rgb(196 181 253 / 25%);
    outline: 1px dashed #c4b5fd;
    pointer-events: none;
  }

  .trim-overlay.removed {
    background: rgb(0 0 0 / 55%);
  }

  .trim-tip {
    position: absolute;
    top: -22px;
    z-index: 30;
    transform: translateX(-50%);
    border-radius: 4px;
    background: #111827;
    padding: 2px 6px;
    color: #ede9fe;
    font-family: ui-monospace, monospace;
    font-size: 10px;
    white-space: nowrap;
    pointer-events: none;
  }

  .insert-marker {
    position: absolute;
    top: 0;
    bottom: 0;
    z-index: 30;
    width: 2px;
    margin-left: -1px;
    background: #fbbf24;
    pointer-events: none;
  }

  .snap-line {
    position: absolute;
    top: 24px;
    bottom: 0;
    z-index: 35;
    width: 1px;
    background: #fcd34d;
    pointer-events: none;
  }

  .playhead {
    position: absolute;
    top: 0;
    bottom: 0;
    z-index: 40;
    width: 1px;
    background: #0a84ff;
    pointer-events: none;
  }

  .playhead span {
    position: absolute;
    top: 0;
    left: 50%;
    width: 10px;
    height: 10px;
    transform: translateX(-50%);
    border-radius: 0 0 2px 2px;
    background: #0a84ff;
  }
</style>
