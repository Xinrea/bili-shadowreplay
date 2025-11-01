<script lang="ts">
  import { invoke, get_cover } from "../invoker";
  import type { RecordItem } from "../db";
  import { fade, scale } from "svelte/transition";
  import { X, FileVideo } from "lucide-svelte";
  import { createEventDispatcher } from "svelte";

  export let showModal = false;
  export let archive: RecordItem | null = null;
  export let roomId: string;
  export const platform: string = "";

  const dispatch = createEventDispatcher();

  let wholeClipArchives: RecordItem[] = [];
  let isLoading = false;
  let encodeDanmu = false;

  // 当modal显示且有archive时，加载相关片段
  $: if (showModal && archive) {
    loadWholeClipArchives(roomId, archive.parent_id);
  }

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
        archive.cover = await get_cover(
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

  function formatDuration(duration: number) {
    const hours = Math.floor(duration / 3600)
      .toString()
      .padStart(2, "0");
    const minutes = Math.floor((duration % 3600) / 60)
      .toString()
      .padStart(2, "0");
    const seconds = (duration % 60).toString().padStart(2, "0");

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

  function closeModal() {
    showModal = false;
    wholeClipArchives = [];
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
      class="mac-modal w-[800px] bg-white dark:bg-[#323234] rounded-xl shadow-xl overflow-hidden flex flex-col max-h-[80vh]"
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
        <!-- Description -->
        <div class="px-6 pt-6 pb-4">
          <p class="text-sm text-gray-600 dark:text-gray-400">
            以下是属于同一场直播的所有片段，将按时间顺序合成为一个完整的视频文件：
          </p>
        </div>

        <!-- Main Content: Left (List) + Right (Summary) -->
        <div class="flex-1 flex min-h-0">
          <!-- Left: Scrollable List -->
          <div class="flex-1 overflow-auto custom-scrollbar-light px-6 min-h-0">
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
              <div class="space-y-3 pb-4">
                {#each wholeClipArchives as archiveItem, index (archiveItem.live_id)}
                  <div
                    class="flex items-center space-x-4 p-4 rounded-lg bg-gray-50 dark:bg-gray-700/30"
                  >
                    <div
                      class="flex-shrink-0 w-8 h-8 rounded-full bg-blue-500 flex items-center justify-center text-white text-sm font-medium"
                    >
                      {index + 1}
                    </div>

                    {#if archiveItem.cover}
                      <img
                        src={archiveItem.cover}
                        alt="cover"
                        class="w-16 h-10 rounded object-cover flex-shrink-0"
                      />
                    {/if}

                    <div class="flex-1 min-w-0">
                      <div
                        class="text-sm font-medium text-gray-900 dark:text-white truncate"
                      >
                        {archiveItem.title}
                      </div>
                      <div
                        class="text-xs text-gray-500 dark:text-gray-400 mt-1"
                      >
                        {formatTimestamp(archiveItem.created_at)} · {formatDuration(
                          archiveItem.length
                        )} · {formatSize(archiveItem.size)}
                      </div>
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          </div>

          <!-- Right: Fixed Summary -->
          {#if !isLoading && wholeClipArchives.length > 0}
            <div class="w-80 px-6 pb-6 flex-shrink-0 flex items-center">
              <div
                class="p-4 rounded-lg bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 w-full"
              >
                <div class="flex items-center space-x-2 mb-2">
                  <FileVideo class="w-4 h-4 text-blue-600 dark:text-blue-400" />
                  <span
                    class="text-sm font-medium text-blue-900 dark:text-blue-100"
                    >合成信息</span
                  >
                </div>
                <div class="text-sm text-blue-800 dark:text-blue-200">
                  共 {wholeClipArchives.length} 个片段 · 总时长 {formatDuration(
                    wholeClipArchives.reduce(
                      (sum, archiveItem) => sum + archiveItem.length,
                      0
                    )
                  )} · 总大小 {formatSize(
                    wholeClipArchives.reduce(
                      (sum, archiveItem) => sum + archiveItem.size,
                      0
                    )
                  )}
                </div>

                <!-- 压制弹幕选项 -->
                <div
                  class="mt-4 pt-4 border-t border-blue-200 dark:border-blue-700"
                >
                  <div class="flex items-center justify-between">
                    <div class="flex-1">
                      <div
                        class="text-sm font-medium text-blue-900 dark:text-blue-100"
                      >
                        压制弹幕
                      </div>
                      <div
                        class="text-xs text-blue-700 dark:text-blue-300 mt-1"
                      >
                        将弹幕直接压制到视频中，生成包含弹幕的最终视频文件
                      </div>
                    </div>
                    <label
                      class="relative inline-flex items-center cursor-pointer"
                    >
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
          disabled={isLoading || wholeClipArchives.length === 0}
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
