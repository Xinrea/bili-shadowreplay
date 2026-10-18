<script lang="ts" module>
  export interface MaterialInfo {
    id: number;
    name: string;
    coverUrl: string;
    url: string;
    duration: number;
  }

  /** dataTransfer type used when dragging a material onto the timeline. */
  export const MATERIAL_DRAG_TYPE = "application/x-bsr-material";
</script>

<script lang="ts">
  import { Film } from "lucide-svelte";
  import type { MaterialPeaks, ProjectTimeline } from "../../interface";
  import {
    moveClip,
    moveSubtitle,
    placeClips,
    resizeSubtitle,
    timelineDuration,
    trimClip,
    visibleSubtitles,
    type PlacedClip,
    type PlacedSubtitle,
  } from "../../project-timeline";

  interface Props {
    timeline: ProjectTimeline;
    materials: Map<number, MaterialInfo>;
    peaks: Map<number, MaterialPeaks>;
    time: number;
    /** 1 fits the whole project into view. */
    zoom: number;
    selectedClipId: string | null;
    selectedSubtitleId: string | null;
    onSeek: (time: number) => void;
    onScrub?: (active: boolean) => void;
    /** Show an edit while it is being dragged. */
    onPreview: (timeline: ProjectTimeline) => void;
    /** Finish an edit that started from `before`. */
    onCommit: (timeline: ProjectTimeline, before: ProjectTimeline) => void;
    onSelectClip: (id: string | null) => void;
    onSelectSubtitle: (id: string | null) => void;
    onDropMaterial: (materialId: number, index: number) => void;
    onZoom: (zoom: number) => void;
  }

  let {
    timeline,
    materials,
    peaks,
    time,
    zoom,
    selectedClipId,
    selectedSubtitleId,
    onSeek,
    onScrub,
    onPreview,
    onCommit,
    onSelectClip,
    onSelectSubtitle,
    onDropMaterial,
    onZoom,
  }: Props = $props();

  const RULER_HEIGHT = 24;
  const LANE_HEIGHT = 28;
  const VIDEO_TRACK_HEIGHT = 76;
  const EDGE_PX = 7;
  const SNAP_PX = 8;
  const DRAG_THRESHOLD_PX = 4;
  const TRAILING_PX = 160;
  const TICK_STEPS = [0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];

  let viewport: HTMLElement = $state();
  let viewportWidth = $state(800);
  let snapTime: number | null = $state(null);
  let dropIndex: number | null = $state(null);

  type ClipDrag = {
    kind: "move";
    clipId: string;
    pointerX: number;
    moved: boolean;
    index: number;
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
  let drag: ClipDrag | TrimDrag | SubtitleDrag | null = $state(null);

  let duration = $derived(timelineDuration(timeline));
  let pxPerSecond = $derived(
    Math.max(0.0005, ((viewportWidth - 48) / Math.max(duration, 10)) * zoom),
  );
  let contentWidth = $derived(Math.max(viewportWidth, duration * pxPerSecond + TRAILING_PX));
  let placed = $derived(placeClips(timeline));
  let subtitles = $derived(visibleSubtitles(timeline));
  let lanes = $derived(assignLanes(subtitles));
  let laneCount = $derived(Math.max(1, ...lanes.map((lane) => lane + 1)));
  let subtitleTrackHeight = $derived(laneCount * LANE_HEIGHT + 8);
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
    if (!viewport || drag) return;
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
    const total = Math.round(seconds * 10) / 10;
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const secs = total % 60;
    const secText = tickStep < 1 ? secs.toFixed(1).padStart(4, "0") : String(Math.floor(secs)).padStart(2, "0");
    return hours > 0 ? `${hours}:${String(minutes).padStart(2, "0")}:${secText}` : `${minutes}:${secText}`;
  }

  function formatDuration(seconds: number) {
    const minutes = Math.floor(seconds / 60);
    return `${minutes}:${(seconds % 60).toFixed(1).padStart(4, "0")}`;
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
    return [...placed.map((p) => p.offset), duration];
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

  function clickEmpty(event: MouseEvent) {
    if (event.target !== event.currentTarget) return;
    onSelectClip(null);
    onSelectSubtitle(null);
    onSeek(Math.min(timeAt(event.clientX), duration));
  }

  // Clips: reorder by dragging the body, trim by dragging an edge.
  function startClipDrag(event: PointerEvent, item: PlacedClip) {
    if (event.button !== 0) return;
    event.stopPropagation();
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const x = event.clientX - rect.left;
    const edge = x < EDGE_PX ? "start" : rect.width - x < EDGE_PX ? "end" : null;
    onSelectClip(item.clip.id);
    onSelectSubtitle(null);
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
      drag = { kind: "move", clipId: item.clip.id, pointerX: event.clientX, moved: false, index: item.index };
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
    if (!state || state.kind === "subtitle") return;
    if (state.kind === "move") {
      if (!state.moved && Math.abs(event.clientX - state.pointerX) < DRAG_THRESHOLD_PX) return;
      drag = { ...state, moved: true, index: insertionIndex(event.clientX, state.clipId) };
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
    return trimClip(state.base, state.clipId, state.edge, result.source, material?.duration ?? 0);
  }

  function endClipDrag() {
    window.removeEventListener("pointermove", moveClipDrag);
    window.removeEventListener("pointerup", endClipDrag);
    const state = drag;
    drag = null;
    snapTime = null;
    if (!state || state.kind === "subtitle") return;
    if (state.kind === "move") {
      const from = timeline.clips.findIndex((clip) => clip.id === state.clipId);
      if (state.moved && state.index !== from) onCommit(moveClip(timeline, state.clipId, state.index), timeline);
      return;
    }
    if (Math.abs(state.deltaPx) < 0.5) return;
    const next = trimmed(state);
    onCommit(next, state.base);
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
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const x = event.clientX - rect.left;
    const edgeSize = Math.min(EDGE_PX, rect.width / 3);
    const mode = x < edgeSize ? "start" : rect.width - x < edgeSize ? "end" : "move";
    onSelectSubtitle(item.subtitle.id);
    onSelectClip(null);
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
      onPreview(moveSubtitle(drag.base, drag.id, start, start + length));
    } else {
      const edgeTime = snap((drag.mode === "start" ? drag.start : drag.end) + delta, targets, enabled);
      onPreview(resizeSubtitle(drag.base, drag.id, drag.mode, edgeTime));
    }
  }

  function endSubtitleDrag() {
    window.removeEventListener("pointermove", moveSubtitleDrag);
    window.removeEventListener("pointerup", endSubtitleDrag);
    const state = drag;
    drag = null;
    snapTime = null;
    if (state?.kind !== "subtitle") return;
    if (state.moved) onCommit(timeline, state.base);
    else onSeek(state.start);
  }

  // Materials dragged in from the material bin.
  function dragOver(event: DragEvent) {
    if (!event.dataTransfer?.types.includes(MATERIAL_DRAG_TYPE)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    dropIndex = insertionIndex(event.clientX);
  }

  function drop(event: DragEvent) {
    const id = Number(event.dataTransfer?.getData(MATERIAL_DRAG_TYPE));
    const index = dropIndex ?? timeline.clips.length;
    dropIndex = null;
    if (!Number.isFinite(id) || id <= 0) return;
    event.preventDefault();
    onDropMaterial(id, index);
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

  // Waveform of the visible part of a clip.
  function waveform(canvas: HTMLCanvasElement, params: { item: PlacedClip; data: MaterialPeaks | undefined; width: number }) {
    function draw({ item, data, width }: typeof params) {
      const ratio = window.devicePixelRatio || 1;
      // Browsers cap canvas size; long clips at high zoom are drawn coarser.
      const pixels = Math.min(Math.max(1, Math.round(width * ratio)), 16000);
      const height = canvas.clientHeight || 40;
      canvas.width = pixels;
      canvas.height = Math.round(height * ratio);
      const context = canvas.getContext("2d");
      if (!context) return;
      context.clearRect(0, 0, canvas.width, canvas.height);
      if (!data || data.peaks.length === 0) return;
      const from = item.clip.start * data.rate;
      const span = item.duration * data.rate;
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
    return { update: draw };
  }
</script>

<div class="timeline">
  <div class="labels">
    <div class="label ruler-label" style:height="{RULER_HEIGHT}px">时间</div>
    <div class="label" style:height="{subtitleTrackHeight}px">
      <span class="dot subtitle-dot"></span>字幕
    </div>
    <div class="label" style:height="{VIDEO_TRACK_HEIGHT}px">
      <span class="dot video-dot"></span>视频
    </div>
  </div>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="viewport sidebar-scrollbar" bind:this={viewport} onwheel={wheel}>
    <div class="content" style:width="{contentWidth}px">
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
      <div class="track subtitle-track" style:height="{subtitleTrackHeight}px" onclick={clickEmpty}>
        {#if subtitles.length === 0}
          <div class="track-hint">暂无字幕，可在右侧使用 AI 字幕或手动添加</div>
        {/if}
        {#each subtitles as item, index (item.subtitle.id)}
          <div
            class="subtitle"
            class:selected={selectedSubtitleId === item.subtitle.id}
            class:active={time >= item.start && time < item.end}
            role="button"
            tabindex="-1"
            style:left="{item.start * pxPerSecond}px"
            style:width="{Math.max(2, (item.end - item.start) * pxPerSecond)}px"
            style:top="{4 + lanes[index] * LANE_HEIGHT}px"
            title={item.subtitle.text}
            onpointerdown={(event) => startSubtitleDrag(event, item)}
          >
            <span>{item.subtitle.text || "空字幕"}</span>
          </div>
        {/each}
      </div>

      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="track video-track"
        style:height="{VIDEO_TRACK_HEIGHT}px"
        onclick={clickEmpty}
        ondragover={dragOver}
        ondragleave={() => (dropIndex = null)}
        ondrop={drop}
      >
        {#if placed.length === 0}
          <div class="track-hint">从左侧素材库拖入或点击「＋」添加素材</div>
        {/if}
        {#each placed as item (item.clip.id)}
          {@const material = materials.get(item.clip.material_id)}
          {@const width = item.duration * pxPerSecond}
          {@const overlay = trimOverlay(item)}
          <div
            class="clip"
            class:selected={selectedClipId === item.clip.id}
            class:dragging={drag?.kind === "move" && drag.moved && drag.clipId === item.clip.id}
            class:missing={!material}
            role="button"
            tabindex="-1"
            style:left="{item.offset * pxPerSecond}px"
            style:width="{Math.max(3, width)}px"
            onpointerdown={(event) => startClipDrag(event, item)}
          >
            <div class="clip-head">
              {#if material?.coverUrl && width > 64}
                <img src={material.coverUrl} alt="" draggable="false" />
              {:else}
                <Film class="h-3 w-3 shrink-0 text-violet-300" />
              {/if}
              <span class="clip-name">{material?.name ?? "素材已删除"}</span>
              <span class="clip-length">{formatDuration(item.duration)}</span>
            </div>
            <canvas
              class="clip-wave"
              use:waveform={{ item, data: peaks.get(item.clip.material_id), width }}
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
                {overlay.label} {formatDuration(overlay.source)}
              </div>
            {/if}
          </div>
        {/each}
        {#if drag?.kind === "move" && drag.moved}
          <div class="insert-marker" style:left="{insertionX(drag.index, drag.clipId)}px"></div>
        {/if}
        {#if dropIndex !== null}
          <div class="insert-marker" style:left="{insertionX(dropIndex)}px"></div>
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
  .timeline {
    display: flex;
    border-top: 1px solid rgb(31 41 55 / 50%);
    background: #1c1c1e;
    user-select: none;
  }

  .labels {
    display: flex;
    width: 72px;
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
    border-bottom: 1px solid rgb(31 41 55 / 70%);
    padding: 0 12px;
  }

  .ruler-label {
    font-size: 10px;
    letter-spacing: 0.05em;
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 2px;
  }

  .subtitle-dot {
    background: #0a84ff;
  }

  .video-dot {
    background: #8b5cf6;
  }

  .viewport {
    position: relative;
    flex: 1;
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
    z-index: 10;
    display: flex;
    height: 24px;
    align-items: center;
    overflow: hidden;
    border: 1px solid rgb(10 132 255 / 50%);
    border-radius: 6px;
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

  .clip {
    position: absolute;
    top: 4px;
    bottom: 4px;
    overflow: visible;
    border: 1px solid rgb(139 92 246 / 50%);
    border-radius: 6px;
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
