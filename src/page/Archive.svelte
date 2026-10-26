<script lang="ts">
  import { formatTimecode } from "../lib/time-format";
  import { formatSize } from "../lib/size-format";
  import { liveRoomUrl, platformLabel } from "../lib/platform";
  import { createStateRefresh, watchStateChanges } from "../lib/state-refresh";
  import { invoke, get_static_url } from "../lib/invoker";
  import type { RecordItem, RecordSummaryStatus } from "../lib/db";
  import { onMount } from "svelte";
  import { registerCommands } from "../lib/stores/commands";
  import {
    Play,
    Trash2,
    Calendar,
    Clock,
    HardDrive,
    RefreshCw,
    ChevronDown,
    ChevronUp,
    Video,
    Globe,
    Home,
    FileVideo,
    History,
    Sparkles,
    Loader2,
  } from "lucide-svelte";
  import BilibiliIcon from "../lib/components/BilibiliIcon.svelte";
  import DouyinIcon from "../lib/components/DouyinIcon.svelte";
  import DouyuIcon from "../lib/components/DouyuIcon.svelte";
  import KuaishouIcon from "../lib/components/KuaishouIcon.svelte";
  import HuyaIcon from "../lib/components/HuyaIcon.svelte";
  import TikTokIcon from "../lib/components/TikTokIcon.svelte";
  import TwitchIcon from "../lib/components/TwitchIcon.svelte";
  import YouTubeIcon from "../lib/components/YouTubeIcon.svelte";
  import { SvelteSet } from "svelte/reactivity";
  import GenerateWholeClipModal from "../lib/components/GenerateWholeClipModal.svelte";
  import ArchiveSummaryModal from "../lib/components/ArchiveSummaryModal.svelte";
  import PageTitlebar from "../lib/components/PageTitlebar.svelte";
  import DeleteConfirmActions from "../lib/components/DeleteConfirmActions.svelte";
  import type { RecorderInfo, RecorderList } from "src/lib/interface";

  let archives: RecordItem[] = [];
  let filteredArchives: RecordItem[] = $state([]);
  let loading = $state(false);
  let sortBy = $state("created_at");
  let sortOrder = $state("desc");
  type RoomOption = {
    id: string;
    label: string;
  };

  let selectedRoomId: string | null = $state(null);
  let roomOptions: RoomOption[] = $state([]);

  let selectedArchives = new SvelteSet<string>();
  let showDeleteConfirm = $state(false);
  let archiveToDelete: RecordItem | null = $state(null);

  // 生成完整录播相关状态
  let showWholeClipModal = $state(false);
  let wholeClipArchive: RecordItem | null = $state(null);
  let showSummaryModal = $state(false);
  let summaryArchive: RecordItem | null = $state(null);
  let summaryStatuses: Record<string, RecordSummaryStatus> = $state({});

  // 分页相关状态
  let currentPage = $state(1);
  let pageSize = $state(20);
  let totalPages = $state(1);
  let totalCount = $state(0);
  let isLoading = false;
  let loadError = $state("");

  // 页面大小选项
  const pageSizeOptions = [10, 20, 50, 100];

  // 所有数据缓存
  let allArchives = [];
  let allRooms: RecorderInfo[] = [];

  const stateRefresh = createStateRefresh(fetchArchives);

  onMount(() => {
    // 从本地存储恢复分页大小设置
    const savedPageSize = localStorage.getItem("archive-page-size");
    if (savedPageSize && pageSizeOptions.includes(parseInt(savedPageSize))) {
      pageSize = parseInt(savedPageSize);
    }

    return watchStateChanges(stateRefresh, ["records-changed", "recorders-changed"]);
  });

  onMount(() =>
    registerCommands([
      {
        id: "archive:refresh",
        group: "action",
        title: "刷新录播列表",
        detail: "录播",
        icon: RefreshCw,
        keywords: ["refresh archives", "reload"],
        page: "录播",
        run: loadArchives,
      },
    ]),
  );

  /**
   * 初始化加载所有录播数据
   */
  async function loadArchives() {
    await stateRefresh.run();
  }

  async function fetchArchives() {
    if (isLoading) return;

    isLoading = true;
    // Keep the existing list visible during background event reconciliation.
    loading = allArchives.length === 0;
    loadError = "";

    try {
      // 获取所有直播间列表
      const recorderList: RecorderList = await invoke("get_recorder_list");
      allRooms = recorderList.recorders || [];

      // 收集所有直播间，用账号名/直播间标题+直播间号展示
      roomOptions = allRooms
        .map((room: RecorderInfo) => {
          const id = room.room_info.room_id;
          const name =
            room.user_info?.user_name?.trim() ||
            room.room_info?.room_title?.trim() ||
            "";
          const label = name ? `${name} (${id})` : id;
          return { id, label };
        })
        .sort((a, b) => a.label.localeCompare(b.label));

      // 加载所有录播数据
      allArchives = [];
      for (const room of allRooms) {
        try {
          const roomArchives = await invoke<RecordItem[]>("get_archives", {
            roomId: room.room_info.room_id,
            offset: 0,
            limit: 100, // 每个直播间获取更多数据
          });

          // 处理封面
          for (const archive of roomArchives) {
            archive.cover = await get_static_url(
              "cache",
              `${archive.platform}/${archive.room_id}/${archive.live_id}/cover.jpg`
            );
          }

          allArchives = [...allArchives, ...roomArchives];
        } catch (error) {
          console.warn(`Failed to load archives for room ${room}:`, error);
        }
      }

      // 按创建时间排序
      allArchives.sort((a, b) => {
        return (
          new Date(b.created_at).getTime() - new Date(a.created_at).getTime()
        );
      });

      await loadSummaryStatuses();

      totalCount = allArchives.length;
      updatePagination();
    } catch (error) {
      console.error("Failed to load archives:", error);
      loadError = "加载失败，请重试";
    } finally {
      isLoading = false;
      loading = false;
    }
  }

  async function loadSummaryStatuses() {
    try {
      const statuses = await invoke<RecordSummaryStatus[]>(
        "get_archive_summary_statuses",
      );
      summaryStatuses = Object.fromEntries(
        statuses.map((status) => [summaryStatusKey(status), status]),
      );
    } catch (error) {
      console.warn("Failed to load Summary statuses:", error);
      summaryStatuses = {};
    }
  }

  function summaryStatusKey(
    record: Pick<RecordItem, "platform" | "room_id" | "live_id">,
  ) {
    return `${record.platform}:${record.room_id}:${record.live_id}`;
  }

  function getSummaryStatus(archive: RecordItem) {
    return summaryStatuses[summaryStatusKey(archive)]?.status;
  }

  function summaryButtonClass(archive: RecordItem) {
    switch (getSummaryStatus(archive)) {
      case "processing":
        return "hover:bg-accent-soft";
      case "success":
        return "hover:bg-success-soft";
      case "failed":
        return "hover:bg-danger-soft";
      default:
        return "hover:bg-accent-soft";
    }
  }

  function summaryIconClass(archive: RecordItem) {
    switch (getSummaryStatus(archive)) {
      case "processing":
        return "text-accent-fg";
      case "success":
        return "text-success-fg";
      case "failed":
        return "text-danger-fg";
      default:
        return "text-accent-fg";
    }
  }

  function summaryButtonTitle(archive: RecordItem) {
    switch (getSummaryStatus(archive)) {
      case "processing":
        return "Summary 生成中";
      case "success":
        return "查看 Summary";
      case "failed":
        return "Summary 生成失败，点击重试";
      default:
        return "生成 Summary";
    }
  }

  function updateSummaryStatus(status: RecordSummaryStatus) {
    summaryStatuses = {
      ...summaryStatuses,
      [summaryStatusKey(status)]: status,
    };
  }

  /**
   * 更新分页信息和当前页数据
   */
  function updatePagination() {
    totalPages = Math.ceil(totalCount / pageSize);
    if (currentPage > totalPages) {
      currentPage = totalPages || 1;
    }
    applyFilters();
  }

  /**
   * 上一页
   */
  function prevPage() {
    if (currentPage > 1) {
      currentPage--;
      applyFilters();
    }
  }

  /**
   * 下一页
   */
  function nextPage() {
    if (currentPage < totalPages) {
      currentPage++;
      applyFilters();
    }
  }

  /**
   * 更改页面大小
   */
  function changePageSize(newPageSize: number) {
    pageSize = newPageSize;
    currentPage = 1; // 重置到第一页

    // 保存到本地存储
    localStorage.setItem("archive-page-size", newPageSize.toString());

    applyFilters();
  }

  function applyFilters() {
    let filtered: RecordItem[] = [...allArchives];

    // Apply room filter
    if (selectedRoomId !== null) {
      filtered = filtered.filter(
        (archive) => archive.room_id === selectedRoomId
      );
    }

    // Apply sorting
    filtered.sort((a, b) => {
      let aValue: any, bValue: any;

      switch (sortBy) {
        case "title":
          aValue = a.title.toLowerCase();
          bValue = b.title.toLowerCase();
          break;
        case "length":
          aValue = a.length;
          bValue = b.length;
          break;
        case "size":
          aValue = a.size;
          bValue = b.size;
          break;
        case "created_at":
          aValue = new Date(a.created_at);
          bValue = new Date(b.created_at);
          break;
        case "room_id":
          aValue = a.room_id;
          bValue = b.room_id;
          break;
        case "platform":
          aValue = (a.platform || "").toLowerCase();
          bValue = (b.platform || "").toLowerCase();
          break;
        default:
          aValue = new Date(a.created_at);
          bValue = new Date(b.created_at);
      }

      if (sortOrder === "asc") {
        return aValue > bValue ? 1 : -1;
      } else {
        return aValue < bValue ? 1 : -1;
      }
    });

    // 更新总数和分页信息
    totalCount = filtered.length;
    totalPages = Math.ceil(totalCount / pageSize);

    // 确保当前页在有效范围内
    if (currentPage > totalPages && totalPages > 0) {
      currentPage = totalPages;
    }

    // Apply pagination
    const startIndex = (currentPage - 1) * pageSize;
    const endIndex = startIndex + pageSize;
    filteredArchives = filtered.slice(startIndex, endIndex);

    // 更新archives用于其他功能
    archives = filtered;
  }

  function formatDate(dateString: string) {
    const date = new Date(dateString);
    return date.toLocaleString();
  }

  function calcBitrate(size: number, duration: number) {
    if (!duration || duration <= 0 || !size || size <= 0) {
      return "0";
    }
    return ((size * 8) / duration / 1024).toFixed(0);
  }

  function getArchiveKey(archive: RecordItem) {
    return `${archive.platform}-${archive.room_id}-${archive.parent_id}-${archive.live_id}`;
  }

  function toggleSort(field: string) {
    if (sortBy === field) {
      sortOrder = sortOrder === "asc" ? "desc" : "asc";
    } else {
      sortBy = field;
      sortOrder = "asc";
    }
    applyFilters();
  }

  function toggleArchiveSelection(liveId: string) {
    if (selectedArchives.has(liveId)) {
      selectedArchives.delete(liveId);
    } else {
      selectedArchives.add(liveId);
    }
  }

  function selectAllArchives() {
    const currentArchives = filteredArchives;
    if (selectedArchives.size === currentArchives.length) {
      selectedArchives.clear();
    } else {
      currentArchives.forEach((archive) =>
        selectedArchives.add(archive.live_id)
      );
    }
  }

  async function deleteArchive(archive: RecordItem) {
    await invoke("delete_archive", {
      platform: archive.platform,
      roomId: archive.room_id,
      liveId: archive.live_id,
    });
    selectedArchives.delete(archive.live_id);
    await loadArchives();
    showDeleteConfirm = false;
    archiveToDelete = null;
  }

  async function deleteSelectedArchives() {
    // Deletion events can refresh the list while this batch is in flight.
    const targets = filteredArchives.filter((archive) => selectedArchives.has(archive.live_id));
    for (const archive of targets) {
      await invoke("delete_archive", {
        platform: archive.platform,
        roomId: archive.room_id,
        liveId: archive.live_id,
      });
      // A retry after partial failure must not delete completed items again.
      selectedArchives.delete(archive.live_id);
    }
    selectedArchives.clear();
    await loadArchives();
    showDeleteConfirm = false;
    archiveToDelete = null;
  }

  async function playArchive(archive: RecordItem) {
    try {
      await invoke("open_live", {
        platform: archive.platform,
        roomId: archive.room_id,
        liveId: archive.live_id,
      });
    } catch (error) {
      console.error("Failed to play archive:", error);
    }
  }

  function openWholeClipModal(archive: RecordItem) {
    wholeClipArchive = archive;
    showWholeClipModal = true;
  }

  function openSummaryModal(archive: RecordItem) {
    summaryArchive = archive;
    showSummaryModal = true;
  }

  function handleWholeClipGenerated() {
    // 生成完成后可以刷新列表或显示通知
    console.log("完整录播生成已开始");
  }
</script>

<PageTitlebar title="录播档案">
  {#snippet subtitle()}
    <span class="truncate text-[12px] text-fg-muted">
      管理所有直播间的录播记录，可以查看、播放和管理历史直播内容。
    </span>
  {/snippet}
  <button
    class="titlebar-icon-button"
    title="刷新"
    aria-label="刷新"
    onclick={loadArchives}
    disabled={loading}
  >
    <RefreshCw class="h-4 w-4 {loading ? 'animate-spin' : ''}" />
  </button>
</PageTitlebar>

<div class="flex-1 overflow-auto bg-canvas p-5 custom-scrollbar-light">
  <div class="space-y-4">
    <!-- 筛选和排序工具栏 -->
    <div class="ui-card space-y-3 p-3">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <!-- 左侧：筛选器和分页 -->
        <div class="flex items-center gap-2.5">
          <select
            bind:value={selectedRoomId}
            onchange={applyFilters}
            class="ui-input min-w-[160px]"
          >
            <option value={null}>所有直播间</option>
            {#each roomOptions as option}
              <option value={option.id}>{option.label}</option>
            {/each}
          </select>

          <!-- 分页控制 -->
          {#if totalCount > 0}
            <div
              class="flex h-[30px] items-center gap-2.5 rounded-[8px] bg-sunken px-2.5 text-[12px]"
            >
              <!-- 记录统计 -->
              <div class="flex items-center gap-1">
                <span class="font-medium tabular-nums text-accent-fg">
                  {totalCount}
                </span>
                <span class="text-fg-muted">条记录</span>
              </div>

              <!-- 分隔线 -->
              <div class="h-3.5 w-px bg-line-strong"></div>

              <!-- 每页大小选择 -->
              <div class="flex items-center gap-1.5">
                <span class="text-fg-muted">每页</span>
                <select
                  bind:value={pageSize}
                  onchange={() => changePageSize(pageSize)}
                  class="ui-input h-[22px] min-w-[56px] rounded-[6px] pl-2 tabular-nums"
                >
                  {#each pageSizeOptions as size}
                    <option value={size}>{size}</option>
                  {/each}
                </select>
                <span class="text-fg-muted">条</span>
              </div>

              <!-- 分页导航 -->
              {#if totalPages > 1}
                <!-- 分隔线 -->
                <div class="h-3.5 w-px bg-line-strong"></div>

                <div class="flex items-center gap-1">
                  <button
                    class="inline-flex h-[22px] w-[22px] items-center justify-center rounded-[6px] text-fg-muted transition-colors hover:bg-control-hover hover:text-fg disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-fg-muted"
                    onclick={prevPage}
                    disabled={currentPage === 1}
                    title="上一页"
                  >
                    <ChevronUp class="h-4 w-4 rotate-[-90deg]" />
                  </button>

                  <div
                    class="flex h-[22px] min-w-[56px] items-center justify-center rounded-[6px] bg-selected px-2 tabular-nums shadow-sm"
                  >
                    <span class="font-medium text-fg">
                      {currentPage}
                    </span>
                    <span class="mx-1 text-fg-faint">/</span>
                    <span class="text-fg-muted">
                      {totalPages}
                    </span>
                  </div>

                  <button
                    class="inline-flex h-[22px] w-[22px] items-center justify-center rounded-[6px] text-fg-muted transition-colors hover:bg-control-hover hover:text-fg disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-fg-muted"
                    onclick={nextPage}
                    disabled={currentPage === totalPages}
                    title="下一页"
                  >
                    <ChevronDown class="h-4 w-4 rotate-[-90deg]" />
                  </button>
                </div>
              {/if}
            </div>
          {/if}
        </div>

        <!-- 右侧：排序按钮 -->
        <div class="flex items-center gap-1.5">
          <span class="mr-0.5 text-[12px] text-fg-muted">排序:</span>
          <button
            class="ui-btn {sortBy === 'room_id' ? 'ui-btn-active' : ''}"
            onclick={() => toggleSort("room_id")}
          >
            直播间号
            {#if sortBy === "room_id"}
              {#if sortOrder === "asc"}
                <ChevronUp class="h-3 w-3" />
              {:else}
                <ChevronDown class="h-3 w-3" />
              {/if}
            {/if}
          </button>
          <button
            class="ui-btn {sortBy === 'title' ? 'ui-btn-active' : ''}"
            onclick={() => toggleSort("title")}
          >
            标题
            {#if sortBy === "title"}
              {#if sortOrder === "asc"}
                <ChevronUp class="h-3 w-3" />
              {:else}
                <ChevronDown class="h-3 w-3" />
              {/if}
            {/if}
          </button>
          <button
            class="ui-btn {sortBy === 'length' ? 'ui-btn-active' : ''}"
            onclick={() => toggleSort("length")}
          >
            时长
            {#if sortBy === "length"}
              {#if sortOrder === "asc"}
                <ChevronUp class="h-3 w-3" />
              {:else}
                <ChevronDown class="h-3 w-3" />
              {/if}
            {/if}
          </button>
          <button
            class="ui-btn {sortBy === 'size' ? 'ui-btn-active' : ''}"
            onclick={() => toggleSort("size")}
          >
            大小
            {#if sortBy === "size"}
              {#if sortOrder === "asc"}
                <ChevronUp class="h-3 w-3" />
              {:else}
                <ChevronDown class="h-3 w-3" />
              {/if}
            {/if}
          </button>
          <button
            class="ui-btn {sortBy === 'created_at' ? 'ui-btn-active' : ''}"
            onclick={() => toggleSort("created_at")}
          >
            创建时间
            {#if sortBy === "created_at"}
              {#if sortOrder === "asc"}
                <ChevronUp class="h-3 w-3" />
              {:else}
                <ChevronDown class="h-3 w-3" />
              {/if}
            {/if}
          </button>
        </div>
      </div>

      <!-- 批量操作栏 -->
      {#if selectedArchives.size > 0}
        <div
          class="flex items-center justify-between border-t border-line pt-3"
        >
          <div class="flex items-center gap-2">
            <div class="h-2 w-2 rounded-full bg-warning"></div>
            <span class="text-[12px] font-medium text-warning-fg">
              已选择 {selectedArchives.size} 项
            </span>
          </div>
          <button
            class="ui-btn ui-btn-danger"
            onclick={() => {
              showDeleteConfirm = true;
              archiveToDelete = null;
            }}
          >
            <Trash2 class="h-3.5 w-3.5" />
            <span>删除选中</span>
          </button>
        </div>
      {/if}
    </div>

    <!-- Archive List -->
    <div class="ui-card overflow-hidden">
      {#if loadError}
        <div
          class="flex flex-col items-center justify-center gap-3 p-12 text-fg-muted"
        >
          <div class="text-[14px] font-medium text-danger-fg">加载失败</div>
          <p class="text-[12px]">{loadError}</p>
          <button class="ui-btn ui-btn-primary" onclick={loadArchives}>
            重试
          </button>
        </div>
      {:else if loading}
        <div
          class="flex flex-col items-center justify-center gap-3 p-12 text-fg-muted"
        >
          <RefreshCw class="h-6 w-6 animate-spin text-fg-faint" />
          <span class="text-[13px]">加载录播列表中...</span>
        </div>
      {:else if filteredArchives.length === 0 && !loading}
        <div
          class="flex flex-col items-center justify-center gap-3 p-12 text-fg-muted"
        >
          <History class="h-10 w-10 text-fg-faint" />
          <h3 class="text-[14px] font-medium text-fg">
            暂无录播
          </h3>
          <p class="text-[12px]">
            {selectedRoomId !== null
              ? "该直播间还没有录播记录"
              : "还没有任何录播记录"}
          </p>
        </div>
      {:else}
        <div class="overflow-x-auto custom-scrollbar-light">
          <table class="w-full">
            <thead>
              <tr class="border-b border-line">
                <th class="w-12 px-4 py-2.5 text-left">
                  <input
                    type="checkbox"
                    checked={selectedArchives.size ===
                      filteredArchives.length && filteredArchives.length > 0}
                    onchange={selectAllArchives}
                    class="ui-checkbox"
                  />
                </th>
                <th
                  class="px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted"
                  >直播时间</th
                >
                <th
                  class="px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted"
                  >直播间</th
                >
                <th
                  class="px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted"
                  >标题</th
                >
                <th
                  class="px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted"
                  >时长</th
                >
                <th
                  class="px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted"
                  >大小</th
                >
                <th
                  class="px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted"
                  >码率</th
                >
                <th
                  class="px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted"
                  >操作</th
                >
              </tr>
            </thead>
            <tbody class="divide-y divide-line">
              {#each filteredArchives as archive (getArchiveKey(archive))}
                {@const roomUrl = liveRoomUrl(archive.platform, archive.room_id)}
                <tr class="group transition-colors hover:bg-control-hover/50">
                  <td class="px-4 py-2.5">
                    <input
                      type="checkbox"
                      checked={selectedArchives.has(archive.live_id)}
                      onchange={() => toggleArchiveSelection(archive.live_id)}
                      class="ui-checkbox"
                    />
                  </td>

                  <td class="px-4 py-2.5">
                    <div class="flex flex-col">
                      <span class="text-[13px] tabular-nums text-fg"
                        >{formatDate(archive.created_at).split(" ")[0]}</span
                      >
                      <span class="text-[11px] tabular-nums text-fg-muted"
                        >{formatDate(archive.created_at).split(" ")[1]}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-2.5">
                    <div class="flex items-center gap-2">
                      {#if archive.platform === "bilibili"}
                        <BilibiliIcon class="w-4 h-4" />
                      {:else if archive.platform === "douyin"}
                        <DouyinIcon class="w-4 h-4" />
                      {:else if archive.platform === "douyu"}
                        <DouyuIcon class="w-4 h-4" />
                      {:else if archive.platform === "kuaishou"}
                        <KuaishouIcon class="w-4 h-4" />
                      {:else if archive.platform === "huya"}
                        <HuyaIcon class="w-4 h-4" />
                      {:else if archive.platform === "tiktok"}
                        <TikTokIcon class="w-5 h-5" />
                      {:else if archive.platform === "twitch"}
                        <TwitchIcon class="w-5 h-5 text-purple-600" />
                      {:else if archive.platform === "youtube"}
                        <YouTubeIcon class="w-4 h-4" />
                      {:else}
                        <Globe class="h-4 w-4 text-fg-faint" />
                      {/if}
                      {#if roomUrl}
                        <a
                          href={roomUrl}
                          target="_blank"
                          rel="noopener noreferrer"
                          class="text-[13px] text-accent-fg underline-offset-2 hover:underline"
                          title={`打开 ${platformLabel(archive.platform)} 直播间`}
                        >
                          {archive.room_id}
                        </a>
                      {:else}
                        <span class="text-[13px] text-fg"
                          >{archive.room_id}</span
                        >
                      {/if}
                    </div>
                  </td>

                  <td class="px-4 py-2.5">
                    <div class="flex items-center gap-3">
                      {#if archive.cover}
                        <img
                          src={archive.cover}
                          alt="封面"
                          class="h-8 w-12 flex-shrink-0 rounded-[5px] object-cover"
                        />
                      {/if}
                      <span class="truncate text-[13px] text-fg"
                        >{archive.title}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-2.5">
                    <div class="flex items-center gap-1.5">
                      <Clock class="h-3.5 w-3.5 text-fg-faint" />
                      <span class="text-[13px] tabular-nums text-fg"
                        >{formatTimecode(archive.length, 0)}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-2.5">
                    <div class="flex items-center gap-1.5">
                      <HardDrive class="h-3.5 w-3.5 text-fg-faint" />
                      <span class="text-[13px] tabular-nums text-fg"
                        >{formatSize(archive.size)}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-2.5">
                    <span class="text-[12px] tabular-nums text-fg-muted"
                      >{calcBitrate(archive.size, archive.length)} Kbps</span
                    >
                  </td>

                  <td class="px-4 py-2.5">
                    <div class="flex items-center gap-1">
                      <button
                        class="rounded-[7px] p-1.5 transition-colors hover:bg-accent-soft"
                        title="预览录播"
                        onclick={() => playArchive(archive)}
                      >
                        <Play class="h-4 w-4 text-accent-fg" />
                      </button>
                      <button
                        class="rounded-[7px] p-1.5 transition-colors hover:bg-accent-soft"
                        title="生成完整切片"
                        onclick={() => openWholeClipModal(archive)}
                      >
                        <FileVideo class="h-4 w-4 text-accent-fg" />
                      </button>
                      <button
                        class="rounded-[7px] p-1.5 transition-colors {summaryButtonClass(archive)}"
                        title={summaryButtonTitle(archive)}
                        onclick={() => openSummaryModal(archive)}
                      >
                        {#if getSummaryStatus(archive) === "processing"}
                          <Loader2 class="h-4 w-4 animate-spin {summaryIconClass(archive)}" />
                        {:else}
                          <Sparkles class="h-4 w-4 {summaryIconClass(archive)}" />
                        {/if}
                      </button>
                      <button
                        class="rounded-[7px] p-1.5 transition-colors hover:bg-danger-soft"
                        title="删除记录"
                        onclick={() => {
                          archiveToDelete = archive;
                          showDeleteConfirm = true;
                        }}
                      >
                        <Trash2 class="h-4 w-4 text-danger-fg" />
                      </button>
                    </div>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </div>
  </div>
</div>

<!-- Delete Confirmation Modal -->
{#if showDeleteConfirm}
  <div class="ui-modal-backdrop">
    <div class="ui-modal w-[400px]">
      <div class="ui-modal-header">
        <h3>确认删除</h3>
      </div>
      <div class="ui-modal-body space-y-2">
        <p class="text-[13px] text-fg-secondary">
          {#if archiveToDelete}
            确定要删除录播 "{archiveToDelete.title}" 吗？
          {:else}
            确定要删除选中的 {selectedArchives.size} 个录播吗？
          {/if}
        </p>
        <p class="text-[12px] text-danger-fg">此操作无法撤销。</p>
      </div>
      <div class="ui-modal-footer">
        <DeleteConfirmActions
          align="end"
          oncancel={() => {
            showDeleteConfirm = false;
            archiveToDelete = null;
          }}
          onconfirm={() => archiveToDelete ? deleteArchive(archiveToDelete) : deleteSelectedArchives()}
        />
      </div>
    </div>
  </div>
{/if}

<!-- 生成完整录播Modal -->
<GenerateWholeClipModal
  bind:showModal={showWholeClipModal}
  archive={wholeClipArchive}
  roomId={wholeClipArchive?.room_id || ""}
  onGenerated={handleWholeClipGenerated}
/>

<ArchiveSummaryModal
  bind:showModal={showSummaryModal}
  archive={summaryArchive}
  onUpdated={updateSummaryStatus}
/>

<style>
  /* fixed icon size in tables */
  :global(.table-icon) {
    width: 1rem; /* 16px, same as Tailwind w-4 */
    height: 1rem; /* 16px, same as Tailwind h-4 */
    flex: 0 0 auto;
  }
</style>
