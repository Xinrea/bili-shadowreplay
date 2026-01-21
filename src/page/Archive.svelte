<script lang="ts">
  import { invoke, get_static_url } from "../lib/invoker";
  import type { RecordItem } from "../lib/db";
  import { onMount } from "svelte";
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
  } from "lucide-svelte";
  import BilibiliIcon from "../lib/components/BilibiliIcon.svelte";
  import DouyinIcon from "../lib/components/DouyinIcon.svelte";
  import KuaishouIcon from "../lib/components/KuaishouIcon.svelte";
  import HuyaIcon from "../lib/components/HuyaIcon.svelte";
  import TikTokIcon from "../lib/components/TikTokIcon.svelte";
  import GenerateWholeClipModal from "../lib/components/GenerateWholeClipModal.svelte";
  import type { RecorderInfo, RecorderList } from "src/lib/interface";

  let archives: RecordItem[] = [];
  let filteredArchives: RecordItem[] = [];
  let loading = false;
  let sortBy = "created_at";
  let sortOrder = "desc";
  type RoomOption = {
    id: string;
    label: string;
  };

  let selectedRoomId: string | null = null;
  let roomOptions: RoomOption[] = [];

  let selectedArchives: Set<string> = new Set();
  let showDeleteConfirm = false;
  let archiveToDelete: RecordItem | null = null;

  // 生成完整录播相关状态
  let showWholeClipModal = false;
  let wholeClipArchive: RecordItem | null = null;

  // 分页相关状态
  let currentPage = 1;
  let pageSize = 20;
  let totalPages = 1;
  let totalCount = 0;
  let isLoading = false;
  let loadError = "";

  // 页面大小选项
  const pageSizeOptions = [10, 20, 50, 100];

  // 所有数据缓存
  let allArchives = [];
  let allRooms: RecorderInfo[] = [];

  onMount(async () => {
    // 从本地存储恢复分页大小设置
    const savedPageSize = localStorage.getItem("archive-page-size");
    if (savedPageSize && pageSizeOptions.includes(parseInt(savedPageSize))) {
      pageSize = parseInt(savedPageSize);
    }

    await loadArchives();
  });

  /**
   * 初始化加载所有录播数据
   */
  async function loadArchives() {
    if (isLoading) return;

    isLoading = true;
    loading = true;
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
   * 跳转到指定页面
   */
  function goToPage(page: number) {
    if (page < 1 || page > totalPages || page === currentPage) return;
    currentPage = page;
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

  function formatSize(size: number) {
    if (size < 1024) {
      return `${size} B`;
    } else if (size < 1024 * 1024) {
      return `${(size / 1024).toFixed(2)} KiB`;
    } else if (size < 1024 * 1024 * 1024) {
      return `${(size / 1024 / 1024).toFixed(2)} MiB`;
    } else {
      return `${(size / 1024 / 1024 / 1024).toFixed(2)} GiB`;
    }
  }

  function formatDuration(seconds: number) {
    seconds = Math.round(seconds);
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    const secs = seconds % 60;

    if (hours > 0) {
      return `${hours}:${minutes.toString().padStart(2, "0")}:${secs.toString().padStart(2, "0")}`;
    } else {
      return `${minutes}:${secs.toString().padStart(2, "0")}`;
    }
  }

  function formatDate(dateString: string) {
    const date = new Date(dateString);
    return date.toLocaleString();
  }

  function formatPlatform(platform: string) {
    switch (platform.toLowerCase()) {
      case "bilibili":
        return "B站";
      case "douyin":
        return "抖音";
      case "huya":
        return "虎牙";
      case "kuaishou":
        return "快手";
      case "tiktok":
        return "TikTok";
      case "youtube":
        return "YouTube";
      default:
        return platform;
    }
  }

  function getRoomUrl(platform: string, roomId: string) {
    if (roomId.startsWith("http")) {
      return roomId;
    }
    switch (platform.toLowerCase()) {
      case "bilibili":
        return `https://live.bilibili.com/${roomId}`;
      case "douyin":
        return `https://live.douyin.com/${roomId}`;
      case "huya":
        return `https://www.huya.com/${roomId}`;
      case "kuaishou":
        return `https://live.kuaishou.com/u/${roomId}`;
      case "tiktok":
        return `https://www.tiktok.com/${roomId}/live`;
      case "youtube":
        return `https://www.youtube.com/channel/${roomId}`;
      default:
        return null;
    }
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
    selectedArchives = selectedArchives; // Trigger reactivity
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
    selectedArchives = selectedArchives; // Trigger reactivity
  }

  async function deleteArchive(archive: RecordItem) {
    try {
      await invoke("delete_archive", {
        platform: archive.platform,
        roomId: archive.room_id,
        liveId: archive.live_id,
      });
      await loadArchives();
      showDeleteConfirm = false;
      archiveToDelete = null;
    } catch (error) {
      console.error("Failed to delete archive:", error);
    }
  }

  async function deleteSelectedArchives() {
    try {
      for (const liveId of selectedArchives) {
        const archive = filteredArchives.find((a) => a.live_id === liveId);
        if (archive) {
          await invoke("delete_archive", {
            platform: archive.platform,
            roomId: archive.room_id,
            liveId: archive.live_id,
          });
        }
      }
      selectedArchives.clear();
      await loadArchives();
      showDeleteConfirm = false;
      archiveToDelete = null;
    } catch (error) {
      console.error("Failed to delete selected archives:", error);
    }
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

  function handleWholeClipGenerated() {
    // 生成完成后可以刷新列表或显示通知
    console.log("完整录播生成已开始");
  }
</script>

<div class="flex-1 p-6 overflow-auto custom-scrollbar-light bg-gray-50 dark:bg-black">
  <div class="space-y-6">
    <!-- Header -->
    <div class="flex justify-between items-center">
      <div class="space-y-1">
        <h1 class="text-2xl font-semibold text-gray-900 dark:text-white">
          录播档案
        </h1>
        <p class="text-sm text-gray-500 dark:text-gray-400">
          管理所有直播间的录播记录，可以查看、播放和管理历史直播内容。
        </p>
      </div>

      <div class="flex items-center space-x-3">
        <button
          class="px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors flex items-center space-x-2 disabled:opacity-50 disabled:cursor-not-allowed"
          on:click={loadArchives}
          disabled={loading}
        >
          <RefreshCw
            class="w-4 h-4 text-white {loading ? 'animate-spin' : ''}"
          />
          <span>刷新</span>
        </button>
      </div>
    </div>

    <!-- 筛选和排序工具栏 -->
    <div
      class="p-4 rounded-xl bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 space-y-4"
    >
      <div class="flex justify-between items-center flex-wrap gap-4">
        <!-- 左侧：筛选器和分页 -->
        <div class="flex space-x-3">
          <select
            bind:value={selectedRoomId}
            on:change={applyFilters}
            class="px-3 py-2 bg-gray-100 dark:bg-gray-700/50 border border-gray-200 dark:border-gray-600 rounded-lg text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500 dark:focus:ring-blue-400 cursor-pointer"
          >
            <option value={null}>所有直播间</option>
            {#each roomOptions as option}
              <option value={option.id}>{option.label}</option>
            {/each}
          </select>

          <!-- 分页控制 -->
          {#if totalCount > 0}
            <div
              class="flex items-center space-x-3 px-4 py-2 bg-gray-50 dark:bg-gray-800/50 rounded-lg border border-gray-200 dark:border-gray-600"
            >
              <!-- 记录统计 -->
              <div class="flex items-center space-x-1">
                <span
                  class="text-sm font-medium text-blue-600 dark:text-blue-400"
                >
                  {totalCount}
                </span>
                <span class="text-sm text-gray-500 dark:text-gray-400"
                  >条记录</span
                >
              </div>

              <!-- 分隔线 -->
              <div class="h-4 w-px bg-gray-300 dark:bg-gray-600"></div>

              <!-- 每页大小选择 -->
              <div class="flex items-center space-x-2">
                <span class="text-sm text-gray-600 dark:text-gray-400"
                  >每页</span
                >
                <select
                  bind:value={pageSize}
                  on:change={() => changePageSize(pageSize)}
                  class="px-2 py-1 text-sm bg-white dark:bg-gray-700 border border-gray-300 dark:border-gray-500 rounded-md text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-blue-500 cursor-pointer min-w-[50px]"
                >
                  {#each pageSizeOptions as size}
                    <option value={size}>{size}</option>
                  {/each}
                </select>
                <span class="text-sm text-gray-600 dark:text-gray-400">条</span>
              </div>

              <!-- 分页导航 -->
              {#if totalPages > 1}
                <!-- 分隔线 -->
                <div class="h-4 w-px bg-gray-300 dark:bg-gray-600"></div>

                <div class="flex items-center space-x-2">
                  <button
                    class="p-1.5 text-gray-500 dark:text-gray-400 hover:text-blue-600 dark:hover:text-blue-400 hover:bg-white dark:hover:bg-gray-700 rounded-md transition-all duration-200 disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-transparent"
                    on:click={prevPage}
                    disabled={currentPage === 1}
                    title="上一页"
                  >
                    <ChevronUp class="w-4 h-4 rotate-[-90deg]" />
                  </button>

                  <div
                    class="flex items-center px-2 py-1 bg-white dark:bg-gray-700 rounded-md border border-gray-200 dark:border-gray-500 min-w-[60px] justify-center"
                  >
                    <span
                      class="text-sm font-medium text-gray-700 dark:text-gray-300"
                    >
                      {currentPage}
                    </span>
                    <span class="text-sm text-gray-400 dark:text-gray-500 mx-1"
                      >/</span
                    >
                    <span class="text-sm text-gray-500 dark:text-gray-400">
                      {totalPages}
                    </span>
                  </div>

                  <button
                    class="p-1.5 text-gray-500 dark:text-gray-400 hover:text-blue-600 dark:hover:text-blue-400 hover:bg-white dark:hover:bg-gray-700 rounded-md transition-all duration-200 disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-transparent"
                    on:click={nextPage}
                    disabled={currentPage === totalPages}
                    title="下一页"
                  >
                    <ChevronDown class="w-4 h-4 rotate-[-90deg]" />
                  </button>
                </div>
              {/if}
            </div>
          {/if}
        </div>

        <!-- 右侧：排序按钮 -->
        <div class="flex items-center space-x-2">
          <span class="text-sm text-gray-600 dark:text-gray-400">排序:</span>
          <button
            class="px-3 py-1.5 text-sm font-medium rounded-lg transition-colors {sortBy ===
            'room_id'
              ? 'bg-blue-500 text-white'
              : 'bg-gray-100 dark:bg-gray-700/50 text-gray-700 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600'}"
            on:click={() => toggleSort("room_id")}
          >
            直播间号
            {#if sortBy === "room_id"}
              {#if sortOrder === "asc"}
                <ChevronUp class="w-3 h-3 inline ml-1" />
              {:else}
                <ChevronDown class="w-3 h-3 inline ml-1" />
              {/if}
            {/if}
          </button>
          <button
            class="px-3 py-1.5 text-sm font-medium rounded-lg transition-colors {sortBy ===
            'title'
              ? 'bg-blue-500 text-white'
              : 'bg-gray-100 dark:bg-gray-700/50 text-gray-700 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600'}"
            on:click={() => toggleSort("title")}
          >
            标题
            {#if sortBy === "title"}
              {#if sortOrder === "asc"}
                <ChevronUp class="w-3 h-3 inline ml-1" />
              {:else}
                <ChevronDown class="w-3 h-3 inline ml-1" />
              {/if}
            {/if}
          </button>
          <button
            class="px-3 py-1.5 text-sm font-medium rounded-lg transition-colors {sortBy ===
            'length'
              ? 'bg-blue-500 text-white'
              : 'bg-gray-100 dark:bg-gray-700/50 text-gray-700 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600'}"
            on:click={() => toggleSort("length")}
          >
            时长
            {#if sortBy === "length"}
              {#if sortOrder === "asc"}
                <ChevronUp class="w-3 h-3 inline ml-1" />
              {:else}
                <ChevronDown class="w-3 h-3 inline ml-1" />
              {/if}
            {/if}
          </button>
          <button
            class="px-3 py-1.5 text-sm font-medium rounded-lg transition-colors {sortBy ===
            'size'
              ? 'bg-blue-500 text-white'
              : 'bg-gray-100 dark:bg-gray-700/50 text-gray-700 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600'}"
            on:click={() => toggleSort("size")}
          >
            大小
            {#if sortBy === "size"}
              {#if sortOrder === "asc"}
                <ChevronUp class="w-3 h-3 inline ml-1" />
              {:else}
                <ChevronDown class="w-3 h-3 inline ml-1" />
              {/if}
            {/if}
          </button>
          <button
            class="px-3 py-1.5 text-sm font-medium rounded-lg transition-colors {sortBy ===
            'created_at'
              ? 'bg-blue-500 text-white'
              : 'bg-gray-100 dark:bg-gray-700/50 text-gray-700 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600'}"
            on:click={() => toggleSort("created_at")}
          >
            创建时间
            {#if sortBy === "created_at"}
              {#if sortOrder === "asc"}
                <ChevronUp class="w-3 h-3 inline ml-1" />
              {:else}
                <ChevronDown class="w-3 h-3 inline ml-1" />
              {/if}
            {/if}
          </button>
        </div>
      </div>

      <!-- 批量操作栏 -->
      {#if selectedArchives.size > 0}
        <div
          class="flex justify-between items-center pt-4 border-t border-gray-200 dark:border-gray-700"
        >
          <div class="flex items-center space-x-2">
            <div class="w-2 h-2 bg-amber-500 rounded-full"></div>
            <span
              class="text-sm text-amber-700 dark:text-amber-300 font-medium"
            >
              已选择 {selectedArchives.size} 项
            </span>
          </div>
          <button
            class="px-4 py-2 bg-red-600 hover:bg-red-700 text-white rounded-lg transition-colors flex items-center space-x-2"
            on:click={() => {
              showDeleteConfirm = true;
              archiveToDelete = null;
            }}
          >
            <Trash2 class="w-4 h-4" />
            <span>删除选中</span>
          </button>
        </div>
      {/if}
    </div>

    <!-- Archive List -->
    <div
      class="bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 rounded-xl overflow-hidden"
    >
      {#if loadError}
        <div
          class="flex flex-col items-center justify-center p-12 space-y-4 text-gray-500 dark:text-gray-400"
        >
          <div class="text-red-500 dark:text-red-400 text-lg">加载失败</div>
          <p class="text-sm">{loadError}</p>
          <button
            class="px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
            on:click={loadArchives}
          >
            重试
          </button>
        </div>
      {:else if loading}
        <div
          class="flex flex-col items-center justify-center p-12 space-y-4 text-gray-500 dark:text-gray-400"
        >
          <RefreshCw class="w-8 h-8 animate-spin" />
          <span>加载录播列表中...</span>
        </div>
      {:else if filteredArchives.length === 0 && !loading}
        <div
          class="flex flex-col items-center justify-center p-12 space-y-4 text-gray-500 dark:text-gray-400"
        >
          <History class="w-12 h-12" />
          <h3 class="text-lg font-medium text-gray-900 dark:text-white">
            暂无录播
          </h3>
          <p class="text-sm">
            {selectedRoomId !== null
              ? "该直播间还没有录播记录"
              : "还没有任何录播记录"}
          </p>
        </div>
      {:else}
        <div class="overflow-x-auto custom-scrollbar-light">
          <table class="w-full">
            <thead>
              <tr class="border-b border-gray-200 dark:border-gray-700/50">
                <th class="px-4 py-3 text-left w-12">
                  <input
                    type="checkbox"
                    checked={selectedArchives.size ===
                      filteredArchives.length && filteredArchives.length > 0}
                    on:change={selectAllArchives}
                    class="rounded border-gray-300 dark:border-gray-600"
                  />
                </th>
                <th
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400"
                  >直播时间</th
                >
                <th
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400"
                  >直播间</th
                >
                <th
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400"
                  >标题</th
                >
                <th
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400"
                  >时长</th
                >
                <th
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400"
                  >大小</th
                >
                <th
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400"
                  >码率</th
                >
                <th
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400"
                  >操作</th
                >
              </tr>
            </thead>
            <tbody class="divide-y divide-gray-200 dark:divide-gray-700/50">
              {#each filteredArchives as archive (getArchiveKey(archive))}
                <tr
                  class="group hover:bg-[#f5f5f7] dark:hover:bg-[#3a3a3c] transition-colors"
                >
                  <td class="px-4 py-3">
                    <input
                      type="checkbox"
                      checked={selectedArchives.has(archive.live_id)}
                      on:change={() => toggleArchiveSelection(archive.live_id)}
                      class="rounded border-gray-300 dark:border-gray-600"
                    />
                  </td>

                  <td class="px-4 py-3">
                    <div class="flex flex-col">
                      <span class="text-sm text-gray-900 dark:text-white"
                        >{formatDate(archive.created_at).split(" ")[0]}</span
                      >
                      <span class="text-xs text-gray-500 dark:text-gray-400"
                        >{formatDate(archive.created_at).split(" ")[1]}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-3">
                    <div class="flex items-center space-x-2">
                      {#if archive.platform === "bilibili"}
                        <BilibiliIcon class="w-4 h-4" />
                      {:else if archive.platform === "douyin"}
                        <DouyinIcon class="w-4 h-4" />
                      {:else if archive.platform === "kuaishou"}
                        <KuaishouIcon class="w-4 h-4" />
                      {:else if archive.platform === "huya"}
                        <HuyaIcon class="w-4 h-4" />
                      {:else if archive.platform === "tiktok"}
                        <TikTokIcon class="w-5 h-5" />
                      {:else}
                        <Globe class="w-4 h-4 text-gray-400" />
                      {/if}
                      {#if getRoomUrl(archive.platform, archive.room_id)}
                        <a
                          href={getRoomUrl(archive.platform, archive.room_id)}
                          target="_blank"
                          rel="noopener noreferrer"
                          class="text-blue-500 hover:text-blue-700 text-sm"
                          title={`打开 ${formatPlatform(archive.platform)} 直播间`}
                        >
                          {archive.room_id}
                        </a>
                      {:else}
                        <span class="text-sm text-gray-900 dark:text-white"
                          >{archive.room_id}</span
                        >
                      {/if}
                    </div>
                  </td>

                  <td class="px-4 py-3">
                    <div class="flex items-center space-x-3">
                      {#if archive.cover}
                        <img
                          src={archive.cover}
                          alt="封面"
                          class="w-12 h-8 rounded object-cover flex-shrink-0"
                        />
                      {/if}
                      <span
                        class="text-sm text-gray-900 dark:text-white truncate"
                        >{archive.title}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-3">
                    <div class="flex items-center space-x-2">
                      <Clock class="w-4 h-4 text-gray-400" />
                      <span class="text-sm text-gray-900 dark:text-white"
                        >{formatDuration(archive.length)}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-3">
                    <div class="flex items-center space-x-2">
                      <HardDrive class="w-4 h-4 text-gray-400" />
                      <span class="text-sm text-gray-900 dark:text-white"
                        >{formatSize(archive.size)}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-3">
                    <span class="text-sm text-gray-500 dark:text-gray-400"
                      >{calcBitrate(archive.size, archive.length)} Kbps</span
                    >
                  </td>

                  <td class="px-4 py-3">
                    <div class="flex items-center space-x-2">
                      <button
                        class="p-1.5 rounded-lg hover:bg-blue-500/10 transition-colors"
                        title="预览录播"
                        on:click={() => playArchive(archive)}
                      >
                        <Play class="w-4 h-4 text-blue-500" />
                      </button>
                      <button
                        class="p-1.5 rounded-lg hover:bg-blue-500/10 transition-colors"
                        title="生成完整切片"
                        on:click={() => openWholeClipModal(archive)}
                      >
                        <FileVideo class="w-4 h-4 text-blue-500" />
                      </button>
                      <button
                        class="p-1.5 rounded-lg hover:bg-red-500/10 transition-colors"
                        title="删除记录"
                        on:click={() => {
                          archiveToDelete = archive;
                          showDeleteConfirm = true;
                        }}
                      >
                        <Trash2 class="w-4 h-4 text-red-500" />
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
  <div
    class="fixed inset-0 bg-black/20 dark:bg-black/40 backdrop-blur-sm z-50 flex items-center justify-center"
  >
    <div
      class="mac-modal w-[400px] bg-white dark:bg-[#323234] rounded-xl shadow-xl overflow-hidden"
    >
      <div class="p-6 space-y-4">
        <div class="text-center space-y-2">
          <h3 class="text-base font-medium text-gray-900 dark:text-white">
            确认删除
          </h3>
          <p class="text-sm text-gray-500 dark:text-gray-400">
            {#if archiveToDelete}
              确定要删除录播 "{archiveToDelete.title}" 吗？
            {:else}
              确定要删除选中的 {selectedArchives.size} 个录播吗？
            {/if}
          </p>
          <p class="text-xs text-red-600 dark:text-red-500">此操作无法撤销。</p>
        </div>
        <div class="flex justify-center space-x-3">
          <button
            class="w-24 px-4 py-2 text-sm font-medium text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-600 rounded-lg transition-colors"
            on:click={() => {
              showDeleteConfirm = false;
              archiveToDelete = null;
            }}
          >
            取消
          </button>
          <button
            class="w-24 px-4 py-2 bg-red-600 hover:bg-red-700 text-white text-sm font-medium rounded-lg transition-colors"
            on:click={() => {
              if (archiveToDelete) {
                deleteArchive(archiveToDelete);
              } else {
                deleteSelectedArchives();
              }
            }}
          >
            删除
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

<!-- 生成完整录播Modal -->
<GenerateWholeClipModal
  bind:showModal={showWholeClipModal}
  archive={wholeClipArchive}
  roomId={wholeClipArchive?.room_id || ""}
  platform={wholeClipArchive?.platform || ""}
  on:generated={handleWholeClipGenerated}
/>

<style>
  /* macOS style modal */
  .mac-modal {
    box-shadow:
      0 20px 25px -5px rgba(0, 0, 0, 0.1),
      0 10px 10px -5px rgba(0, 0, 0, 0.04);
  }

  :global(.dark) .mac-modal {
    box-shadow:
      0 20px 25px -5px rgba(0, 0, 0, 0.3),
      0 10px 10px -5px rgba(0, 0, 0, 0.1);
  }

  /* fixed icon size in tables */
  :global(.table-icon) {
    width: 1rem; /* 16px, same as Tailwind w-4 */
    height: 1rem; /* 16px, same as Tailwind h-4 */
    flex: 0 0 auto;
  }
</style>
