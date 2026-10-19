<script lang="ts">
  import {
    AlignCenter,
    AlignLeft,
    AlignRight,
    ArrowDown,
    ArrowUp,
    Bold,
    Copy,
    Film,
    FlipHorizontal2,
    ImageUp,
    Loader2,
    MoveDiagonal2,
    Redo2,
    RotateCw,
    Smile,
    Trash2,
    Type,
    Undo2,
    X,
  } from "lucide-svelte";
  import { onMount, tick, untrack } from "svelte";
  import {
    clamp,
    COVER_HEIGHT,
    COVER_WIDTH,
    elementAt,
    elementSize,
    fontFamily,
    HOMEPAGE_CROP,
    layoutText,
    LINE_HEIGHT,
    newSticker,
    newText,
    nextId,
    reorder,
    scaledSize,
    snapRotation,
    snapToCenter,
    styleOf,
    TEXT_STYLES,
    type CoverElement,
    type StickerElement,
    type TextElement,
    type TextStyle,
  } from "../../cover-elements";
  import { invoke, log } from "../../invoker";
  import ColorPicker from "./ColorPicker.svelte";
  import CoverFontSelect from "./CoverFontSelect.svelte";
  import { drawBackground, drawElements, isCrossOrigin, loadSticker, measure, STICKER_GROUPS } from "./render";
  import StickerPanel from "./StickerPanel.svelte";
  import TextPanel from "./TextPanel.svelte";

  interface Props {
    /** `file` is the video to pick frames from; `value` its id when saving to it. */
    video?: { file?: string; value?: number } | null;
    /** The current cover, used as the background at first. */
    image?: string;
    /** Shown under the homepage preview. */
    title?: string;
    show?: boolean;
    onCoverUpdate?: (cover: { cover: string }) => void;
    /** Store the cover somewhere other than the video, such as an upload. */
    onSave?: (cover: string) => Promise<void>;
  }

  let { video = null, image = "", title = "", show = $bindable(false), onCoverUpdate, onSave }: Props = $props();

  // Elements, front last. Kept while the editor is hidden, so reopening
  // after saving edits them again over the same background.
  let elements: CoverElement[] = $state([]);
  let selectedId: number | null = $state(null);
  let editingId: number | null = $state(null);
  let panel: "text" | "sticker" = $state("text");
  let stickerGroup = $state(STICKER_GROUPS[0].name);
  let showCrop = $state(true);
  let saving = $state(false);
  let guides = $state({ x: false, y: false });
  let lastStyle: TextStyle = styleOf(TEXT_STYLES[0]);
  let lastFont = "";

  let selected = $derived(elements.find((element) => element.id === selectedId) ?? null);
  let selectedText = $derived(selected?.kind === "text" ? selected : null);
  let selectedSticker = $derived(selected?.kind === "sticker" ? selected : null);
  let selectedBox = $derived(selected ? elementSize(selected, measure) : null);
  let editing = $derived((elements.find((e) => e.id === editingId && e.kind === "text") as TextElement) ?? null);

  let canvas: HTMLCanvasElement = $state();
  let ctx: CanvasRenderingContext2D | null = null;
  let stageArea: HTMLDivElement = $state();
  let surface: HTMLDivElement = $state();
  let editField: HTMLDivElement = $state();
  let previewHome: HTMLCanvasElement = $state();
  let previewWide: HTMLCanvasElement = $state();
  let fileInput: HTMLInputElement = $state();
  let stageWidth = $state(0);
  let scale = $derived(stageWidth / COVER_WIDTH);
  let frameRequest: number | null = null;

  // The background, already fitted to the cover.
  const backgroundCanvas = document.createElement("canvas");
  backgroundCanvas.width = COVER_WIDTH;
  backgroundCanvas.height = COVER_HEIGHT;
  let hasBackground = $state(false);
  let backgroundKind: "cover" | "frame" | "upload" | null = $state(null);
  let originalCover = $state("");
  // Kept once loaded: saving can delete the file it came from.
  let originalImage: HTMLImageElement | null = null;
  // Bumped by each background change so a slower image load cannot win.
  let backgroundToken = 0;
  let followVideo = false;
  // Picked by the user rather than a frame shown while the cover loads.
  let backgroundChosen = false;

  let videoElement: HTMLVideoElement = $state();
  let opened = $state(false);
  let videoReady = $state(false);
  let duration = $state(0);
  let frameTime = $state(0);

  onMount(() => {
    ctx = canvas.getContext("2d");
    const observer = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      stageWidth = Math.floor(Math.max(0, Math.min(width, (height * COVER_WIDTH) / COVER_HEIGHT)));
    });
    observer.observe(stageArea);
    scheduleRedraw();
    return () => {
      observer.disconnect();
      if (frameRequest !== null) cancelAnimationFrame(frameRequest);
      clearTimeout(commitTimer);
    };
  });

  $effect(() => {
    $state.snapshot(elements);
    editingId;
    scheduleRedraw();
  });

  $effect(() => {
    if (!show) return;
    const cover = image;
    untrack(() => {
      opened = true;
      if (cover && !originalCover) {
        originalCover = cover;
        if (!backgroundChosen) void useImage(cover, "cover");
      }
    });
  });

  function scheduleRedraw() {
    if (frameRequest !== null) return;
    frameRequest = requestAnimationFrame(() => {
      frameRequest = null;
      redraw();
    });
  }

  function redraw() {
    if (!ctx) return;
    ctx.clearRect(0, 0, COVER_WIDTH, COVER_HEIGHT);
    if (hasBackground) {
      ctx.drawImage(backgroundCanvas, 0, 0);
    } else {
      ctx.fillStyle = "#2A2F3A";
      ctx.fillRect(0, 0, COVER_WIDTH, COVER_HEIGHT);
    }
    const pending = drawElements(ctx, elements, editingId);
    if (pending.length) {
      // Only redraw for stickers that arrived, or a failed one would loop.
      void Promise.all(pending.map((request) => request.then(() => true, () => false))).then(
        (loaded) => loaded.some(Boolean) && scheduleRedraw(),
      );
    }
    drawPreview(previewHome, HOMEPAGE_CROP);
    drawPreview(previewWide, { x: 0, y: 0, width: COVER_WIDTH, height: COVER_HEIGHT });
  }

  function drawPreview(target: HTMLCanvasElement | undefined, crop: typeof HOMEPAGE_CROP) {
    const preview = target?.getContext("2d");
    if (!preview) return;
    preview.imageSmoothingQuality = "high";
    preview.drawImage(canvas, crop.x, crop.y, crop.width, crop.height, 0, 0, target.width, target.height);
  }

  // Background

  function setBackground(source: CanvasImageSource, width: number, height: number, kind: typeof backgroundKind) {
    const context = backgroundCanvas.getContext("2d");
    context.clearRect(0, 0, COVER_WIDTH, COVER_HEIGHT);
    drawBackground(context, source, width, height);
    hasBackground = true;
    backgroundKind = kind;
    scheduleRedraw();
  }

  function loadImage(src: string): Promise<HTMLImageElement> {
    return new Promise((resolve, reject) => {
      const image = new Image();
      if (isCrossOrigin(src)) image.crossOrigin = "anonymous";
      image.onload = () => resolve(image);
      image.onerror = () => reject(new Error(`Failed to load ${src}`));
      image.src = src;
    });
  }

  // Awaited before saving, so a quick save does not miss the background.
  let backgroundLoad: Promise<void> = Promise.resolve();

  function useImage(src: string, kind: "cover" | "upload") {
    backgroundLoad = loadBackground(src, kind);
    return backgroundLoad;
  }

  async function loadBackground(src: string, kind: "cover" | "upload") {
    const token = ++backgroundToken;
    followVideo = false;
    try {
      const image = src === originalCover && originalImage ? originalImage : await loadImage(src);
      if (src === originalCover) originalImage = image;
      if (token === backgroundToken) setBackground(image, image.naturalWidth, image.naturalHeight, kind);
    } catch (error) {
      log.error("Failed to load the cover background:", String(error));
      if (token === backgroundToken && !hasBackground && videoReady) seekFrame(defaultFrameTime());
    }
  }

  async function handleUpload(event: Event & { currentTarget: HTMLInputElement }) {
    const file = event.currentTarget.files?.[0];
    event.currentTarget.value = "";
    if (!file) return;
    backgroundChosen = true;
    const url = URL.createObjectURL(file);
    await useImage(url, "upload");
    URL.revokeObjectURL(url);
  }

  // Past the black frames most recordings start with.
  const defaultFrameTime = () => Math.min(duration * 0.1, 30);

  function handleVideoLoaded() {
    videoReady = true;
    duration = videoElement.duration;
    if (!hasBackground && !originalCover) seekFrame(defaultFrameTime());
  }

  function seekFrame(time: number) {
    if (!videoElement) return;
    ++backgroundToken;
    followVideo = true;
    frameTime = time;
    videoElement.currentTime = time;
  }

  function handleSeeked() {
    if (!followVideo || !videoElement.videoWidth) return;
    try {
      setBackground(videoElement, videoElement.videoWidth, videoElement.videoHeight, "frame");
    } catch (error) {
      log.error("Failed to capture video frame:", String(error));
    }
  }

  function formatTime(seconds: number): string {
    const mins = Math.floor(seconds / 60);
    const secs = Math.floor(seconds % 60);
    return `${mins}:${secs.toString().padStart(2, "0")}`;
  }

  // History

  let history = [JSON.stringify([])];
  let historyIndex = $state(0);
  let historyLength = $state(1);
  let commitTimer: ReturnType<typeof setTimeout> | undefined;

  function commit() {
    clearTimeout(commitTimer);
    commitTimer = undefined;
    const snapshot = JSON.stringify($state.snapshot(elements));
    if (snapshot === history[historyIndex]) return;
    history = [...history.slice(0, historyIndex + 1), snapshot].slice(-100);
    historyIndex = history.length - 1;
    historyLength = history.length;
  }

  /** Commit once a run of small changes, like dragging a color, settles. */
  function commitSoon() {
    clearTimeout(commitTimer);
    commitTimer = setTimeout(commit, 400);
  }

  function restore(index: number) {
    if (commitTimer) commit();
    if (index < 0 || index >= history.length) return;
    historyIndex = index;
    elements = JSON.parse(history[index]);
    if (!elements.some((element) => element.id === selectedId)) selectedId = null;
  }

  const undo = () => restore(historyIndex - 1);
  const redo = () => restore(historyIndex + 1);

  // Elements

  function addText(style: TextStyle = lastStyle, font = lastFont) {
    const text = newText(elements, style, font);
    elements.push(text);
    // Committed when its first edit finishes.
    void startEditing(text.id, true);
  }

  async function addSticker(src: string, size?: number) {
    let image: HTMLImageElement;
    try {
      image = await loadSticker(src);
    } catch {
      alert("贴纸加载失败，可能已被删除");
      return;
    }
    const element = newSticker(elements, src, image.naturalWidth / image.naturalHeight || 1, size);
    elements.push(element);
    selectedId = element.id;
    commit();
  }

  function pickStyle(style: TextStyle) {
    lastStyle = styleOf(style);
    if (!selectedText) return addText(style);
    Object.assign(selectedText, styleOf(style));
    commit();
  }

  function pickFont(family: string) {
    lastFont = family;
    if (!selectedText) return addText(lastStyle, family);
    selectedText.font = family;
    commit();
  }

  function update(changes: Partial<TextElement> | Partial<StickerElement>) {
    if (!selected) return;
    Object.assign(selected, changes);
    commitSoon();
  }

  function remove(id: number) {
    elements = elements.filter((element) => element.id !== id);
    if (selectedId === id) selectedId = null;
    commit();
  }

  function duplicate(element: CoverElement) {
    const copy = {
      ...$state.snapshot(element),
      id: nextId(elements),
      x: clamp(element.x + 40, 0, COVER_WIDTH),
      y: clamp(element.y + 40, 0, COVER_HEIGHT),
    } as CoverElement;
    elements.push(copy);
    selectedId = copy.id;
    commit();
  }

  function shift(element: CoverElement, step: 1 | -1) {
    const index = elements.findIndex((e) => e.id === element.id);
    elements = reorder(elements, index, index + step);
    commit();
  }

  const PANELS = [
    { id: "text", label: "文字", icon: Type },
    { id: "sticker", label: "贴纸", icon: Smile },
  ] as const;

  const ALIGNMENTS = [
    { value: "left", label: "左对齐", icon: AlignLeft },
    { value: "center", label: "居中", icon: AlignCenter },
    { value: "right", label: "右对齐", icon: AlignRight },
  ] as const;

  // Text editing on the canvas

  async function startEditing(id: number, selectAll = false) {
    finishEditing();
    const element = elements.find((e) => e.id === id);
    if (element?.kind !== "text") return;
    selectedId = id;
    editingId = id;
    await tick();
    if (!editField) return;
    try {
      editField.contentEditable = "plaintext-only";
    } catch {
      // Older engines reject it; innerText still reads plain text.
      editField.contentEditable = "true";
    }
    editField.textContent = element.content;
    editField.focus();
    const range = document.createRange();
    range.selectNodeContents(editField);
    if (!selectAll) range.collapse(false);
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
  }

  function finishEditing() {
    const id = editingId;
    if (id === null) return;
    const field = editField;
    editingId = null;
    const element = elements.find((e) => e.id === id);
    if (element?.kind === "text" && field) {
      const content = field.innerText.replace(/\r/g, "").replace(/\n$/, "");
      if (content.trim()) {
        element.content = content;
      } else {
        elements = elements.filter((e) => e.id !== id);
        selectedId = null;
      }
    }
    commit();
  }

  function handleEditKeydown(event: KeyboardEvent) {
    if (event.isComposing) return;
    if (event.key === "Escape" || (event.key === "Enter" && (event.metaKey || event.ctrlKey))) {
      // Leave editing, not the editor.
      event.preventDefault();
      event.stopPropagation();
      finishEditing();
      surface.focus({ preventScroll: true });
    }
  }

  // Pointer interaction

  interface Drag {
    mode: "move" | "rotate" | "scale";
    id: number;
    start: { x: number; y: number };
    origin: CoverElement;
    moved: boolean;
  }

  let drag: Drag | null = null;

  function toCover(event: MouseEvent) {
    const rect = surface.getBoundingClientRect();
    return {
      x: ((event.clientX - rect.left) / rect.width) * COVER_WIDTH,
      y: ((event.clientY - rect.top) / rect.height) * COVER_HEIGHT,
    };
  }

  function hitTest(event: MouseEvent) {
    const point = toCover(event);
    return elementAt(elements, point.x, point.y, measure, 4 / scale);
  }

  function beginDrag(event: PointerEvent, mode: Drag["mode"], element: CoverElement) {
    event.stopPropagation();
    if (event.button !== 0) return;
    surface.setPointerCapture(event.pointerId);
    drag = { mode, id: element.id, start: toCover(event), origin: $state.snapshot(element) as CoverElement, moved: false };
  }

  function handlePointerDown(event: PointerEvent) {
    if (event.button !== 0) return;
    finishEditing();
    surface.focus({ preventScroll: true });
    const hit = hitTest(event);
    selectedId = hit?.id ?? null;
    if (hit) beginDrag(event, "move", hit);
  }

  function handlePointerMove(event: PointerEvent) {
    if (!drag) {
      surface.style.cursor = hitTest(event) ? "move" : "";
      return;
    }
    const element = elements.find((e) => e.id === drag.id);
    if (!element) return;
    const point = toCover(event);
    const origin = drag.origin;
    if (drag.mode === "move") {
      const dx = point.x - drag.start.x;
      const dy = point.y - drag.start.y;
      // A click selects without nudging.
      if (!drag.moved && Math.hypot(dx, dy) * scale < 3) return;
      const snapped = snapToCenter(
        clamp(origin.x + dx, 0, COVER_WIDTH),
        clamp(origin.y + dy, 0, COVER_HEIGHT),
        8 / scale,
      );
      element.x = Math.round(snapped.x);
      element.y = Math.round(snapped.y);
      guides = { x: snapped.snapX, y: snapped.snapY };
    } else if (drag.mode === "rotate") {
      const from = Math.atan2(drag.start.y - origin.y, drag.start.x - origin.x);
      const to = Math.atan2(point.y - origin.y, point.x - origin.x);
      element.rotation = snapRotation(origin.rotation + ((to - from) * 180) / Math.PI, event.shiftKey ? 15 : undefined);
    } else {
      const ratio =
        Math.hypot(point.x - origin.x, point.y - origin.y) /
        Math.max(1, Math.hypot(drag.start.x - origin.x, drag.start.y - origin.y));
      Object.assign(element, scaledSize(origin, ratio));
    }
    drag.moved = true;
  }

  function handlePointerUp() {
    if (!drag) return;
    const moved = drag.moved;
    drag = null;
    guides = { x: false, y: false };
    if (moved) commit();
  }

  function handleDoubleClick(event: MouseEvent) {
    const hit = hitTest(event);
    if (hit?.kind === "text") void startEditing(hit.id, true);
  }

  function handleKeydown(event: KeyboardEvent) {
    if (!show || saving || event.defaultPrevented) return;
    const target = event.target as HTMLElement | null;
    const typing = Boolean(target?.closest?.("input, textarea, select, [contenteditable]"));
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopImmediatePropagation();
      if (typing) target.blur();
      else if (selectedId !== null) selectedId = null;
      else handleClose();
      return;
    }
    if (typing || editingId !== null) return;
    const mod = event.metaKey || event.ctrlKey;
    const key = event.key.toLowerCase();
    if (mod && (key === "z" || key === "y")) {
      event.preventDefault();
      if (key === "y" || event.shiftKey) redo();
      else undo();
      return;
    }
    if (!selected) return;
    if (key === "delete" || key === "backspace") {
      event.preventDefault();
      remove(selected.id);
    } else if (mod && key === "d") {
      event.preventDefault();
      duplicate(selected);
    } else if (key === "enter" && selected.kind === "text") {
      event.preventDefault();
      void startEditing(selected.id, true);
    } else if (key.startsWith("arrow")) {
      event.preventDefault();
      const step = event.shiftKey ? 20 : 2;
      const dx = key === "arrowleft" ? -step : key === "arrowright" ? step : 0;
      const dy = key === "arrowup" ? -step : key === "arrowdown" ? step : 0;
      update({ x: clamp(selected.x + dx, 0, COVER_WIDTH), y: clamp(selected.y + dy, 0, COVER_HEIGHT) });
    }
  }

  function close() {
    finishEditing();
    selectedId = null;
    show = false;
  }

  // A save in flight writes the cover anyway, so it cannot be cancelled.
  function handleClose() {
    if (!saving) close();
  }

  async function handleSave() {
    if (saving) return;
    finishEditing();
    saving = true;
    try {
      await backgroundLoad;
      const loads = await Promise.allSettled(
        elements.flatMap((element) => (element.kind === "sticker" ? [loadSticker(element.src)] : [])),
      );
      if (loads.some((load) => load.status === "rejected")) {
        alert("有贴纸没有加载出来，请删除后再保存封面");
        return;
      }
      redraw();
      const cover = canvas.toDataURL("image/jpeg", 0.92);
      if (onSave) {
        await onSave(cover);
      } else {
        await invoke("update_video_cover", { id: video.value, cover });
      }
      onCoverUpdate?.({ cover });
      close();
    } catch (e) {
      alert("更新封面失败: " + e);
    } finally {
      saving = false;
    }
  }
</script>

<svelte:window
  onkeydown={handleKeydown}
  onblur={handlePointerUp}
/>

<!-- Kept mounted while hidden so the canvas and video stay ready. -->
<div
  class="ui-modal-backdrop z-[1100] transition-opacity duration-200"
  class:opacity-0={!show}
  class:pointer-events-none={!show}
  inert={!show}
  data-hotkey-block
>
  <div
    class="ui-modal h-[min(820px,92vh)] max-h-[92vh] w-[1320px] max-w-full transition-transform duration-200"
    class:scale-95={!show}
    role="dialog"
    aria-modal="true"
    aria-labelledby="cover-editor-title"
  >
    <div class="ui-modal-header">
      <h3 id="cover-editor-title">封面制作</h3>
      <button
        type="button"
        class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost -mr-1.5"
        aria-label="关闭"
        disabled={saving}
        onclick={handleClose}
      >
        <X class="h-4 w-4" />
      </button>
    </div>

    <!-- Changes after the canvas is captured would be lost. -->
    <div class="flex min-h-0 flex-1" inert={saving}>
      <!-- Element kinds -->
      <nav class="flex w-[60px] shrink-0 flex-col items-center gap-1 border-r border-line bg-chrome/50 py-3" aria-label="元素">
        {#each PANELS as item (item.id)}
          <button
            type="button"
            class="flex w-12 flex-col items-center gap-1 rounded-[8px] py-2 text-[11px] transition-colors {panel === item.id
              ? 'bg-accent-soft font-medium text-accent-fg'
              : 'text-fg-muted hover:bg-control-hover hover:text-fg'}"
            aria-pressed={panel === item.id}
            onclick={() => (panel = item.id)}
          >
            <item.icon class="h-5 w-5" />
            {item.label}
          </button>
        {/each}
      </nav>

      <div class="custom-scrollbar-light w-[252px] shrink-0 overflow-y-auto border-r border-line p-3">
        <!-- Both stay mounted so an import keeps running while the other is shown. -->
        <div class:hidden={panel !== "text"}>
          <TextPanel selected={selectedText} onAdd={() => addText()} onStyle={pickStyle} onFont={pickFont} />
        </div>
        <div class="min-h-full" class:hidden={panel !== "sticker"}>
          <StickerPanel active={show && panel === "sticker"} bind:group={stickerGroup} onPick={addSticker} />
        </div>
      </div>

      <!-- Canvas -->
      <div class="flex min-w-0 flex-1 flex-col bg-sunken">
        <div class="flex min-h-[46px] shrink-0 flex-wrap items-center gap-1 border-b border-line bg-surface px-3 py-2">
          {#if selectedText}
            <CoverFontSelect
              value={selectedText.font}
              onSelect={(font) => {
                lastFont = font;
                selectedText.font = font;
                commit();
              }}
            />
            <input
              type="number"
              min="12"
              max="800"
              value={selectedText.fontSize}
              class="ui-input h-[26px] w-[66px] pr-1 tabular-nums"
              title="字号"
              aria-label="字号"
              onchange={(event) => {
                const size = Math.round(Number(event.currentTarget.value));
                update({ fontSize: clamp(size || selectedText.fontSize, 12, 800) });
                event.currentTarget.value = String(selectedText.fontSize);
              }}
            />
            <span class="toolbar-separator"></span>
            <button
              type="button"
              class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost {selectedText.bold ? 'ui-btn-active' : ''}"
              title="加粗"
              aria-label="加粗"
              aria-pressed={selectedText.bold}
              onclick={() => update({ bold: !selectedText.bold })}
            >
              <Bold class="h-3.5 w-3.5" />
            </button>
            <ColorPicker kind="fill" label="文字颜色" value={selectedText.fill} onchange={(fill) => update({ fill })} />
            <ColorPicker
              kind="stroke"
              label="描边"
              allowNone
              value={selectedText.stroke}
              onchange={(stroke) => update({ stroke })}
            >
              <label class="flex items-center gap-2 text-[11px] text-fg-muted">
                粗细
                <input
                  type="range"
                  min="0.02"
                  max="0.25"
                  step="0.01"
                  value={selectedText.strokeWidth}
                  disabled={!selectedText.stroke}
                  class="min-w-0 flex-1"
                  oninput={(event) => update({ strokeWidth: Number(event.currentTarget.value) })}
                />
              </label>
            </ColorPicker>
            <ColorPicker
              kind="background"
              label="背景色"
              allowNone
              value={selectedText.background}
              onchange={(background) => update({ background })}
            />
            <button
              type="button"
              class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost {selectedText.shadow ? 'ui-btn-active' : ''}"
              title="阴影"
              aria-label="阴影"
              aria-pressed={selectedText.shadow}
              onclick={() => update({ shadow: !selectedText.shadow })}
            >
              <span class="shadow-glyph text-[13px] font-bold leading-none">A</span>
            </button>
            {@const alignment = ALIGNMENTS.find((a) => a.value === selectedText.align)}
            {@const multiline = selectedText.content.includes("\n")}
            <button
              type="button"
              class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost"
              title={multiline ? `对齐：${alignment.label}` : "多行文字可设置对齐"}
              aria-label="对齐方式"
              disabled={!multiline}
              onclick={() =>
                update({ align: ALIGNMENTS[(ALIGNMENTS.indexOf(alignment) + 1) % ALIGNMENTS.length].value })}
            >
              <alignment.icon class="h-3.5 w-3.5" />
            </button>
          {:else if selectedSticker}
            <button
              type="button"
              class="ui-btn ui-btn-sm {selectedSticker.flip ? 'ui-btn-active' : ''}"
              aria-pressed={selectedSticker.flip}
              onclick={() => update({ flip: !selectedSticker.flip })}
            >
              <FlipHorizontal2 class="h-3.5 w-3.5" />
              水平翻转
            </button>
          {:else}
            <span class="text-[12px] text-fg-muted">从左侧添加文字或贴纸，双击文字可直接修改。</span>
          {/if}

          <div class="ml-auto flex items-center gap-1">
            {#if selected}
              {@const index = elements.indexOf(selected)}
              <button
                type="button"
                class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost"
                title="上移一层"
                aria-label="上移一层"
                disabled={index === elements.length - 1}
                onclick={() => shift(selected, 1)}
              >
                <ArrowUp class="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost"
                title="下移一层"
                aria-label="下移一层"
                disabled={index === 0}
                onclick={() => shift(selected, -1)}
              >
                <ArrowDown class="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost"
                title="复制 (⌘/Ctrl+D)"
                aria-label="复制"
                onclick={() => duplicate(selected)}
              >
                <Copy class="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost hover:text-danger-fg"
                title="删除 (Delete)"
                aria-label="删除"
                onclick={() => remove(selected.id)}
              >
                <Trash2 class="h-3.5 w-3.5" />
              </button>
              <span class="toolbar-separator"></span>
            {/if}
            <button
              type="button"
              class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost"
              title="撤销 (⌘/Ctrl+Z)"
              aria-label="撤销"
              disabled={historyIndex === 0}
              onclick={undo}
            >
              <Undo2 class="h-3.5 w-3.5" />
            </button>
            <button
              type="button"
              class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost"
              title="重做 (⇧⌘Z / Ctrl+Y)"
              aria-label="重做"
              disabled={historyIndex >= historyLength - 1}
              onclick={redo}
            >
              <Redo2 class="h-3.5 w-3.5" />
            </button>
          </div>
        </div>

        <div bind:this={stageArea} class="relative m-5 flex min-h-0 flex-1 items-center justify-center">
          <div
            class="relative overflow-hidden rounded-[4px] shadow-lg"
            style:width="{stageWidth}px"
            style:height="{(stageWidth * COVER_HEIGHT) / COVER_WIDTH}px"
          >
            <canvas bind:this={canvas} width={COVER_WIDTH} height={COVER_HEIGHT} class="absolute inset-0 h-full w-full"
            ></canvas>
            <!-- Focusable so Delete and the arrow keys reach the selected element. -->
            <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
            <div
              bind:this={surface}
              class="absolute inset-0 touch-none outline-none"
              role="application"
              aria-label="封面画布：拖动移动元素，双击编辑文字"
              tabindex="0"
              onpointerdown={handlePointerDown}
              onpointermove={handlePointerMove}
              onpointerup={handlePointerUp}
              onpointercancel={handlePointerUp}
              ondblclick={handleDoubleClick}
            >
              {#if showCrop}
                <div class="pointer-events-none absolute inset-0">
                  <div class="crop-shade left-0" style:width="{HOMEPAGE_CROP.x * scale}px"></div>
                  <div class="crop-shade right-0" style:width="{HOMEPAGE_CROP.x * scale}px"></div>
                  <div
                    class="absolute inset-y-0 border-x border-dashed border-white/80"
                    style:left="{HOMEPAGE_CROP.x * scale}px"
                    style:width="{HOMEPAGE_CROP.width * scale}px"
                  >
                    <span class="absolute left-1.5 top-1.5 rounded bg-black/50 px-1.5 py-px text-[10px] text-white/90">4:3</span>
                  </div>
                </div>
              {/if}
              {#if guides.x}
                <div class="pointer-events-none absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-[#FF2D7A]"></div>
              {/if}
              {#if guides.y}
                <div class="pointer-events-none absolute inset-x-0 top-1/2 h-px -translate-y-1/2 bg-[#FF2D7A]"></div>
              {/if}

              {#if selected && selectedBox && editingId === null}
                <div
                  class="pointer-events-none absolute border-[1.5px] border-accent"
                  style:left="{selected.x * scale}px"
                  style:top="{selected.y * scale}px"
                  style:width="{selectedBox.width * scale}px"
                  style:height="{selectedBox.height * scale}px"
                  style:transform="translate(-50%, -50%) rotate({selected.rotation}deg)"
                >
                  <button
                    type="button"
                    class="handle -left-2.5 -top-2.5 hover:text-danger-fg"
                    title="删除"
                    aria-label="删除"
                    onpointerdown={(event) => event.stopPropagation()}
                    onclick={() => remove(selected.id)}
                  >
                    <X class="h-3 w-3" />
                  </button>
                  <button
                    type="button"
                    class="handle -right-2.5 -top-2.5 cursor-grab"
                    title="旋转，按住 Shift 以 15° 为步长"
                    aria-label="旋转"
                    onpointerdown={(event) => beginDrag(event, "rotate", selected)}
                  >
                    <RotateCw class="h-3 w-3" />
                  </button>
                  <button
                    type="button"
                    class="handle -bottom-2.5 -left-2.5 cursor-nesw-resize"
                    title="缩放"
                    aria-label="缩放"
                    onpointerdown={(event) => beginDrag(event, "scale", selected)}
                  >
                    <MoveDiagonal2 class="h-3 w-3 rotate-90" />
                  </button>
                  <button
                    type="button"
                    class="handle -bottom-2.5 -right-2.5 cursor-nwse-resize"
                    title="缩放"
                    aria-label="缩放"
                    onpointerdown={(event) => beginDrag(event, "scale", selected)}
                  >
                    <MoveDiagonal2 class="h-3 w-3" />
                  </button>
                </div>
              {/if}

              {#if editing}
                {@const layout = layoutText(editing, measure)}
                <!-- Typing and selecting stay inside the field. -->
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <div
                  class="absolute"
                  style:left="{editing.x * scale}px"
                  style:top="{editing.y * scale}px"
                  style:transform="translate(-50%, -50%) rotate({editing.rotation}deg)"
                  onpointerdown={(event) => event.stopPropagation()}
                  ondblclick={(event) => event.stopPropagation()}
                >
                  <div
                    bind:this={editField}
                    contenteditable="plaintext-only"
                    role="textbox"
                    tabindex="0"
                    aria-multiline="true"
                    aria-label="文字内容"
                    spellcheck="false"
                    class="edit-field"
                    style="-webkit-text-stroke: {editing.stroke
                      ? `${editing.strokeWidth * editing.fontSize * 2 * scale}px ${editing.stroke}`
                      : '0'}"
                    style:font-family={fontFamily(editing.font)}
                    style:font-weight={editing.bold ? "bold" : "normal"}
                    style:font-size="{editing.fontSize * scale}px"
                    style:line-height={LINE_HEIGHT}
                    style:color={editing.fill}
                    style:text-align={editing.align}
                    style:padding="{layout.padY * scale}px {layout.padX * scale}px"
                    style:background={editing.background ?? "transparent"}
                    style:border-radius="{editing.fontSize * 0.18 * scale}px"
                    onkeydown={handleEditKeydown}
                    onblur={finishEditing}
                  ></div>
                </div>
              {/if}
            </div>
          </div>
        </div>

        <!-- Background -->
        <div class="flex h-12 shrink-0 items-center gap-3 border-t border-line bg-surface px-3">
          <span class="flex shrink-0 items-center gap-1.5 text-[12px] font-medium text-fg-secondary">
            <Film class="h-3.5 w-3.5 text-fg-muted" />
            视频帧
          </span>
          {#if video?.file}
            <input
              type="range"
              min="0"
              max={duration || 0}
              step="0.1"
              value={frameTime}
              class="min-w-0 flex-1"
              aria-label="视频帧位置"
              disabled={!videoReady}
              oninput={(event) => {
                backgroundChosen = true;
                seekFrame(Number(event.currentTarget.value));
              }}
            />
            <span class="shrink-0 text-[11px] tabular-nums text-fg-muted">
              {formatTime(frameTime)} / {formatTime(duration)}
            </span>
          {:else}
            <span class="min-w-0 flex-1 text-[11px] text-fg-faint">没有可选帧的视频</span>
          {/if}
          <div class="flex shrink-0 items-center gap-1.5">
            {#if originalCover}
              <button
                type="button"
                class="ui-btn ui-btn-sm {backgroundKind === 'cover' ? 'ui-btn-active' : ''}"
                onclick={() => {
                  backgroundChosen = true;
                  void useImage(originalCover, "cover");
                }}
              >
                原封面
              </button>
            {/if}
            <button
              type="button"
              class="ui-btn ui-btn-sm {backgroundKind === 'upload' ? 'ui-btn-active' : ''}"
              onclick={() => fileInput.click()}
            >
              <ImageUp class="h-3.5 w-3.5" />
              上传图片
            </button>
            <input bind:this={fileInput} type="file" accept="image/*" class="hidden" onchange={handleUpload} />
          </div>
          <!-- svelte-ignore a11y_media_has_caption -->
          <video
            bind:this={videoElement}
            src={opened ? video?.file : undefined}
            class="hidden"
            crossorigin="anonymous"
            preload="auto"
            onloadedmetadata={handleVideoLoaded}
            onseeked={handleSeeked}
          ></video>
        </div>
      </div>

      <!-- Preview -->
      <aside class="custom-scrollbar-light flex w-[220px] shrink-0 flex-col gap-5 overflow-y-auto border-l border-line p-4 max-[1279px]:hidden">
        <section class="space-y-2">
          <div class="flex items-center justify-between">
            <h4 class="text-[12px] font-medium text-fg-secondary">首页推荐 · 4:3</h4>
            <label class="flex cursor-pointer items-center gap-1.5 text-[11px] text-fg-muted">
              <input type="checkbox" class="ui-checkbox" bind:checked={showCrop} />
              参考框
            </label>
          </div>
          <div class="ui-card overflow-hidden">
            <canvas bind:this={previewHome} width="376" height="282" class="block aspect-[4/3] w-full"></canvas>
            <p class="line-clamp-2 px-2 py-1.5 text-[11px] leading-snug text-fg">{title || "视频标题"}</p>
          </div>
        </section>
        <section class="space-y-2">
          <h4 class="text-[12px] font-medium text-fg-secondary">个人空间 · 16:9</h4>
          <div class="ui-card overflow-hidden">
            <canvas bind:this={previewWide} width="376" height="212" class="block aspect-video w-full"></canvas>
            <p class="line-clamp-2 px-2 py-1.5 text-[11px] leading-snug text-fg">{title || "视频标题"}</p>
          </div>
        </section>
        <p class="ui-hint">首页推荐只展示封面中间的 4:3 区域，重要的文字和贴纸请放在参考框内。</p>
      </aside>
    </div>

    <div class="ui-modal-footer justify-between">
      <span class="text-[11px] text-fg-muted">保存为 1920×1080 的 JPG 封面</span>
      <div class="flex gap-2">
        <button type="button" class="ui-btn" disabled={saving} onclick={handleClose}>取消</button>
        <button type="button" class="ui-btn ui-btn-primary" disabled={saving} onclick={handleSave}>
          {#if saving}<Loader2 class="h-3.5 w-3.5 animate-spin" />{/if}
          完成
        </button>
      </div>
    </div>
  </div>
</div>

<style>
  .toolbar-separator {
    margin: 0 4px;
    height: 16px;
    width: 1px;
    background: rgb(var(--c-line-strong));
  }

  .shadow-glyph {
    text-shadow: 1.5px 1.5px 1px rgb(var(--c-fg-faint));
  }

  .crop-shade {
    position: absolute;
    top: 0;
    bottom: 0;
    background: rgb(0 0 0 / 35%);
  }

  .handle {
    pointer-events: auto;
    position: absolute;
    display: flex;
    height: 20px;
    width: 20px;
    align-items: center;
    justify-content: center;
    border-radius: 9999px;
    border: 1px solid rgb(var(--c-line-strong));
    background: white;
    color: #3a4252;
    box-shadow: 0 1px 4px rgb(0 0 0 / 30%);
  }

  .edit-field {
    min-width: 0.6em;
    white-space: pre;
    outline: 1.5px dashed rgb(var(--c-accent));
    outline-offset: 2px;
    paint-order: stroke fill;
    cursor: text;
    user-select: text;
    -webkit-user-select: text;
  }

  input[type="range"] {
    -webkit-appearance: none;
    appearance: none;
    height: 4px;
    border-radius: 2px;
    background: rgb(var(--c-fg-faint) / 50%);
    cursor: pointer;
  }

  input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: white;
    box-shadow: 0 1px 3px rgb(0 0 0 / 35%);
  }

  input[type="range"]::-moz-range-thumb {
    width: 12px;
    height: 12px;
    border: none;
    border-radius: 50%;
    background: white;
  }

  input[type="range"]:focus {
    outline: none;
  }

  input[type="range"]:focus-visible {
    outline: 2px solid rgb(var(--c-accent));
    outline-offset: 4px;
  }

  input[type="range"]:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }
</style>
