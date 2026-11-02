<script lang="ts">
  import { onMount } from "svelte";
  import { Activity, AlertCircle, ChevronRight, Loader2, RefreshCw, X } from "lucide-svelte";
  import { loadRoomArchives, summarizeRoomArchives } from "../archives";
  import { trapFocus } from "../focus-trap";
  import { get, invoke } from "../invoker";
  import { pageVisibility } from "../page-visibility";
  import { platformLabel } from "../platform";
  import { formatBitrate, formatSize } from "../size-format";
  import { formatTimecode } from "../time-format";
  import { createStateRefresh, watchStateChanges } from "../state-refresh";
  import { archiveViewMode } from "../stores/archive-view";
  import type { RecordItem } from "../db";
  import type { RecorderInfo, RecorderStatus } from "../interface";
  import ArchiveList from "./ArchiveList.svelte";
  import ArchiveCover from "./ArchiveCover.svelte";
  import RoomCheckTimeline from "./RoomCheckTimeline.svelte";

  interface Props { room: RecorderInfo; onclose: () => void; }
  let { room, onclose }: Props = $props();
  let status: RecorderStatus | null = $state(null);
  let loading = $state(true);
  let refreshing = $state(false);
  let statusError = $state("");
  let archives: RecordItem[] = $state([]);
  let archivesLoading = $state(true);
  let archivesLoaded = $state(false);
  let archivesError = $state("");
  let archiveDialogOpen = $state(false);
  let coverUrl = $state<string | null>(null);
  let requestedCover: string | null = null;
  let coverRequest = 0;
  let disposed = false;
  const pageVisible = pageVisibility();

  const info = $derived(status?.recorder ?? room);
  const currentRecord = $derived(status?.current_record ?? null);
  const check = $derived(status?.check);
  const archiveTotals = $derived(summarizeRoomArchives(archives, room.room_info, currentRecord));

  function releaseCover() {
    if (coverUrl) URL.revokeObjectURL(coverUrl);
    coverUrl = null;
  }

  async function refreshCover(source = "") {
    if (requestedCover === source) return;
    requestedCover = source;
    const request = ++coverRequest;
    if (!source) { releaseCover(); return; }
    try {
      const response = await get(source);
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const blob = await response.blob();
      if (disposed || request !== coverRequest) return;
      const next = URL.createObjectURL(blob);
      releaseCover();
      coverUrl = next;
    } catch {
      if (!disposed && request === coverRequest) {
        requestedCover = null;
        releaseCover();
      }
    }
  }

  async function refreshStatus() {
    refreshing = true;
    try {
      const result = await invoke<RecorderStatus>("get_recorder_status", {
        platform: room.room_info.platform, roomId: room.room_info.room_id,
      });
      if (disposed) return;
      status = result;
      statusError = "";
      void refreshCover(result.recorder.room_info.room_cover);
    } catch (error) {
      if (!disposed) statusError = `状态加载失败：${String(error)}`;
    } finally {
      if (!disposed) { loading = false; refreshing = false; }
    }
  }

  async function refreshArchives() {
    archivesLoading = true;
    try {
      const result = await loadRoomArchives(room.room_info, () => !disposed);
      if (disposed) return;
      archives = result;
      archivesLoaded = true;
      archivesError = "";
    } catch (error) {
      if (!disposed) archivesError = `录播记录加载失败：${String(error)}`;
    } finally {
      if (!disposed) archivesLoading = false;
    }
  }

  const statusRefresh = createStateRefresh(refreshStatus, 500);
  const archivesRefresh = createStateRefresh(refreshArchives, 1000);
  function refresh() {
    void statusRefresh.run();
    void archivesRefresh.run();
  }

  function formatTimestamp(value: string | number | null | undefined, timeOnly = false) {
    if (value === null || value === undefined || value === "") return "—";
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? "—" : timeOnly ? date.toLocaleTimeString() : date.toLocaleString();
  }

  onMount(() => {
    const stopStatus = watchStateChanges(statusRefresh, ["recorders-changed", "records-changed"], 5000);
    const stopArchives = watchStateChanges(archivesRefresh, ["records-changed"]);
    return () => { disposed = true; ++coverRequest; releaseCover(); stopStatus(); stopArchives(); };
  });
</script>

<svelte:window onkeydown={(event) => {
  if (event.key === "Escape" && pageVisible() && !archiveDialogOpen && !event.defaultPrevented && !event.isComposing) {
    event.preventDefault();
    event.stopImmediatePropagation();
    onclose();
  }
}} />

<div class="ui-modal-backdrop">
  <button class="absolute inset-0" type="button" aria-label="关闭房间状态" onclick={onclose}></button>
  <div use:trapFocus={{ active: pageVisible() }} role="dialog" aria-modal="true" aria-labelledby="room-status-title" tabindex="-1" class="ui-modal room-status-modal relative w-[1040px] max-w-full" class:calendar-open={$archiveViewMode === "calendar"}>
    <div class="ui-modal-header">
      <div class="flex items-center gap-2"><Activity class="h-4 w-4 text-fg-muted" /><h2 id="room-status-title">房间状态</h2></div>
      <div class="flex items-center gap-1.5">
        <button class="ui-btn ui-btn-ghost ui-btn-sm" type="button" onclick={refresh} disabled={refreshing || archivesLoading}>
          <RefreshCw class={`h-3.5 w-3.5 ${refreshing ? "animate-spin motion-reduce:animate-none" : ""}`} />刷新
        </button>
        <button class="ui-btn ui-btn-ghost ui-btn-sm ui-btn-icon -mr-1.5" type="button" aria-label="关闭房间状态窗口" onclick={onclose}><X class="h-4 w-4" /></button>
      </div>
    </div>

    <div class="room-status-overview custom-scrollbar-light max-h-[52vh] shrink-0 overflow-y-auto border-b border-line px-4 py-4">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div class="flex min-w-0 flex-[1_1_240px] items-center gap-3">
          <ArchiveCover src={coverUrl || room.room_info.room_cover} alt="直播间封面" class="h-16 w-28 shrink-0 rounded-[7px]" loading="eager" />
          <div class="min-w-0 flex-1">
            <h3 class="truncate text-[14px] font-medium text-fg" title={info.room_info.room_title}>{info.room_info.room_title || "等待获取房间信息"}</h3>
            <div class="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-fg-muted">
              <span>{info.user_info.user_name || "—"}</span>
              <span class="select-text break-all">{platformLabel(info.room_info.platform)} #{info.room_info.room_id}</span>
              <details class="room-extra-info group">
                <summary class="inline-flex cursor-pointer list-none items-center gap-1 text-fg-muted hover:text-fg-secondary"><ChevronRight class="h-3 w-3 group-open:rotate-90" />更多房间信息</summary>
                <dl class="mt-2 grid gap-x-6 gap-y-2 rounded-[7px] bg-control p-2.5 sm:grid-cols-2">
                  <div class="flex gap-2"><dt class="shrink-0 text-fg-muted">主播 ID</dt><dd class="select-text break-all text-fg">{info.user_info.user_id || "—"}</dd></div>
                  <div class="flex gap-2"><dt class="shrink-0 text-fg-muted">平台直播 ID</dt><dd class="select-text break-all text-fg">{info.platform_live_id || "—"}</dd></div>
                  <div class="flex gap-2"><dt class="shrink-0 text-fg-muted">录播 ID</dt><dd class="select-text break-all text-fg">{info.live_id || "—"}</dd></div>
                  <div class="flex gap-2"><dt class="shrink-0 text-fg-muted">最近成功</dt><dd class="select-text tabular-nums text-fg">{formatTimestamp(check?.last_success_at)}</dd></div>
                </dl>
              </details>
            </div>
          </div>
        </div>
        <div class="flex items-center gap-2">
          <span class="ui-badge" class:ui-badge-success={!!check?.last_success_at && info.room_info.status}>{check?.last_success_at ? info.room_info.status ? "直播中" : "未开播" : "直播状态未知"}</span>
          <span class="ui-badge" class:ui-badge-accent={info.enabled}>{info.enabled ? "自动录制已启用" : "自动录制已停用"}</span>
        </div>
      </div>

      {#if statusError}
        <div role="alert" class="mt-3 flex items-start gap-2 rounded-[8px] bg-danger-soft p-2.5 text-[12px] text-danger-fg">
          <AlertCircle class="mt-0.5 h-4 w-4 shrink-0" /><span class="min-w-0 flex-1 select-text break-words">{statusError}</span>
          <button class="shrink-0 underline" onclick={() => statusRefresh.run()}>重试</button>
        </div>
      {/if}

      <div class="recording-overview mt-4" role="group" aria-label="录制统计">
        <section aria-labelledby="room-recording-totals-title" class="min-w-0">
          <div class="flex min-h-5 flex-wrap items-center justify-between gap-2 text-[11px]">
            <h3 id="room-recording-totals-title" class="text-[12px] font-medium text-fg-secondary">累计录制</h3>
            {#if !archivesLoaded}<span class="text-fg-muted">{archivesLoading ? "加载中…" : "统计暂不可用"}</span>{/if}
          </div>
          <dl aria-label="房间录制总计" aria-busy={archivesLoading && !archivesLoaded} title={archivesLoaded ? `统计这个房间现存的 ${archiveTotals.count} 条录播${info.recording ? "，包含当前正在录制的内容" : ""}。` : archivesLoading ? "正在加载全部录播记录" : "录播记录加载失败，暂时无法统计"} class="recording-metrics mt-2.5 grid grid-cols-2 gap-3">
            <div><dt>总录制时长</dt><dd data-room-total="length">{archivesLoaded ? formatTimecode(archiveTotals.length, 0) : "—"}</dd></div>
            <div><dt>总录制大小</dt><dd data-room-total="size">{archivesLoaded ? formatSize(archiveTotals.size) : "—"}</dd></div>
          </dl>
        </section>

        <section aria-labelledby="room-recording-title" class="min-w-0">
          <div class="flex min-h-5 flex-wrap items-center justify-between gap-2 text-[11px]">
            <div class="flex items-center gap-2"><h3 id="room-recording-title" class="text-[12px] font-medium text-fg-secondary">本次录制</h3><span class="ui-badge" class:ui-badge-danger={info.recording}>{info.recording ? "录制中" : "未在录制"}</span></div>
            {#if info.recording}<span class="tabular-nums text-fg-muted" title={formatTimestamp(currentRecord?.created_at)}>开始于 {formatTimestamp(currentRecord?.created_at, true)}</span>{/if}
          </div>
          {#if info.recording}
            <dl class="recording-metrics mt-2.5 grid grid-cols-[repeat(3,minmax(0,1fr))] gap-3">
              <div><dt>本次时长</dt><dd>{currentRecord ? formatTimecode(currentRecord.length, 0) : "—"}</dd></div>
              <div><dt>本次大小</dt><dd>{currentRecord ? formatSize(currentRecord.size) : "—"}</dd></div>
              <div><dt>平均码率</dt><dd>{currentRecord ? formatBitrate(currentRecord.size, currentRecord.length) : "—"}</dd></div>
            </dl>
          {:else}
            <p class="mt-3 text-[12px] leading-5 text-fg-secondary">{!status?.active ? "等待录制器就绪。" : !info.enabled ? "启用自动录制后，将在开播时开始录制。" : "等待直播开始或下一次录制。"}</p>
          {/if}
        </section>
      </div>

      <RoomCheckTimeline history={check?.history ?? []} {loading}>
        {#snippet statusBadge()}
          {#if loading || check?.checking}
            <span role="status" class="flex items-center gap-1 text-[11px] text-fg-muted"><Loader2 class="h-3 w-3 animate-spin motion-reduce:animate-none" />{loading ? "加载中" : "检查中"}</span>
          {:else if status && !status.active}<span class="ui-badge ui-badge-warning">录制器未就绪</span>
          {:else if check?.error}<span class="ui-badge ui-badge-danger">检查失败</span>
          {:else if check?.checked_at}<span class="ui-badge ui-badge-success">检查成功</span>
          {:else}<span class="text-[11px] text-fg-muted">等待首次检查</span>{/if}
        {/snippet}
        {#snippet metadata()}
          <span title={formatTimestamp(check?.checked_at)}>最近检查 <span class="select-text tabular-nums text-fg-secondary">{formatTimestamp(check?.checked_at, true)}</span></span>
          <span>耗时 <span class="tabular-nums text-fg-secondary">{check?.duration_ms == null ? "—" : `${check.duration_ms} ms`}</span></span>
          <span>检查间隔 <span class="tabular-nums text-fg-secondary">{status?.check_interval_secs ? `${status.check_interval_secs} 秒` : "—"}</span></span>
        {/snippet}
      </RoomCheckTimeline>

      {#if check?.error}
        <div role="alert" class="mt-3 rounded-[8px] bg-warning-soft p-2.5 text-[12px] text-warning-fg">
          <p class="select-text break-words">{check.error}</p>
          <p class="mt-1 text-[11px]">{check.last_success_at ? `房间信息保留自最近一次成功检查（${formatTimestamp(check.last_success_at)}）。` : "尚未成功获取房间信息。"}</p>
        </div>
      {/if}

    </div>

    <ArchiveList {archives} rooms={[info]} loading={archivesLoading} error={archivesError} compact bind:dialogOpen={archiveDialogOpen} onrefresh={() => archivesRefresh.run()} />

    <div class="flex shrink-0 items-center justify-between gap-3 border-t border-line bg-chrome px-4 py-2 text-[11px] text-fg-muted">
      <span>状态自动更新，累计数据按现存录播计算。</span>
      <button type="button" class="ui-btn ui-btn-sm" onclick={onclose}>关闭</button>
    </div>
  </div>
</div>

<style>
  .room-status-modal { max-height: min(85vh, 900px); }
  .room-extra-info[open] { flex-basis: 100%; }
  .recording-overview { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 340px), 1fr)); gap: 16px 28px; }
  .recording-metrics > div { min-width: 0; }
  .recording-metrics dt { color: rgb(var(--c-fg-muted)); font-size: 11px; }
  .recording-metrics dd { margin-top: 2px; color: rgb(var(--c-fg)); font-size: 20px; font-weight: 500; line-height: 30px; font-variant-numeric: tabular-nums; white-space: nowrap; }
  summary::-webkit-details-marker { display: none; }
  @media (max-height: 850px) {
    .room-status-modal { max-height: calc(100vh - 72px); }
    .room-status-overview { max-height: min(52vh, calc(100vh - 380px)); }
    .calendar-open .room-status-overview { max-height: calc(100vh - 460px); }
  }
</style>
