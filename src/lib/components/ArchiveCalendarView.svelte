<script lang="ts">
  import { onDestroy, type Snippet } from "svelte";
  import { ChevronLeft, ChevronRight, X } from "lucide-svelte";
  import {
    ARCHIVE_CALENDAR_SPANS, buildArchiveCalendar, calendarDateKey, latestArchiveDate,
    type ArchiveCalendarEvent,
  } from "../archive-calendar";
  import { archiveKey } from "../archives";
  import type { RecordItem } from "../db";
  import type { RecorderInfo } from "../interface";
  import { trapFocus } from "../focus-trap";
  import { pageVisibility } from "../page-visibility";
  import { platformLabel } from "../platform";
  import { formatBitrate, formatSize } from "../size-format";
  import { formatTimecode } from "../time-format";
  import ArchiveCover from "./ArchiveCover.svelte";

  let {
    archives, rooms = [], compact = false, selectedArchives, recordingKeys,
    ontoggle, actions, onshowlist, menuOpen = $bindable(false),
  }: {
    archives: RecordItem[]; rooms?: RecorderInfo[]; compact?: boolean;
    selectedArchives: ReadonlySet<string>; recordingKeys: ReadonlySet<string>;
    ontoggle: (liveId: string) => void; actions: Snippet<[RecordItem, () => void]>;
    onshowlist: () => void; menuOpen?: boolean;
  } = $props();

  const id = $props.id();
  const pageVisible = pageVisibility();
  let requestedSpanHours = $state<number | null>(null);
  const spanHours = $derived(requestedSpanHours ?? (compact ? 12 : 24));
  let anchor = $state<number | null>(null);
  let selectedKey = $state<string | null>(null);
  let trigger: HTMLButtonElement | null = null;
  let root: HTMLDivElement | undefined = $state();
  let popover: HTMLDivElement | undefined = $state();
  let viewportWidth = $state(0);
  let drag = $state<{ id: number; x: number; center: number; width: number; hours: number } | null>(null);
  const latest = $derived(latestArchiveDate(archives));
  const layout = $derived(buildArchiveCalendar(archives, anchor ?? latest ?? Date.now(), spanHours, viewportWidth || 960));
  const laneLayout = $derived.by(() => {
    const heights = Array<number>(layout.lanes).fill(0);
    for (const event of layout.events) heights[event.lane] = Math.max(heights[event.lane], eventHeight(event));
    let top = 8;
    const offsets = heights.map(height => { const offset = top; top += height + 8; return offset; });
    return { offsets, height: Math.max(96, top + 8) };
  });
  const selectedArchive = $derived(archives.find(archive => archiveKey(archive) === selectedKey) ?? null);
  const selectedEvent = $derived(layout.events.find(event => archiveKey(event.archive) === selectedKey));
  const roomNames = $derived(new Map(rooms.map(room => [
    `${room.room_info.platform}:${room.room_info.room_id}`, room.user_info.user_name || room.room_info.room_id,
  ])));

  $effect(() => { if (anchor === null && latest !== null) { const date = new Date(latest); date.setMinutes(0, 0, 0); anchor = date.getTime(); } });
  $effect(() => {
    menuOpen = selectedArchive !== null && selectedEvent !== undefined;
    if (selectedKey && (!selectedArchive || !selectedEvent)) selectedKey = null;
  });
  $effect(() => {
    if (!pageVisible()) { selectedKey = null; drag = null; }
  });
  onDestroy(() => { menuOpen = false; });

  function closeMenu() { const wasOpen = selectedKey !== null; selectedKey = null; if (wasOpen) trigger?.focus({ preventScroll: true }); }
  function eventHeight(event: ArchiveCalendarEvent) { return event.width >= 176 ? 54 : event.width >= 40 ? 72 : 24; }
  function navigate(date: number) { if (!Number.isFinite(new Date(date).getTime())) return; closeMenu(); anchor = date; }
  function move(direction: number) { navigate((anchor ?? latest ?? Date.now()) + direction * spanHours * 3_600_000 / 2); }
  function zoom(hours: number) { closeMenu(); requestedSpanHours = hours; }
  function inputDate(timestamp: number) { const date = new Date(timestamp); return `${calendarDateKey(timestamp)}T${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`; }
  function startPan(event: PointerEvent) {
    if (event.button !== 0) return;
    closeMenu();
    drag = { id: event.pointerId, x: event.clientX, center: anchor ?? latest ?? Date.now(), width: Math.max(1, viewportWidth), hours: spanHours };
    (event.currentTarget as HTMLButtonElement).setPointerCapture(event.pointerId);
  }
  function pan(event: PointerEvent) {
    if (drag?.id !== event.pointerId) return;
    anchor = drag.center - (event.clientX - drag.x) / drag.width * drag.hours * 3_600_000;
  }
  function wheelPan(node: HTMLElement) {
    const handler = (event: WheelEvent) => {
      const delta = event.deltaX || (event.shiftKey ? event.deltaY : 0);
      if (!delta) return;
      event.preventDefault();
      navigate((anchor ?? latest ?? Date.now()) + delta / Math.max(1, viewportWidth) * spanHours * 3_600_000);
    };
    node.addEventListener("wheel", handler, { passive: false });
    return { destroy() { node.removeEventListener("wheel", handler); } };
  }
  function openEvent(event: ArchiveCalendarEvent, element: HTMLButtonElement) {
    const key = archiveKey(event.archive);
    trigger = element;
    selectedKey = selectedKey === key ? null : key;
  }
  function time(timestamp: number) {
    return new Date(timestamp).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hour12: false });
  }
  function eventTime(event: ArchiveCalendarEvent) {
    if (!event.timingKnown) return `创建于 ${time(event.start)}，录制时间范围未知`;
    const sameDate = calendarDateKey(event.start) === calendarDateKey(event.end);
    return sameDate ? `${time(event.start)}–${time(event.end)}`
      : `${new Date(event.start).toLocaleDateString(undefined, { month: "numeric", day: "numeric" })} ${time(event.start)}–${new Date(event.end).toLocaleDateString(undefined, { month: "numeric", day: "numeric" })} ${time(event.end)}`;
  }
  function rangeLabel() {
    const format = (time: number) => new Date(time).toLocaleString(undefined, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit", hour12: false });
    return `${format(layout.start)} — ${format(layout.end)}`;
  }

  // Keep the menu inside its containing dialog/page, above the scrolling calendar.
  function placeMenu(node: HTMLDivElement) {
    const host = root?.closest<HTMLElement>(".ui-modal, .page") ?? document.body;
    host.appendChild(node);
    function place() {
      if (!trigger?.isConnected) { closeMenu(); return; }
      const bounds = host.getBoundingClientRect();
      const target = trigger.getBoundingClientRect();
      const width = Math.min(340, bounds.width - 24);
      node.style.width = `${width}px`;
      node.style.maxHeight = `${Math.max(160, bounds.height - 24)}px`;
      const fitsRight = target.right + width + 10 <= bounds.right - 12;
      const fitsLeft = target.left - width - 10 >= bounds.left + 12;
      const left = fitsRight ? target.right + 10 : fitsLeft ? target.left - width - 10 : target.left;
      let top = target.top;
      if (!fitsRight && !fitsLeft) top = target.bottom + node.offsetHeight + 8 <= bounds.bottom - 12
        ? target.bottom + 8 : target.top - node.offsetHeight - 8;
      node.style.left = `${Math.max(12, Math.min(bounds.width - width - 12, left - bounds.left)) + host.scrollLeft - host.clientLeft}px`;
      node.style.top = `${Math.max(12, Math.min(bounds.height - node.offsetHeight - 12, top - bounds.top)) + host.scrollTop - host.clientTop}px`;
    }
    const observer = new ResizeObserver(place);
    observer.observe(node);
    if (trigger) observer.observe(trigger);
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    queueMicrotask(place);
    return { destroy() {
      observer.disconnect();
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
      node.remove();
    } };
  }
</script>

<svelte:window
  onpointerdown={(event) => {
    const target = event.target as Node;
    if (selectedKey && !popover?.contains(target) && !trigger?.contains(target)) closeMenu();
  }}
  onkeydowncapture={(event) => {
    if (!selectedKey || !popover?.contains(document.activeElement) || event.key !== "Escape" || event.defaultPrevented || event.isComposing) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    closeMenu();
  }}
/>

<div class="archive-calendar" class:compact class:is-dragging={drag !== null} bind:this={root} role="region" aria-label="录播时间日历">
  <div class="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-line px-3 py-2">
    <div class="flex flex-wrap items-center gap-2">
      <div class="flex items-center gap-0.5">
        <button class="ui-btn ui-btn-ghost ui-btn-sm ui-btn-icon" aria-label="向前浏览时间轴" onclick={() => move(-1)}><ChevronLeft class="h-3.5 w-3.5" /></button>
        <span class="text-center text-[11px] font-medium tabular-nums text-fg" style={`min-width:${compact ? 176 : 206}px`}>{rangeLabel()}</span>
        <button class="ui-btn ui-btn-ghost ui-btn-sm ui-btn-icon" aria-label="向后浏览时间轴" onclick={() => move(1)}><ChevronRight class="h-3.5 w-3.5" /></button>
      </div>
      <button class="ui-btn ui-btn-sm" onclick={() => navigate(Date.now())}>现在</button>
      <button class="ui-btn ui-btn-ghost ui-btn-sm" disabled={latest === null} onclick={() => latest !== null && navigate(latest)}>最近录播</button>
      <input type="datetime-local" class="ui-input h-7 w-[170px] px-2 py-0 text-[11px]" aria-label="定位时间" value={inputDate(anchor ?? latest ?? Date.now())} onchange={(event) => { if (event.currentTarget.value) navigate(Date.parse(event.currentTarget.value)); }} />
    </div>
    <div class="flex items-center gap-2 text-[11px] text-fg-muted">
      <span class="tabular-nums">该范围 {layout.count} 条</span>
      <label class="flex items-center gap-1.5">缩放
        <select class="ui-input h-7 py-0 text-[11px]" aria-label="时间轴缩放" value={spanHours} onchange={(event) => zoom(Number(event.currentTarget.value))}>
          {#each ARCHIVE_CALENDAR_SPANS as hours}<option value={hours}>{hours < 168 ? `${hours} 小时` : `${hours / 24} 天`}</option>{/each}
        </select>
      </label>
    </div>
  </div>

  <div class="calendar-scroll custom-scrollbar-light" style={`height:${laneLayout.height + 48}px`} use:wheelPan bind:clientWidth={viewportWidth}>
    <div class="timeline-board" style={`width:${layout.width}px`}>
      <div class="timeline-axis" aria-label="连续时间刻度">
        {#each layout.ticks as axisTick (axisTick.time)}
          <span class="axis-tick" class:axis-start={axisTick.align === "start"} class:axis-end={axisTick.align === "end"} style={`left:${axisTick.left}px`}>
            <span class="block text-[11px] tabular-nums text-fg-secondary">{axisTick.label}</span><span class="mt-0.5 block text-[10px] text-fg-muted">{axisTick.detail}</span>
          </span>
        {/each}
      </div>
      <div class="timeline-canvas" style={`height:${laneLayout.height}px`}>
        <button type="button" class="timeline-pan" aria-label="拖动时间轴，或用左右方向键浏览" onpointerdown={startPan} onpointermove={pan} onpointerup={() => drag = null} onpointercancel={() => drag = null} onkeydown={(event) => { if (event.key === "ArrowLeft" || event.key === "ArrowRight") { event.preventDefault(); move(event.key === "ArrowLeft" ? -1 : 1); } }}></button>
        <div class="pointer-events-none absolute inset-0" aria-hidden="true">{#each layout.ticks as axisTick (axisTick.time)}<span class="absolute inset-y-0 border-l border-line/60" style={`left:${axisTick.left}px`}></span>{/each}</div>
        {#each layout.events as event (event.key)}
          {@const key = archiveKey(event.archive)}
          {@const recording = recordingKeys.has(key)}
          <button type="button" class="calendar-event" class:is-open={selectedKey === key} class:is-selected={selectedArchives.has(event.archive.live_id)} class:is-recording={recording}
            style={`top:${laneLayout.offsets[event.lane]}px;height:${eventHeight(event)}px;left:${event.hitLeft}px;width:${event.hitWidth}px`}
            aria-label={`录播事件：${event.archive.title || "未命名录播"}，${new Date(event.start).toLocaleString()}，时长 ${formatTimecode(event.archive.length, 0)}${recording ? "，录制中" : ""}`}
            aria-haspopup="dialog" aria-expanded={selectedKey === key} data-archive-key={key} data-recording={recording} data-lane={event.lane}
            title={`${event.archive.title || "未命名录播"}\n${event.timingKnown ? `${new Date(event.start).toLocaleString()} — ${new Date(event.end).toLocaleString()}` : `创建于 ${new Date(event.start).toLocaleString()}，录制时间范围未知`}`}
            onclick={(click) => openEvent(event, click.currentTarget)}>
            <span class="event-block" class:padded={event.width >= 40} style={`left:${event.left - event.hitLeft}px;width:${event.width}px`}>
              {#if event.width >= 40}
                <span class="event-body" class:side-cover={event.width >= 176}>
                  <ArchiveCover src={event.archive.cover} class={event.width >= 176 ? "h-[45px] w-20 shrink-0 rounded-[3px]" : "h-8 w-full shrink-0 rounded-[3px]"} />
                  <span class="min-w-0 flex-1">
                    <span class="event-title">{event.continuesBefore ? "← " : ""}{event.archive.title || "未命名录播"}{event.continuesAfter ? " →" : ""}</span>
                    {#if event.width >= 176}<span class="mt-0.5 block truncate text-[10px] leading-3 tabular-nums opacity-80">{eventTime(event)}</span>{/if}
                  </span>
                </span>
              {/if}
            </span>
          </button>
        {/each}
        {#if layout.count === 0}<div class="pointer-events-none absolute inset-0 flex items-center justify-center text-[12px] text-fg-muted">这个时间范围没有录播</div>{/if}
      </div>
    </div>
  </div>
  <div class="flex shrink-0 flex-wrap items-center justify-between gap-1 border-t border-line px-3 py-2 text-[11px] text-fg-muted">
    <span>拖动浏览时间，点击事件查看详情与操作</span>
    {#if layout.undated}<button class="text-accent-fg hover:underline" onclick={onshowlist}>{layout.undated} 条录播缺少时间，切换列表查看</button>{/if}
  </div>
</div>

{#if selectedArchive && selectedEvent}
  <div use:placeMenu use:trapFocus={{ active: pageVisible() }} role="dialog" aria-modal="false" aria-labelledby={`${id}-event-title`} tabindex="-1" bind:this={popover} class="calendar-menu custom-scrollbar-light">
    <div class="relative -mx-3 -mt-3 mb-3">
      <ArchiveCover src={selectedArchive.cover} alt={`直播封面：${selectedArchive.title || "未命名录播"}`} class="aspect-video w-full rounded-t-[9px]" loading="eager" />
      <button class="absolute right-2 top-2 flex h-6 w-6 items-center justify-center rounded-full bg-black/55 text-white backdrop-blur-sm hover:bg-black/70" aria-label="关闭录播操作" onclick={closeMenu}><X class="h-3.5 w-3.5" /></button>
    </div>
    <h3 id={`${id}-event-title`} class="select-text break-words text-[13px] font-medium text-fg">{selectedArchive.title || "未命名录播"}</h3>
    <div class="mt-1 flex items-center gap-2 text-[11px] text-fg-muted"><span>{platformLabel(selectedArchive.platform)} · {roomNames.get(`${selectedArchive.platform}:${selectedArchive.room_id}`) || selectedArchive.room_id}</span>{#if recordingKeys.has(archiveKey(selectedArchive))}<span class="ui-badge ui-badge-danger">录制中</span>{/if}</div>
    <dl class="my-3 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-[11px] tabular-nums">
      <dt class="text-fg-muted">{selectedEvent.timingKnown ? "开始" : "创建"}</dt><dd class="select-text text-fg-secondary">{new Date(selectedEvent.start).toLocaleString()}</dd>
      <dt class="text-fg-muted">{recordingKeys.has(archiveKey(selectedArchive)) ? "已录至" : "结束"}</dt><dd class="select-text text-fg-secondary">{selectedEvent.timingKnown ? new Date(selectedEvent.end).toLocaleString() : "未知"}</dd>
      <dt class="text-fg-muted">录制时长</dt><dd class="text-fg-secondary">{formatTimecode(selectedArchive.length, 0)}</dd>
      <dt class="text-fg-muted">大小 / 码率</dt><dd class="text-fg-secondary">{formatSize(selectedArchive.size)} / {formatBitrate(selectedArchive.size, selectedArchive.length)}</dd>
    </dl>
    <div class="-mx-1 border-t border-line pt-1">{@render actions(selectedArchive, closeMenu)}</div>
    <label class="mt-1 flex cursor-pointer items-center gap-2 border-t border-line pt-2 text-[11px] text-fg-muted"><input type="checkbox" class="ui-checkbox" checked={selectedArchives.has(selectedArchive.live_id)} onchange={() => ontoggle(selectedArchive.live_id)} />加入批量选择</label>
  </div>
{/if}

<style>
  .archive-calendar { display: flex; min-height: 0; flex: 1; flex-direction: column; }
  .calendar-scroll { min-height: 0; flex: 1 1 auto; overflow-y: auto; overflow-x: hidden; overscroll-behavior: contain; }
  .timeline-board { position: relative; }
  .timeline-axis { position: sticky; top: 0; z-index: 10; height: 48px; border-bottom: 1px solid rgb(var(--c-line)); background: rgb(var(--c-surface)); }
  .axis-tick { position: absolute; top: 6px; transform: translateX(-50%); white-space: nowrap; text-align: center; }
  .axis-tick.axis-start { transform: none; padding-left: 8px; text-align: left; }
  .axis-tick.axis-end { transform: translateX(-100%); padding-right: 8px; text-align: right; }
  .timeline-canvas { position: relative; }
  .timeline-pan { position: absolute; inset: 0; width: 100%; height: 100%; cursor: grab; touch-action: pan-y; }
  .is-dragging .timeline-pan { cursor: grabbing; }
  .calendar-event { position: absolute; z-index: 1; padding: 0; text-align: left; }
  .event-block { position: absolute; top: 0; height: 100%; display: flex; flex-direction: column; overflow: hidden; border-radius: 5px; background: rgb(var(--c-accent-soft)); box-shadow: inset 3px 0 0 rgb(var(--c-accent)); color: rgb(var(--c-accent-fg)); transition: width 250ms ease, background-color 120ms; }
  .event-block.padded { padding: 3px 6px; }
  .event-block > span { flex-shrink: 0; }
  .event-body { display: flex; min-width: 0; height: 100%; flex-direction: column; gap: 2px; }
  .event-body.side-cover { flex-direction: row; align-items: center; gap: 6px; }
  .calendar-event:hover { z-index: 2; }
  .calendar-event:hover .event-block { filter: brightness(.97); }
  .calendar-event.is-open .event-block, .calendar-event:focus-visible .event-block { outline: 2px solid rgb(var(--c-accent)); outline-offset: 1px; }
  .calendar-event.is-selected .event-block { outline: 1px solid rgb(var(--c-accent)); }
  .calendar-event.is-recording .event-block { background: rgb(var(--c-danger-soft)); box-shadow: inset 3px 0 0 rgb(var(--c-danger)); color: rgb(var(--c-danger-fg)); }
  .calendar-event.is-recording.is-open .event-block { outline-color: rgb(var(--c-danger)); }
  .event-title { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; overflow: hidden; font-size: 11px; font-weight: 500; line-height: 15px; }
  .calendar-menu { position: absolute; z-index: 1100; overflow: auto; border: 1px solid rgb(var(--c-line-strong)); border-radius: 10px; background: rgb(var(--c-surface)); padding: 12px; color: rgb(var(--c-fg)); box-shadow: 0 8px 32px rgb(0 0 0 / .16); }
  @media (prefers-reduced-motion: reduce) { .event-block { transition: none; } }
</style>
