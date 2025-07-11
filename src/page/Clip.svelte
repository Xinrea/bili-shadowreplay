<script lang="ts">
  import { invoke } from "../lib/invoker";
  import type { VideoItem } from "../lib/interface";
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
    Upload,
    Home,
  } from "lucide-svelte";

  let videos: VideoItem[] = [];
  let filteredVideos: VideoItem[] = [];
  let loading = false;
  let sortBy = "created_at";
  let sortOrder = "desc";
  let selectedRoomId: number | null = null;
  let roomIds: number[] = [];
  let selectedPlatform: string | null = null;
  let platforms: string[] = [];
  let selectedVideos: Set<number> = new Set();
  let showDeleteConfirm = false;
  let videoToDelete: VideoItem | null = null;

  onMount(async () => {
    await loadVideos();
  });

  let cover_cache: Map<number, string> = new Map();

  async function loadVideos() {
    loading = true;
    try {
      // Get all videos from all rooms
      const allVideos: VideoItem[] = [];

      // First, get all room IDs and platforms that have videos
      const roomIdsSet = new Set<number>();
      const platformsSet = new Set<string>();
      const tempVideos = await invoke<VideoItem[]>("get_all_videos");
      for (const video of tempVideos) {
        if (cover_cache.has(video.id)) {
          video.cover = cover_cache.get(video.id) || "";
        } else {
          video.cover = await invoke<string>("get_video_cover", {
            id: video.id,
          });
          cover_cache.set(video.id, video.cover);
        }
      }

      for (const video of tempVideos) {
        roomIdsSet.add(video.room_id);
        if (video.platform) {
          platformsSet.add(video.platform);
        }
        allVideos.push(video);
      }

      videos = allVideos;
      roomIds = Array.from(roomIdsSet).sort((a, b) => a - b);
      platforms = Array.from(platformsSet).sort();

      applyFilters();
    } catch (error) {
      console.error("Failed to load videos:", error);
    } finally {
      loading = false;
    }
  }

  function applyFilters() {
    console.log("applyFilters", selectedRoomId, selectedPlatform);
    let filtered = [...videos];

    // Apply room filter
    if (selectedRoomId !== null) {
      filtered = filtered.filter((video) => video.room_id === selectedRoomId);
    }

    // Apply platform filter
    if (selectedPlatform !== null) {
      filtered = filtered.filter(
        (video) => video.platform === selectedPlatform
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
      case "youtube":
        return "YouTube";
      default:
        return platform;
    }
  }

  function getRoomUrl(platform: string | undefined, roomId: number) {
    if (!platform) return null;
    switch (platform.toLowerCase()) {
      case "bilibili":
        return `https://live.bilibili.com/${roomId}`;
      case "douyin":
        return `https://live.douyin.com/${roomId}`;
      case "huya":
        return `https://www.huya.com/${roomId}`;
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
</script>

<!-- svelte-ignore a11y-click-events-have-key-events -->
<div class="flex-1 p-6 overflow-auto custom-scrollbar-light bg-gray-50">
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
            'platform'
              ? 'bg-blue-500 text-white'
              : 'bg-gray-100 dark:bg-gray-700/50 text-gray-700 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600'}"
            on:click={() => toggleSort("platform")}
          >
            平台
            {#if sortBy === "platform"}
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
          <RefreshCw class="w-8 h-8 animate-spin" />
          <span>加载中...</span>
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
          <table class="w-full table-fixed">
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
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400 w-20"
                  >平台</th
                >
                <th
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400 w-24"
                  >直播间</th
                >
                <th
                  class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400 w-64"
                  >视频</th
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

                  <td class="px-4 py-3 w-20">
                    <div class="flex items-center space-x-2">
                      <Globe class="w-4 h-4 text-gray-400" />
                      <span
                        class="text-sm text-gray-900 dark:text-white truncate"
                        >{formatPlatform(video.platform)}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-3 w-24">
                    <div class="flex items-center space-x-2">
                      <Home class="w-4 h-4 text-gray-400" />
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
                    </div>
                  </td>

                  <td class="px-4 py-3 w-64">
                    <div class="flex items-center space-x-3 w-full">
                      <div
                        class="w-12 h-8 rounded-lg overflow-hidden bg-gray-100 dark:bg-gray-700 flex items-center justify-center flex-shrink-0"
                      >
                        <img
                          src={video.cover}
                          alt="封面"
                          class="w-full h-full object-cover"
                        />
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
                      <Clock class="w-4 h-4 text-gray-400" />
                      <span class="text-sm text-gray-900 dark:text-white"
                        >{formatDuration(video.length)}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-3 w-24">
                    <div class="flex items-center space-x-2">
                      <HardDrive class="w-4 h-4 text-gray-400" />
                      <span class="text-sm text-gray-900 dark:text-white"
                        >{formatSize(video.size)}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-3 w-28">
                    <div class="flex items-center space-x-2">
                      <Calendar class="w-4 h-4 text-gray-400" />
                      <span
                        class="text-sm text-gray-900 dark:text-white truncate"
                        >{formatDate(video.created_at)}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-3 w-28">
                    <div class="flex items-center space-x-2">
                      <Upload class="w-4 h-4 text-gray-400" />
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
                    <div class="flex items-center space-x-1">
                      <button
                        class="p-1.5 text-gray-600 dark:text-gray-400 hover:text-blue-600 dark:hover:text-blue-400 hover:bg-blue-50 dark:hover:bg-blue-900/20 rounded transition-colors"
                        title="播放"
                        on:click={() => playVideo(video)}
                      >
                        <Play class="w-4 h-4" />
                      </button>
                      <button
                        class="p-1.5 text-gray-600 dark:text-gray-400 hover:text-red-600 dark:hover:text-red-400 hover:bg-red-50 dark:hover:bg-red-900/20 rounded transition-colors"
                        title="删除"
                        on:click={() => {
                          videoToDelete = video;
                          showDeleteConfirm = true;
                        }}
                      >
                        <Trash2 class="w-4 h-4" />
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
            class="w-24 px-4 py-2 text-sm font-medium text-gray-700 dark:text-gray-300 hover:bg-[#f5f5f7] dark:hover:bg-[#3a3a3c] rounded-lg transition-colors"
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

<style>
  /* macOS style modal */
  .mac-modal {
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
  }
</style>
