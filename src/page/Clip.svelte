<script lang="ts">
  import { invoke, TAURI_ENV, get_static_url } from "../lib/invoker";
  import type { VideoItem } from "../lib/interface";
  import ImportVideoDialog from "../lib/components/ImportVideoDialog.svelte";
  import { onMount, onDestroy, tick } from "svelte";
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
    Upload,
    Home,
    FileVideo,
    Scissors,
    Download,
    RotateCw,
    Edit,
  } from "lucide-svelte";
  import { AnnotationOutline } from "flowbite-svelte-icons";
  import BilibiliIcon from "../lib/components/BilibiliIcon.svelte";
  import DouyinIcon from "../lib/components/DouyinIcon.svelte";
  import KuaishouIcon from "../lib/components/KuaishouIcon.svelte";
  import HuyaIcon from "../lib/components/HuyaIcon.svelte";
  import TikTokIcon from "../lib/components/TikTokIcon.svelte";

  let videos: VideoItem[] = [];
  let filteredVideos: VideoItem[] = [];
  let loading = false;
  let sortBy = "created_at";
  let sortOrder = "desc";
  let selectedRoomId = null;
  let roomIds: string[] = [];

  let selectedVideos: Set<number> = new Set();
  let showDeleteConfirm = false;
  let videoToDelete: VideoItem | null = null;
  let showImportDialog = false;

  // 编辑备注相关状态
  let showEditNoteDialog = false;
  let videoToEditNote: VideoItem | null = null;
  let editingNote = "";

  onMount(async () => {
    await loadVideos();
  });

  onDestroy(() => {
    stopProgressPolling();
  });

  let importProgressInfo = null;
  let progressPollingTimer = null;

  /**
   * 响应式语句：当检测到转换任务时，确保 loading 状态为 true
   */
  $: if (importProgressInfo) {
    loading = true;
  }

  /**
   * 启动进度轮询定时器
   * 每3秒检查一次导入任务进度
   */
  function startProgressPolling() {
    if (progressPollingTimer) {
      clearTimeout(progressPollingTimer);
    }
    progressPollingTimer = setTimeout(async () => {
      await checkImportProgress();
    }, 3000);
  }

  /**
   * 检查导入任务进度
   * 如果任务仍在进行中，更新进度信息并继续轮询
   * 如果任务完成，停止轮询并重新加载视频列表
   */
  async function checkImportProgress() {
    try {
      const importProgress = await invoke("get_import_progress");

      if (importProgress) {
        // 仍有进行中的任务，更新进度信息并继续轮询
        importProgressInfo = importProgress;
        loading = true;
        startProgressPolling();
      } else {
        // 任务已完成，延迟100ms确保状态同步后再重新加载
        importProgressInfo = null;
        stopProgressPolling();

        // 延迟处理避免状态竞争
        setTimeout(async () => {
          loading = false;
          // 重新加载视频列表（不包括进度检查）
          await loadVideoList();
        }, 100);
      }
    } catch (error) {
      console.error("轮询检查进度失败:", error);

      // 不要立即重置状态，避免在网络错误时丢失进度
      // 保持现有状态一段时间，然后重置
      setTimeout(() => {
        importProgressInfo = null;
        stopProgressPolling();
        loading = false;
      }, 5000);
    }
  }

  /**
   * 停止进度轮询定时器
   */
  function stopProgressPolling() {
    if (progressPollingTimer) {
      clearTimeout(progressPollingTimer);
      progressPollingTimer = null;
    }
  }

  /**
   * 格式化毫秒转换为可读的时间长度
   * @param milliseconds 毫秒数
   * @returns 格式化的时间字符串（如：1小时30分20秒）
   */
  function formatProgressDuration(milliseconds: number): string {
    const seconds = Math.floor(milliseconds / 1000);
    const minutes = Math.floor(seconds / 60);
    const hours = Math.floor(minutes / 60);

    if (hours > 0) {
      return `${hours}小时${minutes % 60}分${seconds % 60}秒`;
    } else if (minutes > 0) {
      return `${minutes}分${seconds % 60}秒`;
    } else {
      return `${seconds}秒`;
    }
  }

  /**
   * 加载视频列表
   * 1. 扫描导入目录中的新视频文件并自动导入
   * 2. 检查是否有正在进行的转换任务
   * 3. 加载并显示所有视频
   */
  async function loadVideos() {
    loading = true;
    try {
      await loadVideoList();
    } catch (error) {
      console.error("Failed to load videos:", error);
      importProgressInfo = null;
      stopProgressPolling();
      loading = false;
    }
  }

  /**
   * 加载视频数据列表
   * 从后端获取所有视频数据，包含封面信息，并应用筛选条件
   */
  async function loadVideoList() {
    try {
      // 获取所有视频
      const allVideos: VideoItem[] = [];
      const roomIdsSet = new Set<string>();
      const tempVideos = await invoke<VideoItem[]>("get_all_videos");

      for (const video of tempVideos) {
        video.cover = await get_static_url("output", video.cover);
      }

      for (const video of tempVideos) {
        roomIdsSet.add(video.room_id);
        allVideos.push(video);
      }

      videos = allVideos;
      roomIds = Array.from(roomIdsSet).sort();

      applyFilters();
    } catch (error) {
      console.error("加载视频列表失败:", error);
      throw error;
    } finally {
      // 只有在没有转换任务且没有轮询定时器时才设置 loading = false
      if (!importProgressInfo && !progressPollingTimer) {
        loading = false;
      }
    }
  }

  function applyFilters() {
    let filtered = [...videos];

    // Apply room filter
    if (selectedRoomId !== null) {
      filtered = filtered.filter((video) => video.room_id === selectedRoomId);
    }

    // Apply sorting
    filtered.sort((a, b) => {
      let aValue: any, bValue: any;

      switch (sortBy) {
        case "title":
          aValue = a.title.toLowerCase();
          bValue = b.title.toLowerCase();
          break;
          bValue = b.note;
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
          aValue = a.created_at;
          bValue = b.created_at;
      }

      if (sortOrder === "asc") {
        return aValue > bValue ? 1 : -1;
      } else {
        return aValue < bValue ? 1 : -1;
      }
    });

    filteredVideos = filtered;
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
    return new Date(dateString).toLocaleString();
  }

  function formatPlatform(platform: string | undefined) {
    if (!platform) return "未知";
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
      case "imported":
        return "导入视频";
      case "clip":
        return "切片";
      default:
        return platform;
    }
  }

  function getRoomUrl(platform: string | undefined, roomId: string) {
    if (!platform) return null;
    if (roomId.startsWith("http")) return roomId;
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

  function toggleSort(field: string) {
    if (sortBy === field) {
      sortOrder = sortOrder === "asc" ? "desc" : "asc";
    } else {
      sortBy = field;
      sortOrder = "asc";
    }
    applyFilters();
  }

  function toggleVideoSelection(id: number) {
    if (selectedVideos.has(id)) {
      selectedVideos.delete(id);
    } else {
      selectedVideos.add(id);
    }
    selectedVideos = selectedVideos; // Trigger reactivity
  }

  function selectAllVideos() {
    const currentVideos = filteredVideos;
    if (selectedVideos.size === currentVideos.length) {
      selectedVideos.clear();
    } else {
      currentVideos.forEach((video) => selectedVideos.add(video.id));
    }
    selectedVideos = selectedVideos; // Trigger reactivity
  }

  async function deleteVideo(video: VideoItem) {
    try {
      await invoke("delete_video", { id: video.id });
      await loadVideos();
      showDeleteConfirm = false;
      videoToDelete = null;
    } catch (error) {
      console.error("Failed to delete video:", error);
    }
  }

  async function deleteSelectedVideos() {
    try {
      for (const id of selectedVideos) {
        await invoke("delete_video", { id });
      }
      selectedVideos.clear();
      await loadVideos();
      showDeleteConfirm = false;
      videoToDelete = null;
    } catch (error) {
      console.error("Failed to delete selected videos:", error);
    }
  }

  async function playVideo(video: VideoItem) {
    try {
      await invoke("open_clip", { videoId: video.id });
    } catch (error) {
      console.error("Failed to play video:", error);
    }
  }

  function handleVideoImported() {
    // 视频导入完成后刷新列表
    loadVideos();
  }

  function handleImageError(event: Event) {
    // 如果图片加载失败，隐藏图片元素并显示默认图标
    const target = event.target as HTMLImageElement;
    target.style.display = "none";
    if (target.parentElement) {
      target.parentElement.innerHTML =
        '<svg class="w-6 h-6 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 10l4.553-2.276A1 1 0 0121 8.618v6.764a1 1 0 01-1.447.894L15 14M5 18h8a2 2 0 002-2V8a2 2 0 00-2-2H5a2 2 0 00-2 2v8a2 2 0 002 2z"></path></svg>';
    }
  }

  async function exportVideo(video: VideoItem) {
    // download video
    const video_url = await get_static_url("output", video.file);
    const video_name = video.title;
    const a = document.createElement("a");
    a.href = video_url;
    a.download = video_name;
    a.click();
  }

  // 编辑备注相关函数
  function openEditNoteDialog(video: VideoItem) {
    videoToEditNote = video;
    editingNote = video.note || "";
    showEditNoteDialog = true;
  }

  function closeEditNoteDialog() {
    showEditNoteDialog = false;
    videoToEditNote = null;
    editingNote = "";
  }

  async function saveNote() {
    if (!videoToEditNote) return;

    try {
      await invoke("update_video_note", {
        id: videoToEditNote.id,
        note: editingNote,
      });

      // 更新本地数据
      videoToEditNote.note = editingNote;

      // 更新筛选后的视频列表
      const index = filteredVideos.findIndex(
        (v) => v.id === videoToEditNote.id
      );
      if (index !== -1) {
        filteredVideos[index].note = editingNote;
      }

      // 更新所有视频列表
      const allIndex = videos.findIndex((v) => v.id === videoToEditNote.id);
      if (allIndex !== -1) {
        videos[allIndex].note = editingNote;
      }

      closeEditNoteDialog();
    } catch (error) {
      console.error("Failed to update video note:", error);
    }
  }

  // 键盘事件处理
  function handleKeydown(event: KeyboardEvent) {
    if (showEditNoteDialog && event.key === "Escape") {
      closeEditNoteDialog();
    }
  }
</script>

<!-- svelte-ignore a11y-click-events-have-key-events -->
<svelte:window on:keydown={handleKeydown} />

<div class="flex-1 p-6 overflow-auto custom-scrollbar-light bg-gray-50 dark:bg-black">
  <div class="space-y-6">
    <!-- Header -->
    <div class="flex justify-between items-center">
      <div class="space-y-1">
        <h1 class="text-2xl font-semibold text-gray-900 dark:text-white">
          切片
        </h1>
        <p class="text-sm text-gray-500 dark:text-gray-400">
          管理所有录播产生的切片；如需生成切片，请从直播间列表进入录播预览页面操作。
        </p>
      </div>

      <div class="flex items-center space-x-3">
        <button
          class="px-4 py-2 bg-green-500 text-white rounded-lg hover:bg-green-600 transition-colors flex items-center space-x-2"
          on:click={() => (showImportDialog = true)}
        >
          <Upload class="w-4 h-4 text-white" />
          <span>导入视频</span>
        </button>
        <button
          class="px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors flex items-center space-x-2 disabled:opacity-50 disabled:cursor-not-allowed"
          on:click={loadVideos}
          disabled={loading}
        >
          <RefreshCw
            class="w-4 h-4 text-white {loading ? 'animate-spin' : ''}"
          />
          <span>刷新</span>
        </button>
      </div>
    </div>

    <!-- Filters -->
    <div
      class="p-4 rounded-xl bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 space-y-4"
    >
      <div class="flex justify-between items-center flex-wrap gap-4">
        <div class="flex space-x-3">
          <select
            bind:value={selectedRoomId}
            on:change={applyFilters}
            class="px-3 py-2 bg-gray-100 dark:bg-gray-700/50 border border-gray-200 dark:border-gray-600 rounded-lg text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500 dark:focus:ring-blue-400 cursor-pointer"
          >
            <option value={null}>所有直播间</option>
            {#each roomIds as roomId}
              <option value={roomId}>{roomId}</option>
            {/each}
          </select>
        </div>

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
            文件名
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
            'note'
              ? 'bg-blue-500 text-white'
              : 'bg-gray-100 dark:bg-gray-700/50 text-gray-700 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600'}"
            on:click={() => toggleSort("note")}
          >
            备注
            {#if sortBy === "note"}
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
    </div>

    <!-- Bulk Actions -->
    {#if selectedVideos.size > 0}
      <div
        class="flex justify-between items-center p-4 rounded-xl bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700"
      >
        <span class="text-sm text-gray-600 dark:text-gray-400">
          已选择 {selectedVideos.size} 个视频
        </span>
        <button
          class="px-4 py-2 bg-red-600 hover:bg-red-700 text-white rounded-lg transition-colors flex items-center space-x-2"
          on:click={() => {
            showDeleteConfirm = true;
            videoToDelete = null;
          }}
        >
          <Trash2 class="w-4 h-4 icon-white" />
          <span>删除选中</span>
        </button>
      </div>
    {/if}

    <!-- Video List -->
    <div
      class="bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 rounded-xl overflow-hidden"
    >
      {#if loading}
        <div
          class="flex flex-col items-center justify-center p-12 space-y-4 text-gray-500 dark:text-gray-400"
        >
          {#if importProgressInfo}
            <!-- 优化的转换进度显示 -->
            <div class="text-center space-y-3 max-w-md">
              <!-- 主要信息：醒目的转换状态 -->
              <div class="flex items-center justify-center space-x-3">
                <RotateCw
                  class="w-7 h-7 text-blue-600 dark:text-blue-400 animate-spin"
                />
                <span
                  class="text-xl font-semibold text-blue-600 dark:text-blue-400"
                >
                  正在转换视频
                </span>
              </div>

              <!-- 副信息：文件名显示在小字部分 -->
              <div
                class="text-sm text-gray-500 dark:text-gray-400 break-all px-4"
              >
                {importProgressInfo.fileName ||
                  importProgressInfo.file_name ||
                  "正在准备..."}
              </div>
            </div>
          {:else}
            <!-- 普通加载状态 -->
            <RefreshCw class="w-8 h-8 animate-spin" />
            <span>加载中...</span>
          {/if}
        </div>
      {:else if filteredVideos.length === 0}
        <div
          class="flex flex-col items-center justify-center p-12 space-y-4 text-gray-500 dark:text-gray-400"
        >
          <Video class="w-12 h-12" />
          <h3 class="text-lg font-medium text-gray-900 dark:text-white">
            暂无视频
          </h3>
          <p class="text-sm">
            {selectedRoomId !== null
              ? "没有找到匹配的视频"
              : "还没有录制任何视频切片"}
          </p>
        </div>
      {:else}
        <div class="overflow-x-auto custom-scrollbar-light">
          <table class="w-full table-fixed whitespace-nowrap">
            <thead>
              <tr class="border-b border-gray-200 dark:border-gray-700/50">
                <th class="px-4 py-3 text-left w-12">
                  <input
                    type="checkbox"
                    checked={selectedVideos.size === filteredVideos.length &&
                      filteredVideos.length > 0}
                    on:change={selectAllVideos}
                    class="rounded border-gray-300 dark:border-gray-600"
                  />
                </th>
                <th
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400 w-28"
                  >直播间</th
                >
                <th
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400 w-64"
                  >视频</th
                >
                <th
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400 w-20"
                  >备注</th
                >
                <th
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400 w-20"
                  >时长</th
                >
                <th
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400 w-24"
                  >大小</th
                >
                <th
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400 w-28"
                  >创建时间</th
                >
                <th
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400 w-28"
                  >投稿状态</th
                >
                <th
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400 w-28"
                  >操作</th
                >
              </tr>
            </thead>
            <tbody class="divide-y divide-gray-200 dark:divide-gray-700/50">
              {#each filteredVideos as video (video.id)}
                <tr
                  class="group hover:bg-[#f5f5f7] dark:hover:bg-[#3a3a3c] transition-colors"
                >
                  <td class="px-4 py-3 w-12">
                    <input
                      type="checkbox"
                      checked={selectedVideos.has(video.id)}
                      on:change={() => toggleVideoSelection(video.id)}
                      class="rounded border-gray-300 dark:border-gray-600"
                    />
                  </td>

                  <td class="px-4 py-3 w-28">
                    <div class="flex items-center space-x-2">
                      {#if video.platform === "imported"}
                        <FileVideo class="table-icon text-gray-400" />
                        <span class="text-sm text-gray-800 dark:text-gray-200"
                          >外部视频</span
                        >
                      {:else if video.platform === "clip"}
                        <Scissors class="table-icon text-gray-400" />
                        <span class="text-sm text-gray-800 dark:text-gray-200"
                          >视频切片</span
                        >
                      {:else if video.platform === "bilibili"}
                        <BilibiliIcon class="w-4 h-4 flex-shrink-0" />
                        {#if getRoomUrl(video.platform, video.room_id)}
                          <a
                            href={getRoomUrl(video.platform, video.room_id)}
                            target="_blank"
                            rel="noopener noreferrer"
                            class="text-blue-500 hover:text-blue-700 text-sm"
                            title={`打开 ${formatPlatform(video.platform)} 直播间`}
                          >
                            {video.room_id}
                          </a>
                        {:else}
                          <span class="text-sm text-gray-900 dark:text-white"
                            >{video.room_id}</span
                          >
                        {/if}
                      {:else if video.platform === "douyin"}
                        <DouyinIcon class="w-4 h-4 flex-shrink-0" />
                        {#if getRoomUrl(video.platform, video.room_id)}
                          <a
                            href={getRoomUrl(video.platform, video.room_id)}
                            target="_blank"
                            rel="noopener noreferrer"
                            class="text-blue-500 hover:text-blue-700 text-sm"
                            title={`打开 ${formatPlatform(video.platform)} 直播间`}
                          >
                            {video.room_id}
                          </a>
                        {:else}
                          <span class="text-sm text-gray-900 dark:text-white"
                            >{video.room_id}</span
                          >
                        {/if}
                      {:else if video.platform === "kuaishou"}
                        <KuaishouIcon class="w-4 h-4 flex-shrink-0" />
                        {#if getRoomUrl(video.platform, video.room_id)}
                          <a
                            href={getRoomUrl(video.platform, video.room_id)}
                            target="_blank"
                            rel="noopener noreferrer"
                            class="text-blue-500 hover:text-blue-700 text-sm"
                            title={`打开 ${formatPlatform(video.platform)} 直播间`}
                          >
                            {video.room_id}
                          </a>
                        {:else}
                          <span class="text-sm text-gray-900 dark:text-white"
                            >{video.room_id}</span
                          >
                        {/if}
                      {:else if video.platform === "huya"}
                        <HuyaIcon class="w-4 h-4 flex-shrink-0" />
                        {#if getRoomUrl(video.platform, video.room_id)}
                          <a
                            href={getRoomUrl(video.platform, video.room_id)}
                            target="_blank"
                            rel="noopener noreferrer"
                            class="text-blue-500 hover:text-blue-700 text-sm"
                            title={`打开 ${formatPlatform(video.platform)} 直播间`}
                          >
                            {video.room_id}
                          </a>
                        {:else}
                          <span class="text-sm text-gray-900 dark:text-white"
                            >{video.room_id}</span
                          >
                        {/if}
                      {:else if video.platform === "tiktok"}
                        <TikTokIcon class="w-5 h-5 flex-shrink-0" />
                        {#if getRoomUrl(video.platform, video.room_id)}
                          <a
                            href={getRoomUrl(video.platform, video.room_id)}
                            target="_blank"
                            rel="noopener noreferrer"
                            class="text-blue-500 hover:text-blue-700 text-sm"
                            title={`打开 ${formatPlatform(video.platform)} 直播间`}
                          >
                            {video.room_id}
                          </a>
                        {:else}
                          <span class="text-sm text-gray-900 dark:text-white"
                            >{video.room_id}</span
                          >
                        {/if}
                      {:else}
                        <Globe class="table-icon text-gray-400" />
                        {#if getRoomUrl(video.platform, video.room_id)}
                          <a
                            href={getRoomUrl(video.platform, video.room_id)}
                            target="_blank"
                            rel="noopener noreferrer"
                            class="text-blue-500 hover:text-blue-700 text-sm"
                            title={`打开 ${formatPlatform(video.platform)} 直播间`}
                          >
                            {video.room_id}
                          </a>
                        {:else}
                          <span class="text-sm text-gray-900 dark:text-white"
                            >{video.room_id}</span
                          >
                        {/if}
                      {/if}
                    </div>
                  </td>

                  <td class="px-4 py-3 w-64">
                    <div class="flex items-center space-x-3 w-full">
                      <div
                        class="w-12 h-8 rounded-lg overflow-hidden bg-gray-100 dark:bg-gray-700 flex items-center justify-center flex-shrink-0"
                      >
                        {#if video.cover && video.cover.trim() !== ""}
                          <img
                            src={video.cover}
                            alt="封面"
                            class="w-full h-full object-cover"
                            on:error={handleImageError}
                          />
                        {:else}
                          <!-- 默认视频图标 -->
                          <svg
                            class="w-6 h-6 text-gray-400"
                            fill="none"
                            stroke="currentColor"
                            viewBox="0 0 24 24"
                          >
                            <path
                              stroke-linecap="round"
                              stroke-linejoin="round"
                              stroke-width="2"
                              d="M15 10l4.553-2.276A1 1 0 0121 8.618v6.764a1 1 0 01-1.447.894L15 14M5 18h8a2 2 0 002-2V8a2 2 0 00-2-2H5a2 2 0 00-2 2v8a2 2 0 002 2z"
                            ></path>
                          </svg>
                        {/if}
                      </div>
                      <div class="min-w-0 flex-1 w-64">
                        <p
                          class="text-sm font-medium text-gray-900 dark:text-white truncate w-full"
                          title={video.title || video.file}
                        >
                          {video.title || video.file}
                        </p>
                        <p
                          class="text-xs text-gray-500 dark:text-gray-400 truncate w-full"
                          title={video.file}
                        >
                          {video.file}
                        </p>
                      </div>
                    </div>
                  </td>

                  <td class="px-4 py-3 w-20">
                    <div class="flex items-center space-x-2">
                      <AnnotationOutline class="table-icon text-gray-400" />
                      <span class="text-sm text-gray-800 truncate"
                        >{video.note}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-3 w-20">
                    <div class="flex items-center space-x-2">
                      <Clock class="table-icon text-gray-400" />
                      <span class="text-sm text-gray-800"
                        >{formatDuration(video.length)}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-3 w-24">
                    <div class="flex items-center space-x-2">
                      <HardDrive class="table-icon text-gray-400" />
                      <span class="text-sm text-gray-800"
                        >{formatSize(video.size)}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-3 w-28">
                    <div class="flex items-center space-x-2">
                      <Calendar class="table-icon text-gray-400" />
                      <span class="text-sm text-gray-800 truncate"
                        >{formatDate(video.created_at)}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-3 w-28">
                    <div class="flex items-center space-x-2">
                      <Upload class="table-icon text-gray-400" />
                      {#if video.bvid}
                        <a
                          href={`https://www.bilibili.com/video/${video.bvid}`}
                          target="_blank"
                          rel="noopener noreferrer"
                          class="text-blue-500 hover:text-blue-700 text-sm truncate"
                          title={video.bvid}
                        >
                          {video.bvid}
                        </a>
                      {:else}
                        <span class="text-gray-500 dark:text-gray-400 text-sm"
                          >未投稿</span
                        >
                      {/if}
                    </div>
                  </td>

                  <td class="px-4 py-3 w-28">
                    <div class="flex items-center space-x-2">
                      <button
                        class="p-1.5 rounded-lg hover:bg-blue-500/10 transition-colors"
                        title="播放"
                        on:click={() => playVideo(video)}
                      >
                        <Play class="w-4 h-4 text-blue-500" />
                      </button>
                      <button
                        class="p-1.5 rounded-lg hover:bg-green-500/10 transition-colors"
                        title="编辑备注"
                        on:click={() => openEditNoteDialog(video)}
                      >
                        <Edit class="w-4 h-4 text-green-500" />
                      </button>
                      {#if !TAURI_ENV}
                        <button
                          class="p-1.5 rounded-lg hover:bg-blue-500/10 transition-colors"
                          title="导出"
                          on:click={async () => await exportVideo(video)}
                        >
                          <Download class="w-4 h-4 text-blue-500" />
                        </button>
                      {/if}
                      <button
                        class="p-1.5 rounded-lg hover:bg-red-500/10 transition-colors"
                        title="删除"
                        on:click={() => {
                          videoToDelete = video;
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
            {#if videoToDelete}
              确定要删除视频 "{videoToDelete.title || videoToDelete.file}" 吗？
            {:else}
              确定要删除选中的 {selectedVideos.size} 个视频吗？
            {/if}
          </p>
          <p class="text-xs text-red-600 dark:text-red-500">此操作无法撤销。</p>
        </div>
        <div class="flex justify-center space-x-3">
          <button
            class="w-24 px-4 py-2 text-sm font-medium text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-600 rounded-lg transition-colors"
            on:click={() => {
              showDeleteConfirm = false;
              videoToDelete = null;
            }}
          >
            取消
          </button>
          <button
            class="w-24 px-4 py-2 bg-red-600 hover:bg-red-700 text-white text-sm font-medium rounded-lg transition-colors"
            on:click={() => {
              if (videoToDelete) {
                deleteVideo(videoToDelete);
              } else {
                deleteSelectedVideos();
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

<!-- Edit Note Dialog -->
{#if showEditNoteDialog && videoToEditNote}
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <!-- svelte-ignore a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 bg-black/30 dark:bg-black/50 z-50 flex items-center justify-center p-4"
    on:click={closeEditNoteDialog}
  >
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div
      class="mac-modal w-[480px] max-w-full bg-white dark:bg-[#2d2d30] rounded-xl shadow-xl border border-gray-200 dark:border-gray-600 overflow-hidden"
      on:click|stopPropagation
      role="dialog"
      aria-labelledby="edit-note-title"
      aria-describedby="edit-note-description"
    >
      <!-- Header -->
      <div
        class="px-6 pt-6 pb-4 border-b border-gray-100 dark:border-gray-700/50"
      >
        <div class="flex items-center space-x-3">
          <div
            class="w-8 h-8 rounded-full bg-blue-100 dark:bg-blue-900/30 flex items-center justify-center"
          >
            <Edit class="w-4 h-4 text-blue-600 dark:text-blue-400" />
          </div>
          <div class="min-w-0 flex-1">
            <h3
              id="edit-note-title"
              class="text-base font-semibold text-gray-900 dark:text-white"
            >
              编辑切片备注
            </h3>
            <p
              id="edit-note-description"
              class="text-sm text-gray-500 dark:text-gray-400 truncate mt-0.5"
            >
              {videoToEditNote.title || videoToEditNote.file}
            </p>
          </div>
        </div>
      </div>

      <!-- Content -->
      <div class="px-6 py-5">
        <div class="space-y-3">
          <label
            for="edit-note-textarea"
            class="block text-sm font-medium text-gray-700 dark:text-gray-300"
          >
            备注内容
          </label>
          <div class="relative">
            <textarea
              id="edit-note-textarea"
              bind:value={editingNote}
              placeholder="为这个切片添加备注信息，如高光时刻、重要内容等..."
              class="w-full px-3 py-2 text-sm
                     bg-white dark:bg-gray-800
                     border border-gray-300 dark:border-gray-500 rounded-md
                     text-gray-900 dark:text-white placeholder-gray-500 dark:placeholder-gray-400
                     focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-blue-500
                     transition-colors duration-150 resize-none"
              rows="5"
              on:keydown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  saveNote();
                }
              }}
            ></textarea>
            <!-- Helper text -->
            <div class="mt-2 text-xs text-gray-500 dark:text-gray-400">
              按 ⌘+Enter 快速保存
            </div>
          </div>
        </div>
      </div>

      <!-- Footer -->
      <div
        class="px-6 py-4 bg-gray-50 dark:bg-gray-800 border-t border-gray-200 dark:border-gray-600"
      >
        <div class="flex justify-end space-x-3">
          <button
            class="w-24 px-4 py-2 text-sm font-medium text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-600 rounded-lg transition-colors"
            on:click={closeEditNoteDialog}
          >
            取消
          </button>
          <button
            class="w-24 px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium rounded-lg transition-colors"
            on:click={saveNote}
          >
            保存
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

<!-- 导入视频对话框 -->
<ImportVideoDialog
  bind:showDialog={showImportDialog}
  roomId={selectedRoomId}
  on:imported={handleVideoImported}
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

  /* macOS style textarea */
  .mac-modal textarea {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
      sans-serif;
  }

  .mac-modal textarea:focus {
    box-shadow: 0 0 0 3px rgba(0, 122, 255, 0.3);
  }
</style>
