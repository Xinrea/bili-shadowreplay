<script lang="ts">
  import { invoke, get_static_url } from "../invoker";
  import { formatSize } from "../size-format";
  import type { RecordItem } from "../db";
  import { fade, scale } from "svelte/transition";
  import { X, FileVideo, Info } from "lucide-svelte";
  import { untrack } from "svelte";
  import { clickOutside } from "../actions/clickOutside";
  import { defaultRenderSettings } from "../project-timeline";
  import RenderSettingsFields from "./RenderSettingsFields.svelte";

  interface Props {
    showModal?: boolean;
    archive?: RecordItem | null;
    roomId: string;
    onGenerated?: () => void;
  }

  let { showModal = $bindable(false), archive = null, roomId, onGenerated }: Props = $props();
  export const platform: string = "";

  let wholeClipArchives: RecordItem[] = $state([]);
  let isLoading = $state(false);
  let encodeDanmu = $state(false);
  let reencode = $state(false);
  let selectedLiveIds: string[] = $state([]);
  let outputName = $state("");
  let showSelectionHelp = $state(false);
  let renderSettings = $state(defaultRenderSettings());
  let isSubmitting = $state(false);
  let generationError = $state("");
  let loadSeq = 0;


  async function loadWholeClipArchives(roomId: string, parentId: string) {
    const seq = ++loadSeq;
    isLoading = true;
    try {
      // 获取与当前archive具有相同parent_id的所有archives
      let sameParentArchives = (await invoke("get_archives_by_parent_id", {
        roomId: roomId,
        parentId: parentId,
      })) as RecordItem[];

      if (seq !== loadSeq) return;

      // 处理封面
      for (const archive of sameParentArchives) {
        if (seq !== loadSeq) return;
        archive.cover = await get_static_url(
          "cache",
          `${archive.platform}/${archive.room_id}/${archive.live_id}/cover.jpg`
        );
      }

      if (seq !== loadSeq) return;

      // 按时间排序
      sameParentArchives.sort((a, b) => {
        return (
          new Date(a.created_at).getTime() - new Date(b.created_at).getTime()
        );
      });

      wholeClipArchives = sameParentArchives;
      selectedLiveIds = sameParentArchives.map((item) => item.live_id);
      outputName = buildDefaultOutputName();
    } catch (error) {
      console.error("Failed to load whole clip archives:", error);
      if (seq === loadSeq) {
        wholeClipArchives = [];
      }
    } finally {
      if (seq === loadSeq) {
        isLoading = false;
      }
    }
  }

  async function generateWholeClip() {
    if (!archive || isLoading || isSubmitting || selectedLiveIds.length === 0) return;
    isSubmitting = true;
    generationError = "";
    try {
      await invoke("generate_whole_clip", {
        encodeDanmu: encodeDanmu,
        platform: archive.platform,
        roomId: archive.room_id,
        parentId: archive.parent_id,
        selectedLiveIds: [...selectedLiveIds],
        outputName: outputName.trim() ? outputName.trim() : undefined,
        renderSettings: encodeDanmu || reencode ? { ...renderSettings } : undefined,
      });

      showModal = false;
      onGenerated?.();
    } catch (error) {
      console.error("Failed to generate whole clip:", error);
      generationError = "生成完整录播失败：" + String(error);
    } finally {
      isSubmitting = false;
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




  function closeModal() {
    if (isSubmitting) return;
    loadSeq++;
    showModal = false;
    isLoading = false;
    wholeClipArchives = [];
    selectedLiveIds = [];
    outputName = "";
    showSelectionHelp = false;
    generationError = "";
  }
  // 当modal显示且有archive时，加载相关片段。
  // 用 untrack 包住加载，避免 isLoading 等内部状态把 $effect 重新触发成死循环。
  $effect(() => {
    if (!showModal || !archive) {
      return;
    }
    const parentId = archive.parent_id;
    const currentRoomId = roomId;
    untrack(() => {
      void loadWholeClipArchives(currentRoomId, parentId);
    });
  });
  let selectedArchives = $derived(selectedLiveIds
    .map((id) => wholeClipArchives.find((item) => item.live_id === id))
    .filter((item): item is RecordItem => Boolean(item)));
  let totalDuration = $derived(selectedArchives.reduce(
    (sum, item) => sum + item.length,
    0
  ));
  let totalSize = $derived(selectedArchives.reduce((sum, item) => sum + item.size, 0));
</script>

{#if showModal}
  <div
    class="ui-modal-backdrop"
    transition:fade={{ duration: 200 }}
    onclick={closeModal}
    onkeydown={(e) => e.key === "Escape" && closeModal()}
    role="dialog"
    tabindex="-1"
    aria-modal="true"
  >
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="ui-modal max-h-[90vh] w-[900px] max-w-full"
      transition:scale={{ duration: 150, start: 0.95 }}
      onclick={(event) => event.stopPropagation()}
    >
      <!-- Header -->
      <div class="ui-modal-header">
        <div class="flex min-w-0 items-baseline gap-2.5">
          <h2 class="shrink-0">生成完整录播</h2>
          <span class="truncate text-[12px] font-normal text-fg-muted">
            {archive?.title || "直播片段"}
          </span>
        </div>
        <button
          class="ui-btn ui-btn-ghost ui-btn-sm ui-btn-icon -mr-1.5"
          onclick={closeModal}
          disabled={isSubmitting}
          aria-label="关闭生成完整录播窗口"
        >
          <X class="h-4 w-4" />
        </button>
      </div>

      <!-- Content -->
      <div class="ui-modal-body custom-scrollbar-light space-y-4 [scrollbar-gutter:stable]">
        <div class="space-y-4">
          <!-- One scroll container keeps the list and settings aligned. -->
          <div class="min-w-0">
            {#if isLoading}
              <div class="flex items-center justify-center py-8">
                <div class="flex items-center space-x-2 text-[12px] text-fg-muted">
                  <div
                    class="h-5 w-5 animate-spin rounded-full border-b-2 border-accent"
                  ></div>
                  <span>加载中...</span>
                </div>
              </div>
            {:else if wholeClipArchives.length === 0}
              <div class="py-8 text-center text-[12px] text-fg-muted">
                未找到相关片段
              </div>
            {:else}
              <div class="flex flex-wrap items-center justify-between gap-2 pb-3">
                <div class="text-[12px] text-fg-muted">
                  默认全选，按勾选顺序合成
                </div>
                <div class="flex items-center space-x-3 text-[12px]">
                  <button
                    class="font-medium text-accent-fg transition-opacity hover:opacity-80"
                    onclick={selectAll}
                  >
                    全选
                  </button>
                  <button
                    class="text-fg-muted transition-colors hover:text-fg"
                    onclick={clearSelection}
                  >
                    取消全选
                  </button>
                  <div class="relative" use:clickOutside={() => (showSelectionHelp = false)}>
                    <button
                      class="flex items-center text-fg-muted transition-colors hover:text-fg"
                      onclick={() =>
                        (showSelectionHelp = !showSelectionHelp)}
                      onmouseenter={() => (showSelectionHelp = true)}
                      onmouseleave={() => (showSelectionHelp = false)}
                      aria-label="选择顺序说明"
                    >
                      <Info class="h-4 w-4" />
                    </button>
                    {#if showSelectionHelp}
                      <div
                        class="absolute right-0 top-6 z-20 w-64 rounded-[8px] border border-line bg-surface p-3 text-[11px] leading-relaxed text-fg-secondary shadow-xl"
                      >
                        取消全选后请按需要勾选片段，勾选的先后顺序即合成顺序。
                      </div>
                    {/if}
                  </div>
                </div>
              </div>
              <div class="space-y-2">
                {#each wholeClipArchives as archiveItem, index (archiveItem.live_id)}
                  <div
                    class="flex items-center gap-3 rounded-[10px] border bg-control p-3 transition-colors {selectedLiveIds.includes(
                      archiveItem.live_id
                    )
                      ? 'border-accent/50'
                      : 'border-transparent'}"
                  >
                    <input
                      type="checkbox"
                      class="ui-checkbox shrink-0"
                      aria-label={`选择片段 ${index + 1}`}
                      checked={selectedLiveIds.includes(archiveItem.live_id)}
                      onchange={(event) =>
                        handleSelectionChange(event, archiveItem.live_id)}
                    />
                    <div class="flex w-5 shrink-0 items-center justify-center">
                      {#if selectedLiveIds.includes(archiveItem.live_id)}
                        <div
                          class="flex h-5 w-5 items-center justify-center rounded-full bg-accent text-[10px] font-semibold tabular-nums text-white"
                        >
                          {selectedLiveIds.indexOf(archiveItem.live_id) + 1}
                        </div>
                      {:else}
                        <span class="text-[11px] font-medium tabular-nums text-fg-faint">
                          {index + 1}
                        </span>
                      {/if}
                    </div>

                    {#if archiveItem.cover}
                      <img
                        src={archiveItem.cover}
                        alt=""
                        class="h-9 w-14 flex-shrink-0 rounded-[6px] bg-sunken object-cover"
                      />
                    {/if}

                    <div class="min-w-0 flex-1 sm:grid sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center sm:gap-x-5">
                      <div class="min-w-0">
                        <div class="truncate text-[12px] font-medium text-fg" title={archiveItem.title}>
                          {archiveItem.title}
                        </div>
                        <div class="mt-1 text-[11px] tabular-nums text-fg-muted">
                          开始时间 {formatTimestamp(archiveItem.created_at)}
                        </div>
                      </div>
                      <dl class="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-[11px] sm:mt-0 sm:gap-x-6 sm:text-right">
                        <div class="sm:min-w-[64px]">
                          <dt class="text-fg-muted">时长</dt>
                          <dd class="mt-0.5 tabular-nums text-fg-secondary">{formatDuration(archiveItem.length)}</dd>
                        </div>
                        <div class="sm:min-w-[72px]">
                          <dt class="text-fg-muted">大小</dt>
                          <dd class="mt-0.5 tabular-nums text-fg-secondary">{formatSize(archiveItem.size)}</dd>
                        </div>
                      </dl>
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
          <div class="min-w-0 space-y-3 border-t border-line pt-4">
            <div class="flex items-center justify-between gap-4">
              <label class="flex items-center gap-2 text-[12px] text-fg-secondary">
                <input
                  type="checkbox"
                  class="ui-checkbox"
                  checked={encodeDanmu || reencode}
                  onchange={(event) => (reencode = event.currentTarget.checked)}
                  disabled={encodeDanmu || isLoading || isSubmitting}
                />
                <span>重新编码</span>
              </label>
              <label class="flex cursor-pointer items-center gap-2 text-[12px] text-fg-secondary">
                <span>内嵌弹幕</span>
                <span class="ui-switch">
                  <input
                    type="checkbox"
                    class="left-0 top-0"
                    bind:checked={encodeDanmu}
                    disabled={isLoading || isSubmitting}
                  />
                  <span></span>
                </span>
              </label>
            </div>
            <p class="ui-hint">
              {encodeDanmu
                ? "内嵌弹幕需要重新编码，无法取消"
                : reencode
                  ? "按以下设置重新编码，耗时较长"
                  : "保留原始编码，直接复制并拼接片段"}
            </p>
            <RenderSettingsFields layout="columns" bind:value={renderSettings} disabled={(!encodeDanmu && !reencode) || isLoading || isSubmitting} />
          </div>
        </div>

        <!-- Bottom Summary -->
        {#if !isLoading && wholeClipArchives.length > 0}
          <label class="block space-y-1.5">
            <span class="flex flex-wrap items-center justify-between gap-2">
              <span class="flex items-center gap-2 text-[12px] font-semibold text-fg">
                <FileVideo class="h-4 w-4 text-accent-fg" />
                输出文件名
              </span>
              <span class="text-[11px] tabular-nums text-fg-muted">
                已选 {selectedArchives.length} 个片段 · 总时长 {formatDuration(totalDuration)} · 总大小 {formatSize(totalSize)}
              </span>
            </span>
            <input
              type="text"
              class="ui-input w-full"
              bind:value={outputName}
              aria-label="输出文件名"
              placeholder="输出文件名"
            />
          </label>
        {/if}
        {#if generationError}
          <p class="text-[12px] text-red-500" role="alert">{generationError}</p>
        {/if}
      </div>

      <!-- Footer -->
      <div class="ui-modal-footer">
        <button class="ui-btn" onclick={closeModal} disabled={isSubmitting}>
          取消
        </button>
        <button
          class="ui-btn ui-btn-primary"
          disabled={isLoading || isSubmitting || selectedArchives.length === 0}
          onclick={generateWholeClip}
        >
          {isSubmitting ? "正在提交…" : "开始合成"}
        </button>
      </div>
    </div>
  </div>
{/if}
