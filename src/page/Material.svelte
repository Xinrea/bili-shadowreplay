<script lang="ts">
  import { formatTimecode } from "../lib/time-format";
  import { formatSize } from "../lib/size-format";
  import { liveRoomUrl, platformLabel } from "../lib/platform";
  import { createStateRefresh, watchStateChanges } from "../lib/state-refresh";
  import { invoke, TAURI_ENV, get_static_url } from "../lib/invoker";
  import type { VideoItem } from "../lib/interface";
  import ImportVideoDialog from "../lib/components/ImportVideoDialog.svelte";
  import PageTitlebar from "../lib/components/PageTitlebar.svelte";
  import DeleteConfirmActions from "../lib/components/DeleteConfirmActions.svelte";
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
    Clapperboard,
    FolderOpen,
  } from "lucide-svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { registerCommands } from "../lib/stores/commands";
  import { requestUpload } from "../lib/uploads";
  import { AnnotationOutline } from "flowbite-svelte-icons";
  import BilibiliIcon from "../lib/components/BilibiliIcon.svelte";
  import DouyinIcon from "../lib/components/DouyinIcon.svelte";
  import DouyuIcon from "../lib/components/DouyuIcon.svelte";
  import KuaishouIcon from "../lib/components/KuaishouIcon.svelte";
  import HuyaIcon from "../lib/components/HuyaIcon.svelte";
  import TikTokIcon from "../lib/components/TikTokIcon.svelte";
  import TwitchIcon from "../lib/components/TwitchIcon.svelte";
  import YouTubeIcon from "../lib/components/YouTubeIcon.svelte";

  let videos: VideoItem[] = [];
  let filteredVideos: VideoItem[] = $state([]);
  let loading = $state(false);
  let sortBy = $state("created_at");
  let sortOrder = $state("desc");
  let selectedRoomId = $state(null);
  let roomIds: string[] = $state([]);

  let selectedVideos = new SvelteSet<number>();
  let showDeleteConfirm = $state(false);
  let videoToDelete: VideoItem | null = $state(null);
  /** Projects that lose clips when the pending deletion goes ahead. */
  let affectedProjects: { id: number; title: string }[] = $state([]);
  let creatingProject = $state(false);
  let showImportDialog = $state(false);

  // 编辑备注相关状态
  let showEditNoteDialog = $state(false);
  let videoToEditNote: VideoItem | null = $state(null);
  let editingNote = $state("");

  onMount(() => watchStateChanges(stateRefresh, ["videos-changed"]));

  onMount(() =>
    registerCommands([
      {
        id: "material:import",
        group: "action",
        title: "导入视频",
        detail: "素材",
        icon: Upload,
        keywords: ["import video", "upload", "material"],
        page: "素材",
        run: () => (showImportDialog = true),
      },
      {
        id: "material:refresh",
        group: "action",
        title: "刷新素材列表",
        detail: "素材",
        icon: RefreshCw,
        keywords: ["refresh materials", "reload"],
        page: "素材",
        run: loadVideos,
      },
    ]),
  );

  onDestroy(() => {
    stopProgressPolling();
  });

  let importProgressInfo = $state(null);
  let progressPollingTimer = null;

  /**
   * 响应式语句：当检测到转换任务时，确保 loading 状态为 true
   */
  $effect(() => {
    if (importProgressInfo) {
      loading = true;
    }
  });

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
  const stateRefresh = createStateRefresh(fetchVideoList);

  async function loadVideoList() {
    await stateRefresh.run();
  }

  async function fetchVideoList() {
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
        case "note":
          aValue = (a.note || "").toLowerCase();
          bValue = (b.note || "").toLowerCase();
          break;
        case "length":
          aValue = a.duration || a.length;
          bValue = b.duration || b.length;
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

  function formatDate(dateString: string) {
    return new Date(dateString).toLocaleString();
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
  }

  function selectAllVideos() {
    const currentVideos = filteredVideos;
    if (selectedVideos.size === currentVideos.length) {
      selectedVideos.clear();
    } else {
      currentVideos.forEach((video) => selectedVideos.add(video.id));
    }
  }

  /** Ask before deleting, naming the projects that use the materials. */
  async function confirmDelete(video: VideoItem | null) {
    videoToDelete = video;
    const ids = video ? [video.id] : [...selectedVideos];
    const usage = await Promise.all(
      ids.map((id) => invoke<{ id: number; title: string }[]>("get_material_usage", { id }).catch(() => [])),
    );
    affectedProjects = [...new Map(usage.flat().map((project) => [project.id, project])).values()];
    showDeleteConfirm = true;
  }

  function closeDeleteConfirm() {
    showDeleteConfirm = false;
    videoToDelete = null;
    affectedProjects = [];
  }

  async function deleteVideo(video: VideoItem) {
    await invoke("delete_video", { id: video.id });
    selectedVideos.delete(video.id);
    await loadVideos();
    closeDeleteConfirm();
  }

  async function deleteSelectedVideos() {
    const ids = [...selectedVideos];
    for (const id of ids) {
      await invoke("delete_video", { id });
      selectedVideos.delete(id);
    }
    await loadVideos();
    closeDeleteConfirm();
  }

  async function playVideo(video: VideoItem) {
    try {
      await invoke("open_clip", { videoId: video.id });
    } catch (error) {
      console.error("Failed to play video:", error);
    }
  }

  /** Create a project from materials, oldest first, and open it. */
  async function createProject(materials: VideoItem[]) {
    if (materials.length === 0 || creatingProject) return;
    creatingProject = true;
    try {
      const ordered = [...materials].sort(
        (a, b) => new Date(a.created_at).getTime() - new Date(b.created_at).getTime(),
      );
      const project = await invoke<{ id: number }>("create_project", {
        title: "",
        roomId: ordered[0].room_id,
        platform: ordered[0].platform ?? "",
        materialIds: ordered.map((video) => video.id),
      });
      selectedVideos.clear();
      await invoke("open_project", { projectId: project.id });
    } catch (error) {
      alert("新建切片失败：" + error);
    } finally {
      creatingProject = false;
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
        '<svg class="w-5 h-5 text-fg-faint" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 10l4.553-2.276A1 1 0 0121 8.618v6.764a1 1 0 01-1.447.894L15 14M5 18h8a2 2 0 002-2V8a2 2 0 00-2-2H5a2 2 0 00-2 2v8a2 2 0 002 2z"></path></svg>';
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

  async function openMaterialFolder() {
    try {
      await invoke("open_material_folder");
    } catch (error) {
      alert(`打开素材目录失败：${error}`);
    }
  }

  // 键盘事件处理
  function handleKeydown(event: KeyboardEvent) {
    if (showEditNoteDialog && event.key === "Escape") {
      closeEditNoteDialog();
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<svelte:window onkeydown={handleKeydown} />

<PageTitlebar title="素材">
  {#snippet subtitle()}
    <span class="truncate text-[12px] text-fg-muted">
      从录播截取或导入的视频素材；选择素材新建切片，在切片中剪辑、加字幕后投稿。
    </span>
  {/snippet}
  <button
    class="titlebar-icon-button"
    title="刷新"
    aria-label="刷新"
    onclick={loadVideos}
    disabled={loading}
  >
    <RefreshCw class="h-4 w-4 {loading ? 'animate-spin' : ''}" />
  </button>
  {#if TAURI_ENV}
    <button
      class="inline-flex h-7 items-center gap-1.5 whitespace-nowrap rounded-[7px] border border-line px-2.5 text-[12px] font-medium text-fg-secondary transition-colors hover:bg-control-hover"
      onclick={openMaterialFolder}
    >
      <FolderOpen class="h-3.5 w-3.5" />
      打开素材目录
    </button>
  {/if}
  <button
    class="inline-flex h-7 items-center gap-1.5 whitespace-nowrap rounded-[7px] bg-success px-2.5 text-[12px] font-medium text-white transition-colors hover:bg-success/90"
    onclick={() => (showImportDialog = true)}
  >
    <Upload class="h-3.5 w-3.5" />
    导入视频
  </button>
</PageTitlebar>

<div class="flex-1 overflow-auto bg-canvas p-5 custom-scrollbar-light">
  <div class="space-y-4">
    <!-- Filters -->
    <div class="ui-card px-3 py-2.5">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div class="flex items-center gap-2">
          <select
            bind:value={selectedRoomId}
            onchange={applyFilters}
            class="ui-input min-w-[160px]"
          >
            <option value={null}>所有直播间</option>
            {#each roomIds as roomId}
              <option value={roomId}>{roomId}</option>
            {/each}
          </select>
        </div>

        <div class="flex flex-wrap items-center gap-1.5">
          <span class="mr-0.5 text-[12px] text-fg-muted">排序:</span>
          <button
            class="ui-btn ui-btn-sm {sortBy === 'room_id' ? 'ui-btn-active' : ''}"
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
            class="ui-btn ui-btn-sm {sortBy === 'title' ? 'ui-btn-active' : ''}"
            onclick={() => toggleSort("title")}
          >
            文件名
            {#if sortBy === "title"}
              {#if sortOrder === "asc"}
                <ChevronUp class="h-3 w-3" />
              {:else}
                <ChevronDown class="h-3 w-3" />
              {/if}
            {/if}
          </button>
          <button
            class="ui-btn ui-btn-sm {sortBy === 'note' ? 'ui-btn-active' : ''}"
            onclick={() => toggleSort("note")}
          >
            备注
            {#if sortBy === "note"}
              {#if sortOrder === "asc"}
                <ChevronUp class="h-3 w-3" />
              {:else}
                <ChevronDown class="h-3 w-3" />
              {/if}
            {/if}
          </button>
          <button
            class="ui-btn ui-btn-sm {sortBy === 'length' ? 'ui-btn-active' : ''}"
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
            class="ui-btn ui-btn-sm {sortBy === 'size' ? 'ui-btn-active' : ''}"
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
            class="ui-btn ui-btn-sm {sortBy === 'created_at' ? 'ui-btn-active' : ''}"
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
    </div>

    <!-- Bulk Actions -->
    {#if selectedVideos.size > 0}
      <div class="ui-card flex items-center justify-between px-4 py-2.5">
        <span class="text-[12px] text-fg-secondary">
          已选择 {selectedVideos.size} 个素材
        </span>
        <div class="flex items-center gap-2">
          <button
            class="ui-btn ui-btn-primary"
            disabled={creatingProject}
            onclick={() => createProject(videos.filter((video) => selectedVideos.has(video.id)))}
          >
            <Clapperboard class="h-3.5 w-3.5" />
            <span>用所选素材新建切片</span>
          </button>
          <button class="ui-btn ui-btn-danger" onclick={() => confirmDelete(null)}>
            <Trash2 class="h-3.5 w-3.5" />
            <span>删除选中</span>
          </button>
        </div>
      </div>
    {/if}

    <!-- Video List -->
    <div class="ui-card overflow-hidden">
      {#if loading}
        <div
          class="flex flex-col items-center justify-center space-y-3 p-12 text-[12px] text-fg-muted"
        >
          {#if importProgressInfo}
            <!-- 优化的转换进度显示 -->
            <div class="max-w-md space-y-2 text-center">
              <!-- 主要信息：醒目的转换状态 -->
              <div class="flex items-center justify-center gap-2.5">
                <RotateCw class="h-5 w-5 animate-spin text-accent-fg" />
                <span class="text-[15px] font-semibold text-accent-fg">
                  正在转换视频
                </span>
              </div>

              <!-- 副信息：文件名显示在小字部分 -->
              <div class="break-all px-4 text-[12px] text-fg-muted">
                {importProgressInfo.fileName ||
                  importProgressInfo.file_name ||
                  "正在准备..."}
              </div>
            </div>
          {:else}
            <!-- 普通加载状态 -->
            <RefreshCw class="h-6 w-6 animate-spin text-fg-faint" />
            <span>加载中...</span>
          {/if}
        </div>
      {:else if filteredVideos.length === 0}
        <div
          class="flex flex-col items-center justify-center space-y-2 p-12 text-fg-muted"
        >
          <Video class="h-10 w-10 text-fg-faint" />
          <h3 class="text-[14px] font-semibold text-fg">
            暂无素材
          </h3>
          <p class="text-[12px]">
            {selectedRoomId !== null
              ? "没有找到匹配的素材"
              : "在录播预览中选择片段生成素材，或导入外部视频"}
          </p>
        </div>
      {:else}
        <div class="overflow-x-auto custom-scrollbar-light">
          <table class="w-full table-fixed whitespace-nowrap">
            <thead>
              <tr class="border-b border-line bg-control/60">
                <th class="w-12 px-4 py-2.5 text-left">
                  <input
                    type="checkbox"
                    checked={selectedVideos.size === filteredVideos.length &&
                      filteredVideos.length > 0}
                    onchange={selectAllVideos}
                    class="ui-checkbox"
                  />
                </th>
                <th
                  class="px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted w-28"
                  >直播间</th
                >
                <th
                  class="px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted w-64"
                  >素材</th
                >
                <th
                  class="px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted w-20"
                  >备注</th
                >
                <th
                  class="px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted w-20"
                  >时长</th
                >
                <th
                  class="px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted w-24"
                  >大小</th
                >
                <th
                  class="px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted w-28"
                  >创建时间</th
                >
                <th
                  class="px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted w-52"
                  >操作</th
                >
              </tr>
            </thead>
            <tbody class="divide-y divide-line">
              {#each filteredVideos as video (video.id)}
                <tr
                  class="group transition-colors hover:bg-control-hover/50 {selectedVideos.has(
                    video.id
                  )
                    ? 'bg-accent-soft/40'
                    : ''}"
                >
                  <td class="w-12 px-4 py-2.5">
                    <input
                      type="checkbox"
                      checked={selectedVideos.has(video.id)}
                      onchange={() => toggleVideoSelection(video.id)}
                      class="ui-checkbox"
                    />
                  </td>

                  <td class="px-4 py-2.5 w-28">
                    <div class="flex items-center space-x-2">
                      {#if video.platform === "imported"}
                        <FileVideo class="table-icon text-fg-faint" />
                        <span class="text-[12px] text-fg-secondary"
                          >外部视频</span
                        >
                      {:else if video.platform === "clip"}
                        <Scissors class="table-icon text-fg-faint" />
                        <span class="text-[12px] text-fg-secondary"
                          >视频切片</span
                        >
                      {:else}
                        {@const roomUrl = liveRoomUrl(video.platform, video.room_id)}
                        {#if video.platform === "bilibili"}
                          <BilibiliIcon class="w-4 h-4 flex-shrink-0" />
                        {:else if video.platform === "douyin"}
                          <DouyinIcon class="w-4 h-4 flex-shrink-0" />
                        {:else if video.platform === "douyu"}
                          <DouyuIcon class="w-4 h-4 flex-shrink-0" />
                        {:else if video.platform === "kuaishou"}
                          <KuaishouIcon class="w-4 h-4 flex-shrink-0" />
                        {:else if video.platform === "huya"}
                          <HuyaIcon class="w-4 h-4 flex-shrink-0" />
                        {:else if video.platform === "tiktok"}
                          <TikTokIcon class="w-5 h-5 flex-shrink-0" />
                        {:else if video.platform === "twitch"}
                          <TwitchIcon class="w-5 h-5 flex-shrink-0 text-purple-600" />
                        {:else if video.platform === "youtube"}
                          <YouTubeIcon class="w-4 h-4 flex-shrink-0" />
                        {:else}
                          <Globe class="table-icon text-fg-faint" />
                        {/if}
                        {#if roomUrl}
                          <a
                            href={roomUrl}
                            target="_blank"
                            rel="noopener noreferrer"
                            class="text-[12px] text-accent-fg hover:underline"
                            title={`打开 ${platformLabel(video.platform)} 直播间`}
                          >
                            {video.room_id}
                          </a>
                        {:else}
                          <span class="text-[12px] text-fg">{video.room_id}</span>
                        {/if}
                      {/if}
                    </div>
                  </td>

                  <td class="px-4 py-2.5 w-64">
                    <div class="flex items-center space-x-3 w-full">
                      <div
                        class="flex h-8 w-12 flex-shrink-0 items-center justify-center overflow-hidden rounded-[6px] border border-line bg-sunken"
                      >
                        {#if video.cover && video.cover.trim() !== ""}
                          <img
                            src={video.cover}
                            alt="封面"
                            class="w-full h-full object-cover"
                            onerror={handleImageError}
                          />
                        {:else}
                          <!-- 默认视频图标 -->
                          <svg
                            class="h-5 w-5 text-fg-faint"
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
                          class="w-full truncate text-[12px] font-medium text-fg"
                          title={video.title || video.file}
                        >
                          {video.title || video.file}
                        </p>
                        <p
                          class="w-full truncate text-[11px] text-fg-muted"
                          title={video.file}
                        >
                          {video.file}
                        </p>
                      </div>
                    </div>
                  </td>

                  <td class="px-4 py-2.5 w-20">
                    <div class="flex items-center space-x-2">
                      <AnnotationOutline class="table-icon text-fg-faint" />
                      <span class="truncate text-[12px] text-fg-secondary"
                        >{video.note}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-2.5 w-20">
                    <div class="flex items-center space-x-2">
                      <Clock class="table-icon text-fg-faint" />
                      <span class="text-[12px] tabular-nums text-fg-secondary"
                        >{formatTimecode(video.duration || video.length, 0)}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-2.5 w-24">
                    <div class="flex items-center space-x-2">
                      <HardDrive class="table-icon text-fg-faint" />
                      <span class="text-[12px] tabular-nums text-fg-secondary"
                        >{formatSize(video.size)}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-2.5 w-28">
                    <div class="flex items-center space-x-2">
                      <Calendar class="table-icon text-fg-faint" />
                      <span class="truncate text-[12px] text-fg-secondary"
                        >{formatDate(video.created_at)}</span
                      >
                    </div>
                  </td>

                  <td class="px-4 py-2.5 w-52">
                    <div class="flex items-center gap-1">
                      <button
                        class="action-button hover:bg-accent-soft"
                        title="预览"
                        onclick={() => playVideo(video)}
                      >
                        <Play class="h-3.5 w-3.5 text-accent-fg" />
                      </button>
                      <button
                        class="action-button hover:bg-accent-soft"
                        title="用此素材新建切片"
                        disabled={creatingProject}
                        onclick={() => createProject([video])}
                      >
                        <Clapperboard class="h-3.5 w-3.5 text-accent-fg" />
                      </button>
                      <button
                        class="action-button hover:bg-accent-soft"
                        title="投稿"
                        onclick={() => requestUpload("material", video.id)}
                      >
                        <Upload class="h-3.5 w-3.5 text-accent-fg" />
                      </button>
                      <button
                        class="action-button hover:bg-success-soft"
                        title="编辑备注"
                        onclick={() => openEditNoteDialog(video)}
                      >
                        <Edit class="h-3.5 w-3.5 text-success-fg" />
                      </button>
                      {#if !TAURI_ENV}
                        <button
                          class="action-button hover:bg-accent-soft"
                          title="导出"
                          onclick={async () => await exportVideo(video)}
                        >
                          <Download class="h-3.5 w-3.5 text-accent-fg" />
                        </button>
                      {/if}
                      <button
                        class="action-button hover:bg-danger-soft"
                        title="删除"
                        onclick={() => confirmDelete(video)}
                      >
                        <Trash2 class="h-3.5 w-3.5 text-danger-fg" />
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
    <div class="ui-modal w-[400px] max-w-full">
      <div class="space-y-4 p-6">
        <div class="space-y-2 text-center">
          <h3 class="text-[14px] font-semibold text-fg">
            确认删除
          </h3>
          <p class="text-[12px] text-fg-muted">
            {#if videoToDelete}
              确定要删除素材 "{videoToDelete.title || videoToDelete.file}" 吗？
            {:else}
              确定要删除选中的 {selectedVideos.size} 个素材吗？
            {/if}
          </p>
          {#if affectedProjects.length > 0}
            <p class="text-[12px] text-fg-secondary">
              以下切片使用了这些素材，删除后会从中移除对应片段：
            </p>
            <ul class="max-h-28 overflow-y-auto text-[12px] text-fg custom-scrollbar-light">
              {#each affectedProjects as project (project.id)}
                <li class="truncate">「{project.title}」</li>
              {/each}
            </ul>
          {/if}
          <p class="text-[11px] text-danger-fg">此操作无法撤销。</p>
        </div>
        <DeleteConfirmActions
          oncancel={closeDeleteConfirm}
          onconfirm={() => videoToDelete ? deleteVideo(videoToDelete) : deleteSelectedVideos()}
        />
      </div>
    </div>
  </div>
{/if}

<!-- Edit Note Dialog -->
{#if showEditNoteDialog && videoToEditNote}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="ui-modal-backdrop" onclick={closeEditNoteDialog}>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="ui-modal w-[480px] max-w-full"
      onclick={(event) => event.stopPropagation()}
      role="dialog"
      tabindex="-1"
      aria-labelledby="edit-note-title"
      aria-describedby="edit-note-description"
    >
      <!-- Header -->
      <div class="shrink-0 border-b border-line px-[18px] py-3">
        <div class="flex items-center gap-3">
          <div
            class="flex h-8 w-8 shrink-0 items-center justify-center rounded-[8px] bg-accent-soft"
          >
            <Edit class="h-4 w-4 text-accent-fg" />
          </div>
          <div class="min-w-0 flex-1">
            <h3 id="edit-note-title" class="text-[14px] font-semibold text-fg">
              编辑素材备注
            </h3>
            <p
              id="edit-note-description"
              class="mt-0.5 truncate text-[12px] text-fg-muted"
            >
              {videoToEditNote.title || videoToEditNote.file}
            </p>
          </div>
        </div>
      </div>

      <!-- Content -->
      <div class="ui-modal-body">
        <div class="space-y-2">
          <label for="edit-note-textarea" class="ui-label block">
            备注内容
          </label>
          <div class="relative">
            <textarea
              id="edit-note-textarea"
              bind:value={editingNote}
              placeholder="为这个素材添加备注信息，如高光时刻、重要内容等..."
              class="ui-input w-full resize-none"
              rows="5"
              onkeydown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  saveNote();
                }
              }}
            ></textarea>
            <!-- Helper text -->
            <div class="ui-hint mt-1.5">
              按 ⌘+Enter 快速保存
            </div>
          </div>
        </div>
      </div>

      <!-- Footer -->
      <div class="ui-modal-footer">
        <button class="ui-btn w-20" onclick={closeEditNoteDialog}>
          取消
        </button>
        <button class="ui-btn ui-btn-primary w-20" onclick={saveNote}>
          保存
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- 导入视频对话框 -->
<ImportVideoDialog
  bind:showDialog={showImportDialog}
  roomId={selectedRoomId}
  onImported={handleVideoImported}
/>

<style>
  /* fixed icon size in tables */
  :global(.table-icon) {
    width: 14px;
    height: 14px;
    flex: 0 0 auto;
  }

  /* Row action icon buttons; the hover tint comes from a utility class. */
  .action-button {
    display: inline-flex;
    width: 26px;
    height: 26px;
    align-items: center;
    justify-content: center;
    border-radius: 7px;
    transition: background-color 120ms ease;
  }

  .action-button:focus-visible {
    outline: 2px solid rgb(var(--c-accent));
    outline-offset: 1px;
  }
</style>
