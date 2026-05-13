<script lang="ts">
  import { invoke, get_static_url } from "../invoker";
  import type { RecordItem } from "../db";
  import { fade, scale } from "svelte/transition";
  import { X, FileVideo, Info } from "lucide-svelte";
  import { createEventDispatcher, onMount } from "svelte";

  export let showModal = false;
  export let archive: RecordItem | null = null;
  export let roomId: string;
  export const platform: string = "";

  const dispatch = createEventDispatcher();

  let wholeClipArchives: RecordItem[] = [];
  let isLoading = false;
  let encodeDanmu = false;
  let selectedLiveIds: string[] = [];
  let outputName = "";
  let outputNameEdited = false;
  let showSelectionHelp = false;
  let selectionHelpRef: HTMLDivElement | null = null;

  // 当modal显示且有archive时，加载相关片段
  $: if (showModal && archive) {
    loadWholeClipArchives(roomId, archive.parent_id);
  }

  onMount(() => {
    const handleOutsideClick = (event: MouseEvent) => {
      if (!showSelectionHelp || !selectionHelpRef) {
        return;
      }
      if (!selectionHelpRef.contains(event.target as Node)) {
        showSelectionHelp = false;
      }
    };
    document.addEventListener("click", handleOutsideClick);
    return () => {
      document.removeEventListener("click", handleOutsideClick);
    };
  });

  async function loadWholeClipArchives(roomId: string, parentId: string) {
    if (isLoading) return;

    isLoading = true;
    try {
      // 获取与当前archive具有相同parent_id的所有archives
      let sameParentArchives = (await invoke("get_archives_by_parent_id", {
        roomId: roomId,
        parentId: parentId,
      })) as RecordItem[];

      // 处理封面
      for (const archive of sameParentArchives) {
        archive.cover = await get_static_url(
          "cache",
          `${archive.platform}/${archive.room_id}/${archive.live_id}/cover.jpg`
        );
      }

      // 按时间排序
      sameParentArchives.sort((a, b) => {
        return (
          new Date(a.created_at).getTime() - new Date(b.created_at).getTime()
        );
      });

      wholeClipArchives = sameParentArchives;
      selectedLiveIds = sameParentArchives.map((item) => item.live_id);
      outputNameEdited = false;
      outputName = buildDefaultOutputName();
    } catch (error) {
      console.error("Failed to load whole clip archives:", error);
      wholeClipArchives = [];
    } finally {
      isLoading = false;
    }
  }

  async function generateWholeClip() {
    try {
      await invoke("generate_whole_clip", {
        encodeDanmu: encodeDanmu,
        platform: archive.platform,
        roomId: archive.room_id,
        parentId: archive.parent_id,
        selectedLiveIds: selectedLiveIds,
        outputName: outputName.trim() ? outputName.trim() : undefined,
      });

      showModal = false;
      dispatch("generated");
    } catch (error) {
      console.error("Failed to generate whole clip:", error);
    }
  }

  function formatTimestamp(ts_string: string) {
    const date = new Date(ts_string);
    return date.toLocaleString();
  }

  function formatCompactTimestamp(date = new Date()) {
    const pad = (value: number) => value.toString().padStart(2, "0");
    return `${date.getFullYear()}${pad(date.getMonth() + 1)}${pad(
      date.getDate()
    )}${pad(date.getHours())}${pad(date.getMinutes())}${pad(date.getSeconds())}`;
  }

  function sanitizeOutputName(name: string) {
    return name.replace(/[\\/:*?"<>|]/g, "");
  }

  function buildDefaultOutputName() {
    if (!archive) {
      return "";
    }
    const timestamp = formatCompactTimestamp();
    const name = `[full][${archive.platform}][${archive.room_id}][${
      archive.parent_id
    }][${timestamp}]${archive.title}.mp4`;
    return sanitizeOutputName(name);
  }

  function selectAll() {
    selectedLiveIds = [...wholeClipArchives.map((item) => item.live_id)];
  }

  function clearSelection() {
    selectedLiveIds = [];
  }

  function selectionIndex(liveId: string) {
    return selectedLiveIds.indexOf(liveId);
  }

  function toggleArchiveSelection(liveId: string, checked: boolean) {
    if (checked) {
      if (!selectedLiveIds.includes(liveId)) {
        selectedLiveIds = [...selectedLiveIds, liveId];
      }
    } else {
      selectedLiveIds = selectedLiveIds.filter((id) => id !== liveId);
    }
  }

  function handleSelectionChange(event: Event, liveId: string) {
    const target = event.currentTarget as HTMLInputElement;
    toggleArchiveSelection(liveId, target.checked);
  }

  function formatDuration(duration: number | string) {
    const numDuration = typeof duration === 'string' ? parseFloat(duration) : duration;
    const totalSeconds = Math.max(0, Math.round(Number.isNaN(numDuration) ? 0 : numDuration));
    const hours = Math.floor(totalSeconds / 3600)
      .toString()
      .padStart(2, "0");
    const minutes = Math.floor((totalSeconds % 3600) / 60)
      .toString()
      .padStart(2, "0");
    const seconds = (totalSeconds % 60).toString().padStart(2, "0");

    return `${hours}:${minutes}:${seconds}`;
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

  $: selectedArchives = selectedLiveIds
    .map((id) => wholeClipArchives.find((item) => item.live_id === id))
    .filter((item): item is RecordItem => Boolean(item));

  $: totalDuration = selectedArchives.reduce(
    (sum, item) => sum + item.length,
    0
  );

  $: totalSize = selectedArchives.reduce((sum, item) => sum + item.size, 0);

  function closeModal() {
    showModal = false;
    wholeClipArchives = [];
    selectedLiveIds = [];
    outputName = "";
    outputNameEdited = false;
    showSelectionHelp = false;
  }
</script>

{#if showModal}
  <div
    class="fixed inset-0 bg-black/20 dark:bg-black/40 backdrop-blur-sm z-50 flex items-center justify-center"
    transition:fade={{ duration: 200 }}
    on:click={closeModal}
    on:keydown={(e) => e.key === "Escape" && closeModal()}
    role="dialog"
    aria-modal="true"
  >
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div
      class="mac-modal w-[900px] bg-white dark:bg-[#323234] rounded-xl shadow-xl overflow-hidden flex flex-col max-h-[90vh]"
      transition:scale={{ duration: 150, start: 0.95 }}
      on:click|stopPropagation
    >
      <!-- Header -->
      <div
        class="flex justify-between items-center px-6 py-4 border-b border-gray-200 dark:border-gray-700/50"
      >
        <div class="flex items-center space-x-3">
          <h2 class="text-base font-medium text-gray-900 dark:text-white">
            生成完整直播切片
          </h2>
          <span class="text-sm text-gray-500 dark:text-gray-400">
            {archive?.title || "直播片段"}
          </span>
        </div>
        <button
          class="p-1.5 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-700/50 transition-colors"
          on:click={closeModal}
        >
          <X class="w-5 h-5 dark:icon-white" />
        </button>
      </div>

      <!-- Content -->
      <div class="flex-1 flex flex-col min-h-0">
        <!-- Main Content: Left (List) -->
        <div class="flex-1 flex min-h-0">
          <!-- Left: Scrollable List -->
          <div class="flex-1 overflow-auto custom-scrollbar-light px-6 py-6 min-h-0">
            {#if isLoading}
              <div class="flex items-center justify-center py-8">
                <div
                  class="flex items-center space-x-2 text-gray-500 dark:text-gray-400"
                >
                  <div
                    class="animate-spin rounded-full h-5 w-5 border-b-2 border-blue-500"
                  ></div>
                  <span>加载中...</span>
                </div>
              </div>
            {:else if wholeClipArchives.length === 0}
              <div class="text-center py-8 text-gray-500 dark:text-gray-400">
                未找到相关片段
              </div>
            {:else}
              <div class="flex items-center justify-between pb-4">
                <div class="text-sm text-gray-600 dark:text-gray-400">
                  默认全选，按勾选顺序合成
                </div>
                <div class="flex items-center space-x-3 text-sm">
                  <button
                    class="text-blue-600 hover:text-blue-700 dark:text-blue-400 dark:hover:text-blue-300"
                    on:click={selectAll}
                  >
                    全选
                  </button>
                  <button
                    class="text-gray-500 hover:text-gray-700 dark:text-gray-300 dark:hover:text-gray-100"
                    on:click={clearSelection}
                  >
                    取消全选
                  </button>
                  <div class="relative" bind:this={selectionHelpRef}>
                    <button
                      class="text-gray-500 hover:text-gray-700 dark:text-gray-300 dark:hover:text-gray-100"
                      on:click={() =>
                        (showSelectionHelp = !showSelectionHelp)}
                      on:mouseenter={() => (showSelectionHelp = true)}
                      on:mouseleave={() => (showSelectionHelp = false)}
                      aria-label="选择顺序说明"
                    >
                      <Info class="w-4 h-4" />
                    </button>
                    {#if showSelectionHelp}
                      <div
                        class="absolute right-0 top-6 z-20 w-64 rounded-lg border border-gray-200 bg-white p-3 text-xs text-gray-700 shadow-lg dark:border-gray-700 dark:bg-[#2c2c2e] dark:text-gray-200"
                      >
                        取消全选后请按需要勾选片段，勾选的先后顺序即合成顺序。
                      </div>
                    {/if}
                  </div>
                </div>
              </div>
              <div class="space-y-2 pb-4">
                {#each wholeClipArchives as archiveItem, index (archiveItem.live_id)}
                  <div
                    class="flex items-center space-x-3 p-3 rounded-lg bg-gray-50 dark:bg-gray-700/30"
                  >
                    <input
                      type="checkbox"
                      class="h-4 w-4 text-blue-600 rounded border-gray-300 focus:ring-blue-500"
                      checked={selectedLiveIds.includes(archiveItem.live_id)}
                      on:change={(event) =>
                        handleSelectionChange(event, archiveItem.live_id)}
                    />
                    <div class="flex items-center justify-center w-6">
                      {#if selectedLiveIds.includes(archiveItem.live_id)}
                        <div
                          class="w-6 h-6 rounded-full bg-blue-500 flex items-center justify-center text-white text-xs font-bold shadow-sm"
                        >
                          {selectedLiveIds.indexOf(archiveItem.live_id) + 1}
                        </div>
                      {:else}
                        <span class="text-xs text-gray-400 dark:text-gray-500 font-medium">
                          {index + 1}
                        </span>
                      {/if}
                    </div>

                    {#if archiveItem.cover}
                      <img
                        src={archiveItem.cover}
                        alt="cover"
                        class="w-14 h-9 rounded object-cover flex-shrink-0"
                      />
                    {/if}

                    <div class="flex-1 min-w-0">
                      <div
                        class="text-sm font-medium text-gray-900 dark:text-white truncate"
                      >
                        {archiveItem.title}
                      </div>
                      <div class="text-xs text-gray-500 dark:text-gray-400 mt-1">
                        开始时间 {formatTimestamp(archiveItem.created_at)}
                      </div>
                      <div class="text-xs text-gray-500 dark:text-gray-400">
                        时长 {formatDuration(archiveItem.length)} · 大小 {formatSize(
                          archiveItem.size
                        )}
                      </div>
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        </div>

        <!-- Bottom Summary -->
        {#if !isLoading && wholeClipArchives.length > 0}
          <div class="px-6 pb-4">
            <div
              class="p-3 rounded-lg bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 w-full"
            >
              <div class="flex items-center justify-between">
                <div class="flex items-center space-x-2">
                  <FileVideo class="w-4 h-4 text-blue-600 dark:text-blue-400" />
                  <span
                    class="text-sm font-medium text-blue-900 dark:text-blue-100"
                  >
                    合成信息
                  </span>
                </div>
                <div class="text-xs text-blue-600 dark:text-blue-300">
                  已选 {selectedArchives.length} 个片段 · 总时长 {formatDuration(totalDuration)} · 总大小 {formatSize(totalSize)}
                </div>
              </div>
              <div class="flex items-center gap-4 mt-3">
                <div class="flex-1">
                  <input
                    type="text"
                    class="w-full px-3 py-1.5 text-sm rounded-lg border border-blue-200 dark:border-blue-700 bg-white dark:bg-[#2c2c2e] text-gray-900 dark:text-gray-100"
                    bind:value={outputName}
                    on:input={() => (outputNameEdited = true)}
                    placeholder="输出文件名"
                  />
                </div>
                <div class="flex items-center space-x-2">
                  <span class="text-xs text-blue-600 dark:text-blue-300">压制弹幕</span>
                  <label class="relative inline-flex items-center cursor-pointer">
                    <input
                      type="checkbox"
                      bind:checked={encodeDanmu}
                      class="sr-only peer"
                    />
                    <div
                      class="w-11 h-6 bg-blue-200 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-blue-300 dark:peer-focus:ring-blue-800 rounded-full peer dark:bg-blue-700 peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-blue-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all dark:border-blue-600 peer-checked:bg-blue-600"
                    ></div>
                  </label>
                </div>
              </div>
            </div>
          </div>
        {/if}
      </div>

      <!-- Footer -->
      <div
        class="px-6 py-4 border-t border-gray-200 dark:border-gray-700/50 flex justify-end space-x-3"
      >
        <button
          class="px-4 py-2 text-sm font-medium text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-600 rounded-lg transition-colors"
          on:click={closeModal}
        >
          取消
        </button>
        <button
          class="px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          disabled={isLoading || selectedArchives.length === 0}
          on:click={generateWholeClip}
        >
          开始合成
        </button>
      </div>
    </div>
  </div>
{/if}

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
</style>
