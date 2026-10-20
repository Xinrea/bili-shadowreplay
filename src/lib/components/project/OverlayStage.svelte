<script lang="ts" module>
  import type { ElementChanges } from "../../cover-elements";
  import type { VideoPlacement } from "../../interface";

  export type OverlayChanges = ElementChanges | Partial<VideoPlacement>;
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import {
    clamp,
    dragChanges,
    ELEMENT_SPACE,
    elementSize,
    scaledSize,
    withinBox,
    type TextElement,
  } from "../../cover-elements";
  import type { PlacedOverlay } from "../../project-timeline";
  import { frameScheduler, measure, redrawWhenLoaded } from "../cover/render";
  import SelectionBox from "../cover/SelectionBox.svelte";
  import TextEditField from "../cover/TextEditField.svelte";
  import OverlayVideo from "./OverlayVideo.svelte";
  import { drawOverlays, type DrawnOverlay } from "./overlay-render";

  // The project's overlays over the preview, one layer per track so a higher
  // track covers a lower one whatever each holds, and the surface that selects,
  // moves, turns and scales them as the cover editor does.

  interface Props {
    /** Visible overlays, bottom track first. */
    placed: PlacedOverlay[];
    trackCount: number;
    time: number;
    playing: boolean;
    /** Width of the preview frame in CSS pixels. */
    width: number;
    selectedId: string | null;
    editingId: string | null;
    editSelectAll: boolean;
    materialUrls: Map<number, string>;
    stickerUrls: Map<string, string>;
    /** Width over height of each material seen so far. */
    aspects: Map<number, number>;
    volume: number;
    muted: boolean;
    onSelect: (id: string | null) => void;
    /** A click on the picture away from any overlay, with none selected. */
    onBackground: () => void;
    /** A drag of an overlay begins; its changes until it ends are one edit. */
    onDragStart: () => void;
    onChange: (id: string, changes: OverlayChanges) => void;
    onDragEnd: () => void;
    onDelete: (id: string) => void;
    onStartEditing: (id: string) => void;
    onEditInput: (content: string) => void;
    onEditDone: (byKey: boolean) => void;
    onNudge: (dx: number, dy: number) => void;
    onAspect: (materialId: number, aspect: number) => void;
    /** Drawn over the overlays and under the selection, like subtitles. */
    above?: Snippet;
  }

  let {
    placed,
    trackCount,
    time,
    playing,
    width,
    selectedId,
    editingId,
    editSelectAll,
    materialUrls,
    stickerUrls,
    aspects,
    volume,
    muted,
    onSelect,
    onBackground,
    onDragStart,
    onChange,
    onDragEnd,
    onDelete,
    onStartEditing,
    onEditInput,
    onEditDone,
    onNudge,
    onAspect,
    above,
  }: Props = $props();

  // Video overlays load this long before they come in.
  const PRELOAD = 3;
  const DEFAULT_ASPECT = 16 / 9;

  let surface: HTMLDivElement = $state();
  let editField: ReturnType<typeof TextEditField> | undefined = $state();
  let canvases: HTMLCanvasElement[] = $state([]);
  let guides = $state({ x: false, y: false });

  let unit = $derived(width / ELEMENT_SPACE.width);
  let active = $derived(placed.filter((p) => time >= p.start && time < p.end));
  let activeKey = $derived(active.map((p) => p.item.id).join());
  let videos = $derived(
    placed.filter((p) => p.item.kind === "video" && time >= p.start - PRELOAD && time < p.end + 0.5),
  );
  let selected = $derived(active.find((p) => p.item.id === selectedId) ?? null);
  let editing = $derived(placed.find((p) => p.item.id === editingId && p.item.kind === "text") ?? null);

  const aspectOf = (materialId: number) => aspects.get(materialId) ?? DEFAULT_ASPECT;

  /** What the text being edited reads now, or null without one. */
  export function editedText(): string | null {
    return editField?.content() ?? null;
  }

  /** The overlay's box on the frame, before it is turned. */
  function boxOf(p: PlacedOverlay) {
    const { x, y, rotation } = p.item.element;
    if (p.item.kind === "video") {
      const { width } = p.item.element;
      return { x, y, rotation, width, height: width / aspectOf(p.item.material_id) };
    }
    return { x, y, rotation, ...elementSize(p.item.element, measure) };
  }

  // Each track's texts and stickers are drawn again only when what shows changes.
  $effect(() => {
    activeKey;
    placed;
    width;
    stickerUrls;
    editingId;
    canvases.length;
    scheduleRedraw();
  });

  const { schedule: scheduleRedraw, cancel: cancelRedraw } = frameScheduler(redraw);

  function redraw() {
    const ratio = window.devicePixelRatio || 1;
    const pending: Promise<unknown>[] = [];
    for (let track = 0; track < trackCount; track++) {
      const canvas = canvases[track];
      if (!canvas) continue;
      const pixels = Math.max(1, Math.round(width * ratio));
      const rows = Math.max(1, Math.round((pixels * ELEMENT_SPACE.height) / ELEMENT_SPACE.width));
      if (canvas.width !== pixels) canvas.width = pixels;
      if (canvas.height !== rows) canvas.height = rows;
      const items = active
        .filter((p) => p.track === track && p.item.kind !== "video")
        .map((p) => p.item as DrawnOverlay);
      pending.push(...drawOverlays(canvas.getContext("2d"), items, (ref) => stickerUrls.get(ref), editingId));
    }
    redrawWhenLoaded(pending, scheduleRedraw);
  }

  $effect(() => cancelRedraw);

  // Pointer interaction

  interface Drag {
    mode: "move" | "rotate" | "scale";
    id: string;
    start: { x: number; y: number };
    /** The element as the drag began. */
    origin: PlacedOverlay["item"]["element"];
    kind: PlacedOverlay["item"]["kind"];
    moved: boolean;
  }

  let drag: Drag | null = null;

  /** The point of the 1920×1080 frame under the pointer. */
  function toFrame(event: MouseEvent) {
    const rect = surface.getBoundingClientRect();
    return {
      x: ((event.clientX - rect.left) / rect.width) * ELEMENT_SPACE.width,
      y: ((event.clientY - rect.top) / rect.height) * ELEMENT_SPACE.height,
    };
  }

  /** The topmost overlay under the pointer: on the highest track, the last there. */
  function hitTest(event: MouseEvent): PlacedOverlay | null {
    const point = toFrame(event);
    const slop = 4 / unit;
    for (let index = active.length - 1; index >= 0; index--) {
      if (withinBox(boxOf(active[index]), point.x, point.y, slop)) return active[index];
    }
    return null;
  }

  function beginDrag(event: PointerEvent, mode: Drag["mode"], p: PlacedOverlay) {
    event.stopPropagation();
    if (event.button !== 0) return;
    surface.setPointerCapture(event.pointerId);
    drag = {
      mode,
      id: p.item.id,
      start: toFrame(event),
      origin: { ...p.item.element },
      kind: p.item.kind,
      moved: false,
    };
    onDragStart();
  }

  function handlePointerDown(event: PointerEvent) {
    if (event.button !== 0) return;
    // Not left to the field's blur, which a window without focus never gets.
    if (editingId) onEditDone(false);
    surface.focus({ preventScroll: true });
    const hit = hitTest(event);
    if (!hit) {
      if (selectedId) onSelect(null);
      else onBackground();
      return;
    }
    onSelect(hit.item.id);
    beginDrag(event, "move", hit);
  }

  function handlePointerMove(event: PointerEvent) {
    if (!drag) {
      surface.style.cursor = hitTest(event) ? "move" : "";
      return;
    }
    const point = toFrame(event);
    const origin = drag.origin;
    const update = dragChanges(drag.mode, origin, drag.start, point, unit, {
      moved: drag.moved,
      snapStep: event.shiftKey ? 15 : undefined,
    });
    if (!update) return;
    let changes: OverlayChanges;
    if ("scale" in update) {
      changes =
        drag.kind === "video"
          ? { width: clamp(Math.round((origin as VideoPlacement).width * update.scale), 64, 3840) }
          : scaledSize(origin as TextElement, update.scale);
    } else {
      changes = update.changes;
      if ("guides" in update) guides = update.guides;
    }
    drag.moved = true;
    onChange(drag.id, changes);
  }

  function handlePointerUp() {
    if (!drag) return;
    drag = null;
    guides = { x: false, y: false };
    surface.style.cursor = "";
    onDragEnd();
  }

  function handleDoubleClick(event: MouseEvent) {
    const hit = hitTest(event);
    if (hit?.item.kind === "text") onStartEditing(hit.item.id);
  }

  function handleKeydown(event: KeyboardEvent) {
    // Arrow keys nudge the selected overlay here, and seek elsewhere.
    if (!selected || editingId || event.altKey || event.metaKey || event.ctrlKey) return;
    const step = event.shiftKey ? 20 : 2;
    const moves: Record<string, [number, number]> = {
      ArrowLeft: [-step, 0],
      ArrowRight: [step, 0],
      ArrowUp: [0, -step],
      ArrowDown: [0, step],
    };
    const move = moves[event.key];
    if (!move) return;
    event.preventDefault();
    event.stopPropagation();
    onNudge(...move);
  }
</script>

<svelte:window onblur={handlePointerUp} />

<div class="stage">
  {#each Array.from({ length: trackCount }, (_, track) => track) as track (track)}
    <div class="layer" style:z-index={track + 1}>
      {#each videos.filter((p) => p.track === track) as p (p.item.id)}
        {@const item = p.item}
        {#if item.kind === "video" && materialUrls.has(item.material_id)}
          <OverlayVideo
            placed={p}
            url={materialUrls.get(item.material_id)}
            {time}
            {playing}
            {unit}
            aspect={aspectOf(item.material_id)}
            {volume}
            {muted}
            onAspect={(aspect) => onAspect(item.material_id, aspect)}
          />
        {/if}
      {/each}
      <canvas bind:this={canvases[track]} class="layer-canvas"></canvas>
    </div>
  {/each}

  {#if above}
    <div class="layer" style:z-index={trackCount + 1}>
      {@render above()}
    </div>
  {/if}

  <!-- Focusable so the arrow keys reach the selected overlay. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    bind:this={surface}
    class="surface"
    style:z-index={trackCount + 2}
    role="application"
    aria-label="预览画面：点击选中文字、贴纸或画中画并拖动，双击文字可直接修改"
    tabindex="0"
    onpointerdown={handlePointerDown}
    onpointermove={handlePointerMove}
    onpointerup={handlePointerUp}
    onpointercancel={handlePointerUp}
    ondblclick={handleDoubleClick}
    onkeydown={handleKeydown}
  >
    {#if guides.x}
      <div class="guide vertical"></div>
    {/if}
    {#if guides.y}
      <div class="guide horizontal"></div>
    {/if}

    {#if selected && !editing}
      {@const box = boxOf(selected)}
      <SelectionBox
        left={box.x * unit}
        top={box.y * unit}
        width={box.width * unit}
        height={box.height * unit}
        rotation={box.rotation}
        onDelete={() => onDelete(selected.item.id)}
        onRotate={(event) => beginDrag(event, "rotate", selected)}
        onScale={(event) => beginDrag(event, "scale", selected)}
      />
    {/if}

    {#if editing && editing.item.kind === "text"}
      {#key editing.item.id}
        <TextEditField
          bind:this={editField}
          text={editing.item.element}
          {unit}
          left={editing.item.element.x * unit}
          top={editing.item.element.y * unit}
          selectAll={editSelectAll}
          onInput={onEditInput}
          onDone={(byKey) => {
            onEditDone(byKey);
            // So the arrow keys go on nudging it.
            if (byKey) surface.focus({ preventScroll: true });
          }}
        />
      {/key}
    {/if}
  </div>
</div>

<style>
  .stage {
    position: absolute;
    inset: 0;
    overflow: hidden;
  }

  .layer {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }

  .layer-canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }

  .surface {
    position: absolute;
    inset: 0;
    touch-action: none;
    outline: none;
  }

  .guide {
    position: absolute;
    pointer-events: none;
    background: #ff2d7a;
  }

  .guide.vertical {
    top: 0;
    bottom: 0;
    left: 50%;
    width: 1px;
    transform: translateX(-50%);
  }

  .guide.horizontal {
    left: 0;
    right: 0;
    top: 50%;
    height: 1px;
    transform: translateY(-50%);
  }
</style>
