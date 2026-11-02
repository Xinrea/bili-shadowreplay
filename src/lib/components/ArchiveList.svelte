<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import {
    ArrowDown, ArrowUp, CalendarDays, ChevronLeft, ChevronRight, FileVideo, History,
    List, Loader2, Play, Sparkles, Trash2,
  } from "lucide-svelte";
  import { archiveKey, sortArchives, type ArchiveSortField, type ArchiveSortOrder } from "../archives";
  import type { RecordItem, RecordSummaryStatus } from "../db";
  import type { RecorderInfo } from "../interface";
  import { trapFocus } from "../focus-trap";
  import { pageVisibility } from "../page-visibility";
  import { invoke } from "../invoker";
  import { liveRoomUrl, platformLabel } from "../platform";
  import { formatBitrate, formatSize } from "../size-format";
  import { formatTimecode } from "../time-format";
  import { createStateRefresh, watchStateChanges } from "../state-refresh";
  import { archiveViewMode, restoreArchiveViewPreference, setArchiveViewMode } from "../stores/archive-view";
  import ArchiveCalendarView from "./ArchiveCalendarView.svelte";
  import ArchiveCover from "./ArchiveCover.svelte";
  import ArchiveSummaryModal from "./ArchiveSummaryModal.svelte";
  import BatchActionLayout from "./BatchActionLayout.svelte";
  import DeleteConfirmActions from "./DeleteConfirmActions.svelte";
  import GenerateWholeClipModal from "./GenerateWholeClipModal.svelte";
  import PlatformIcon from "./PlatformIcon.svelte";

  interface Props {
    archives: RecordItem[];
    rooms?: RecorderInfo[];
    loading?: boolean;
    error?: string;
    compact?: boolean;
    filterRooms?: boolean;
    dialogOpen?: boolean;
    onrefresh: () => Promise<void>;
  }

  let {
    archives, rooms = [], loading = false, error = "", compact = false,
    filterRooms = false, dialogOpen = $bindable(false), onrefresh,
  }: Props = $props();
  const pageVisible = pageVisibility();

  const sortOptions: { value: ArchiveSortField; label: string }[] = [
    { value: "created_at", label: "直播时间" }, { value: "title", label: "标题" },
    { value: "length", label: "时长" }, { value: "size", label: "大小" },
    { value: "room_id", label: "直播间号" },
  ];
  const pageSizeOptions = [10, 20, 50, 100];
  let pageSize = $state(20);
  let currentPage = $state(1);
  let selectedRoomKey = $state("");
  let sortBy: ArchiveSortField = $state("created_at");
  let sortOrder: ArchiveSortOrder = $state("desc");
  const selectedArchives = new SvelteSet<string>();
  let showDeleteConfirm = $state(false);
  let deleting = $state(false);
  let archiveToDelete: RecordItem | null = $state(null);
  let showWholeClipModal = $state(false);
  let wholeClipArchive: RecordItem | null = $state(null);
  let showSummaryModal = $state(false);
  let summaryArchive: RecordItem | null = $state(null);
  let calendarMenuOpen = $state(false);
  let summaryStatuses: Record<string, RecordSummaryStatus> = $state({});
  let actionError = $state("");
  let disposed = false;

  const roomOptions = $derived(rooms.map((room) => ({
    key: `${room.room_info.platform}:${room.room_info.room_id}`,
    label: `${room.user_info.user_name || room.room_info.room_title || platformLabel(room.room_info.platform)} (${room.room_info.room_id})`,
  })).sort((a, b) => a.label.localeCompare(b.label)));
  const filtered = $derived(sortArchives(archives.filter((archive) => !selectedRoomKey ||
    `${archive.platform}:${archive.room_id}` === selectedRoomKey), sortBy, sortOrder));
  const totalPages = $derived(Math.max(1, Math.ceil(filtered.length / pageSize)));
  const page = $derived(Math.min(currentPage, totalPages));
  const filteredArchives = $derived(filtered.slice((page - 1) * pageSize, page * pageSize));
  const pageSelected = $derived(filteredArchives.length > 0 &&
    filteredArchives.every((archive) => selectedArchives.has(archive.live_id)));
  const pagePartlySelected = $derived(!pageSelected &&
    filteredArchives.some((archive) => selectedArchives.has(archive.live_id)));
  const recordingKeys = $derived(new Set(rooms.filter((room) => room.recording).map((room) =>
    archiveKey({ platform: room.room_info.platform, room_id: room.room_info.room_id, live_id: room.live_id }))));

  $effect(() => { dialogOpen = showDeleteConfirm || showWholeClipModal || showSummaryModal || calendarMenuOpen; });
  $effect(() => { if (currentPage > totalPages) currentPage = totalPages; });
  $effect(() => {
    if (deleting) return;
    const available = new Set(archives.map((archive) => archive.live_id));
    for (const id of selectedArchives) if (!available.has(id)) selectedArchives.delete(id);
  });

  async function loadSummaryStatuses() {
    try {
      const statuses = await invoke<RecordSummaryStatus[]>("get_archive_summary_statuses");
      if (!disposed) summaryStatuses = Object.fromEntries(statuses.map((status) => [archiveKey(status), status]));
    } catch (error) {
      console.warn("Failed to load Summary statuses:", error);
    }
  }

  const summaryRefresh = createStateRefresh(loadSummaryStatuses, 1000);
  onMount(() => {
    restoreArchiveViewPreference();
    const savedSize = Number(localStorage.getItem("archive-page-size"));
    if (pageSizeOptions.includes(savedSize)) pageSize = savedSize;
    const stop = watchStateChanges(summaryRefresh, ["records-changed"]);
    return () => { disposed = true; stop(); };
  });

  function changePageSize() {
    currentPage = 1;
    localStorage.setItem("archive-page-size", String(pageSize));
  }

  function changeRoom() {
    currentPage = 1;
    selectedArchives.clear();
  }

  function toggleArchiveSelection(liveId: string) {
    if (selectedArchives.has(liveId)) selectedArchives.delete(liveId);
    else selectedArchives.add(liveId);
  }

  function selectAllArchives() {
    const deselect = pageSelected;
    for (const archive of filteredArchives) {
      if (deselect) selectedArchives.delete(archive.live_id);
      else selectedArchives.add(archive.live_id);
    }
  }

  async function deleteArchive(archive: RecordItem) {
    await invoke("delete_archive", { platform: archive.platform, roomId: archive.room_id, liveId: archive.live_id });
    selectedArchives.delete(archive.live_id);
    await onrefresh();
    showDeleteConfirm = false;
    archiveToDelete = null;
  }

  async function deleteSelectedArchives() {
    // Keep the initial targets across event refreshes and retain unfinished IDs on failure.
    const targets = archives.filter((archive) => selectedArchives.has(archive.live_id));
    for (const archive of targets) {
      await invoke("delete_archive", { platform: archive.platform, roomId: archive.room_id, liveId: archive.live_id });
      selectedArchives.delete(archive.live_id);
    }
    await onrefresh();
    showDeleteConfirm = false;
    archiveToDelete = null;
  }

  async function playArchive(archive: RecordItem) {
    actionError = "";
    try {
      await invoke("open_live", { platform: archive.platform, roomId: archive.room_id, liveId: archive.live_id });
    } catch (error) {
      actionError = `预览失败：${String(error)}`;
    }
  }

  function summaryButtonTitle(archive: RecordItem) {
    switch (summaryStatuses[archiveKey(archive)]?.status) {
      case "processing": return "总结生成中";
      case "success": return "查看总结";
      case "failed": return "总结生成失败，点击重试";
      default: return "生成总结";
    }
  }

  function updateSummaryStatus(status: RecordSummaryStatus) {
    summaryStatuses = { ...summaryStatuses, [archiveKey(status)]: status };
  }

  function cancelDelete() {
    if (deleting) return;
    showDeleteConfirm = false;
    archiveToDelete = null;
  }

  function handleDeleteKeydown(event: KeyboardEvent) {
    if (!showDeleteConfirm || !pageVisible() || event.key !== "Escape" || event.defaultPrevented || event.isComposing) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    cancelDelete();
  }
</script>

<svelte:window onkeydown={handleDeleteKeydown} />

{#snippet deleteSelectedButton()}
  <button class="ui-btn ui-btn-danger" onclick={() => { archiveToDelete = null; showDeleteConfirm = true; }}>
    <Trash2 class="h-3.5 w-3.5" />删除选中
  </button>
{/snippet}

{#snippet archiveActions(archive: RecordItem, menu: boolean, onaction: () => void)}
  {@const summaryStatus = summaryStatuses[archiveKey(archive)]?.status}
  <!-- Dismissing the event menu clears its reactive archive argument. Capture it first. -->
  <div class={menu ? "grid gap-0.5" : "flex items-center gap-0.5"}>
    <button class={`${menu ? "ui-menu-item w-full" : "archive-action"} hover:bg-accent-soft text-accent-fg`} title="预览录播" aria-label={`预览录播：${archive.title}`} disabled={archive.length <= 0} onclick={() => { const target = archive; onaction(); void playArchive(target); }}><Play class="h-4 w-4" />{#if menu}<span>预览录播</span>{/if}</button>
    <button class={`${menu ? "ui-menu-item w-full" : "archive-action"} hover:bg-accent-soft text-accent-fg`} title="生成完整切片" aria-label={`生成完整切片：${archive.title}`} disabled={archive.length <= 0} onclick={() => { const target = archive; onaction(); wholeClipArchive = target; showWholeClipModal = true; }}><FileVideo class="h-4 w-4" />{#if menu}<span>生成完整切片</span>{/if}</button>
    <button class={`${menu ? "ui-menu-item w-full" : "archive-action"} hover:bg-accent-soft`} class:text-success-fg={summaryStatus === "success"} class:text-danger-fg={summaryStatus === "failed"} class:text-accent-fg={summaryStatus !== "success" && summaryStatus !== "failed"} title={summaryButtonTitle(archive)} aria-label={`${summaryButtonTitle(archive)}：${archive.title}`} disabled={archive.length <= 0} onclick={() => { const target = archive; onaction(); summaryArchive = target; showSummaryModal = true; }}>
      {#if summaryStatus === "processing"}<Loader2 class="h-4 w-4 animate-spin motion-reduce:animate-none" />{:else}<Sparkles class="h-4 w-4" />{/if}{#if menu}<span>{summaryButtonTitle(archive)}</span>{/if}
    </button>
    <button class={`${menu ? "ui-menu-item w-full" : "archive-action"} text-danger-fg hover:bg-danger-soft`} title="删除记录" aria-label={`删除记录：${archive.title}`} onclick={() => { const target = archive; onaction(); archiveToDelete = target; showDeleteConfirm = true; }}><Trash2 class="h-4 w-4" />{#if menu}<span>删除录播</span>{/if}</button>
  </div>
{/snippet}

{#snippet calendarActions(archive: RecordItem, close: () => void)}
  {@render archiveActions(archive, true, close)}
{/snippet}

<BatchActionLayout count={selectedArchives.size} itemLabel="录播" onclear={() => selectedArchives.clear()} {compact} scroll={$archiveViewMode === "list"}>
  {#snippet actions()}{@render deleteSelectedButton()}{/snippet}
  <div class={$archiveViewMode === "calendar" ? "flex min-h-0 flex-1 flex-col gap-3" : "space-y-3"}>
    <div class={compact ? "shrink-0 space-y-2" : "ui-card shrink-0 p-3"}>
      <div class="flex flex-wrap items-center justify-between gap-2.5">
        <div class="flex min-w-0 flex-wrap items-center gap-2.5">
          {#if compact}<h3 class="text-[13px] font-medium text-fg">录播记录</h3>{/if}
          {#if filterRooms}
            <select class="ui-input max-w-[240px]" aria-label="筛选直播间" bind:value={selectedRoomKey} onchange={changeRoom}>
              <option value="">所有直播间</option>
              {#each roomOptions as option}<option value={option.key}>{option.label}</option>{/each}
            </select>
          {/if}
          <span class="whitespace-nowrap text-[11px] text-fg-muted">共 <span class="tabular-nums">{filtered.length}</span> 条</span>
          {#if $archiveViewMode === "list"}<label class="flex items-center gap-1.5 text-[11px] text-fg-muted">
            每页
            <select class="ui-input h-6 min-w-[56px] rounded-[6px] py-0 pl-2 text-[11px]" aria-label="每页录播数" bind:value={pageSize} onchange={changePageSize}>
              {#each pageSizeOptions as size}<option value={size}>{size}</option>{/each}
            </select>
          </label>
          {#if totalPages > 1}
            <div class="flex items-center gap-1 text-[11px] tabular-nums">
              <button class="ui-btn ui-btn-ghost ui-btn-sm ui-btn-icon" title="上一页" aria-label="上一页" disabled={page === 1} onclick={() => currentPage = page - 1}><ChevronLeft class="h-3.5 w-3.5" /></button>
              <span class="min-w-[44px] text-center text-fg-secondary" aria-label="当前页">{page} / {totalPages}</span>
              <button class="ui-btn ui-btn-ghost ui-btn-sm ui-btn-icon" title="下一页" aria-label="下一页" disabled={page === totalPages} onclick={() => currentPage = page + 1}><ChevronRight class="h-3.5 w-3.5" /></button>
            </div>
          {/if}{/if}
        </div>
        <div class="flex items-center gap-1.5">
          <div role="group" aria-label="录播视图" class="flex items-center gap-0.5 rounded-[8px] bg-control p-0.5">
            <button type="button" class="ui-btn ui-btn-ghost ui-btn-sm" class:ui-btn-active={$archiveViewMode === "list"} aria-label="列表视图" aria-pressed={$archiveViewMode === "list"} onclick={() => setArchiveViewMode("list")}><List class="h-3.5 w-3.5" />列表</button>
            <button type="button" class="ui-btn ui-btn-ghost ui-btn-sm" class:ui-btn-active={$archiveViewMode === "calendar"} aria-label="日历视图" aria-pressed={$archiveViewMode === "calendar"} onclick={() => setArchiveViewMode("calendar")}><CalendarDays class="h-3.5 w-3.5" />日历</button>
          </div>
          {#if $archiveViewMode === "list"}<select class="ui-input h-7 min-w-[110px] py-0 text-[11px]" aria-label="录播排序" bind:value={sortBy} onchange={() => currentPage = 1}>
            {#each sortOptions.filter((option) => !compact || option.value !== "room_id") as option}<option value={option.value}>按{option.label}</option>{/each}
          </select>
          <button class="ui-btn ui-btn-sm ui-btn-icon" title={sortOrder === "desc" ? "降序，点击切换升序" : "升序，点击切换降序"} aria-label={sortOrder === "desc" ? "切换升序" : "切换降序"} onclick={() => { sortOrder = sortOrder === "desc" ? "asc" : "desc"; currentPage = 1; }}>
            {#if sortOrder === "desc"}<ArrowDown class="h-3.5 w-3.5" />{:else}<ArrowUp class="h-3.5 w-3.5" />{/if}
          </button>{/if}
        </div>
      </div>
    </div>

    {#if error || actionError}
      <div role="alert" class="flex items-start gap-3 rounded-[8px] bg-danger-soft p-3 text-[12px] text-danger-fg">
        <p class="min-w-0 flex-1 select-text break-words">{error || actionError}</p>
        {#if error}<button class="shrink-0 underline" onclick={onrefresh}>重试</button>{/if}
      </div>
    {/if}

    <div class="ui-card overflow-hidden" class:flex={$archiveViewMode === "calendar"} class:flex-col={$archiveViewMode === "calendar"} class:min-h-0={$archiveViewMode === "calendar"}>
      {#if loading && archives.length === 0}
        <div role="status" class="flex items-center justify-center gap-2 p-10 text-[12px] text-fg-muted"><Loader2 class="h-4 w-4 animate-spin motion-reduce:animate-none" />正在加载录播记录…</div>
      {:else if filteredArchives.length === 0}
        <div class="flex flex-col items-center justify-center gap-2 p-10 text-[12px] text-fg-muted">
          <History class="h-8 w-8 text-fg-faint" />
          <p>{error ? "暂时无法获取录播记录" : compact || selectedRoomKey ? "这个房间还没有录播记录。" : "还没有任何录播记录。"}</p>
        </div>
      {:else if $archiveViewMode === "calendar"}
        {#key selectedRoomKey}
          <ArchiveCalendarView archives={filtered} {rooms} {compact} {selectedArchives} {recordingKeys}
            ontoggle={toggleArchiveSelection} actions={calendarActions} bind:menuOpen={calendarMenuOpen}
            onshowlist={() => setArchiveViewMode("list")} />
        {/key}
      {:else}
        <div class="custom-scrollbar-light overflow-x-auto">
          <table class="w-full text-left" class:compact-table={compact}>
            <thead class="border-b border-line text-[11px] font-medium text-fg-muted">
              <tr>
                <th class="w-9 px-3 py-2"><input type="checkbox" class="ui-checkbox" aria-label="选择本页全部录播" checked={pageSelected} indeterminate={pagePartlySelected} onchange={selectAllArchives} /></th>
                <th class="whitespace-nowrap px-3 py-2 font-medium">直播时间</th>
                {#if !compact}<th class="px-3 py-2 font-medium">直播间</th>{/if}
                <th class="px-3 py-2 font-medium">标题</th>
                <th class="px-3 py-2 font-medium">时长</th>
                <th class="px-3 py-2 font-medium">大小</th>
                <th class="whitespace-nowrap px-3 py-2 font-medium">平均码率</th>
                <th class="sticky right-0 z-10 bg-surface px-3 py-2 font-medium">操作</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-line text-[12px]">
              {#each filteredArchives as archive (archiveKey(archive))}
                {@const roomUrl = liveRoomUrl(archive.platform, archive.room_id)}
                <tr class="group hover:bg-control-hover/50">
                  <td class="px-3 py-2.5"><input type="checkbox" class="ui-checkbox" aria-label={`选择录播：${archive.title || archive.live_id}`} checked={selectedArchives.has(archive.live_id)} onchange={() => toggleArchiveSelection(archive.live_id)} /></td>
                  <td class="whitespace-nowrap px-3 py-2.5 tabular-nums">
                    <div>{new Date(archive.created_at).toLocaleDateString()}</div>
                    <div class="mt-0.5 text-[11px] text-fg-muted">{new Date(archive.created_at).toLocaleTimeString()}</div>
                  </td>
                  {#if !compact}
                    <td class="px-3 py-2.5">
                      <div class="flex items-center gap-2"><PlatformIcon platform={archive.platform} class="h-4 w-4 shrink-0" />
                        {#if roomUrl}<a class="text-accent-fg hover:underline" href={roomUrl} target="_blank" rel="noopener noreferrer" title={`打开 ${platformLabel(archive.platform)} 直播间`}>{archive.room_id}</a>{:else}<span>{archive.room_id}</span>{/if}
                      </div>
                    </td>
                  {/if}
                  <td class="w-full max-w-[320px] px-3 py-2.5">
                    <div class="flex min-w-0 items-center gap-2.5">
                      {#if archive.cover}<ArchiveCover src={archive.cover} class="h-8 w-12 shrink-0 rounded-[5px]" />{/if}
                      <span class="truncate" title={archive.title}>{archive.title || "未命名录播"}</span>
                      {#if recordingKeys.has(archiveKey(archive))}<span class="ui-badge ui-badge-danger">录制中</span>{/if}
                    </div>
                  </td>
                  <td class="whitespace-nowrap px-3 py-2.5 tabular-nums">{formatTimecode(archive.length, 0)}</td>
                  <td class="whitespace-nowrap px-3 py-2.5 tabular-nums">{formatSize(archive.size)}</td>
                  <td class="whitespace-nowrap px-3 py-2.5 tabular-nums text-fg-muted">{formatBitrate(archive.size, archive.length)}</td>
                  <td class="sticky right-0 bg-surface px-3 py-2.5 group-hover:bg-control-hover">
                    {@render archiveActions(archive, false, () => {})}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </div>
  </div>
</BatchActionLayout>

{#if showDeleteConfirm}
  <div class="ui-modal-backdrop">
    <button class="absolute inset-0" aria-label="取消删除" disabled={deleting} onclick={cancelDelete}></button>
    <div use:trapFocus={{ active: pageVisible() }} role="dialog" aria-modal="true" aria-labelledby="archive-delete-title" tabindex="-1" class="ui-modal relative w-[400px] max-w-full">
      <div class="ui-modal-header"><h3 id="archive-delete-title">确认删除</h3></div>
      <div class="ui-modal-body space-y-2">
        <p class="text-[13px] text-fg-secondary">{archiveToDelete ? `确定要删除录播「${archiveToDelete.title}」吗？` : `确定要删除选中的 ${selectedArchives.size} 个录播吗？`}</p>
        <p class="text-[12px] text-danger-fg">此操作无法撤销。</p>
      </div>
      <div class="ui-modal-footer"><DeleteConfirmActions bind:busy={deleting} align="end" oncancel={cancelDelete} onconfirm={() => archiveToDelete ? deleteArchive(archiveToDelete) : deleteSelectedArchives()} /></div>
    </div>
  </div>
{/if}

<GenerateWholeClipModal bind:showModal={showWholeClipModal} archive={wholeClipArchive} roomId={wholeClipArchive?.room_id || ""} />
<ArchiveSummaryModal bind:showModal={showSummaryModal} archive={summaryArchive} onUpdated={updateSummaryStatus} />

<style>
  table { min-width: 960px; }
  table.compact-table { min-width: 780px; }
  .archive-action {
    display: inline-flex;
    flex-shrink: 0;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: 7px;
  }
  .archive-action:disabled { cursor: not-allowed; opacity: 0.4; }
  .archive-action:focus-visible { outline: 2px solid rgb(var(--c-accent)); outline-offset: 1px; }
</style>
