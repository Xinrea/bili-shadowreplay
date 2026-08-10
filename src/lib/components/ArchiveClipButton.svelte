<script lang="ts">
  import { createEventDispatcher, onDestroy, onMount } from "svelte";
  import { Loader2, Scissors } from "lucide-svelte";
  import type { RecordItem } from "../db";
  import {
    clipRange,
    generateEventId,
    type Range,
    type VideoItem,
  } from "../interface";
  import { invoke, listen, log } from "../invoker";

  export let archive: RecordItem | null = null;
  export let ranges: Range[] = [];
  export let initialNote = "";
  export let localOffset: number | null = null;
  export let captureCover = false;
  export let variant: "primary" | "compact" = "primary";
  export let disabled = false;
  export let running = false;

  const dispatch = createEventDispatcher<{
    generated: VideoItem;
    failed: string;
  }>();

  const transitionOptions = [
    { value: "none", label: "无" },
    { value: "fade", label: "淡入淡出" },
    { value: "dissolve", label: "溶解" },
    { value: "wipeleft", label: "向左擦除" },
    { value: "wiperight", label: "向右擦除" },
    { value: "slideup", label: "向上滑动" },
    { value: "slidedown", label: "向下滑动" },
  ];

  let showConfirm = false;
  let currentEventId: string | null = null;
  let progressText = "生成切片";
  let clipNote = "";
  let danmuEnabled = false;
  let fixEncoding = false;
  let transition = "none";
  let clearUpdateListener: (() => void) | null = null;
  let clearFinishedListener: (() => void) | null = null;

  $: activeRanges = ranges.filter((range) => range.activated !== false);

  function openConfirm() {
    if (!archive || disabled || currentEventId) return;
    clipNote = initialNote;
    showConfirm = true;
  }

  function closeConfirm() {
    showConfirm = false;
  }

  function formatTime(seconds: number) {
    const total = Math.max(0, Math.round(seconds));
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const secs = total % 60;
    return [hours, minutes, secs]
      .map((part) => part.toString().padStart(2, "0"))
      .join(":");
  }

  function formatDuration(seconds: number) {
    const total = Math.max(0, Math.round(seconds));
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const secs = total % 60;
    const parts: string[] = [];
    if (hours > 0) parts.push(`${hours} 小时`);
    if (minutes > 0) parts.push(`${minutes} 分`);
    parts.push(`${secs} 秒`);
    return parts.join(" ");
  }

  function getCover() {
    if (!captureCover) return "";
    const video = document.getElementById("video") as HTMLVideoElement | null;
    if (!video || !video.videoWidth || !video.videoHeight) return "";
    const canvas = document.createElement("canvas");
    canvas.width = 1280;
    canvas.height = 720;
    const context = canvas.getContext("2d");
    if (!context) return "";
    context.drawImage(
      video,
      0,
      0,
      video.videoWidth,
      video.videoHeight,
      0,
      0,
      canvas.width,
      canvas.height,
    );
    return canvas.toDataURL();
  }

  function clearListeners() {
    clearUpdateListener?.();
    clearUpdateListener = null;
    clearFinishedListener?.();
    clearFinishedListener = null;
  }

  function resetTaskState(eventId?: string) {
    if (eventId && currentEventId !== eventId) return;
    clearListeners();
    currentEventId = null;
    running = false;
    progressText = "生成切片";
  }

  async function generateClip() {
    if (!archive || activeRanges.length === 0 || currentEventId) return;
    showConfirm = false;
    const eventId = generateEventId();
    currentEventId = eventId;
    running = true;
    progressText = "切片生成中";

    clearUpdateListener = await listen(`progress-update:${eventId}`, (event) => {
      progressText = event.payload.content;
    });
    clearFinishedListener = await listen(
      `progress-finished:${eventId}`,
      (event) => {
        if (!event.payload.success) {
          const message = String(event.payload.message || "未知错误");
          alert("请检查 ffmpeg 是否配置正确：" + message);
          dispatch("failed", message);
        }
        resetTaskState(eventId);
      },
    );

    try {
      const video = await clipRange(eventId, {
        title: archive.title,
        source_date: archive.created_at,
        note: clipNote,
        room_id: archive.room_id,
        platform: archive.platform,
        cover: getCover(),
        live_id: archive.live_id,
        ranges: activeRanges,
        danmu: danmuEnabled,
        local_offset:
          localOffset ??
          (parseInt(
            localStorage.getItem(`local_offset:${archive.live_id}`) || "0",
            10,
          ) || 0),
        fix_encoding: fixEncoding,
        transition: transition !== "none" ? transition : undefined,
      });
      clipNote = "";
      transition = "none";
      dispatch("generated", video as VideoItem);
    } catch (error) {
      if (currentEventId === eventId) {
        const message = String(error);
        alert("切片生成失败：" + message);
        dispatch("failed", message);
      }
    } finally {
      resetTaskState(eventId);
    }
  }

  async function cancelClip() {
    if (!currentEventId) return;
    const eventId = currentEventId;
    try {
      await invoke("cancel", { eventId });
    } catch (error) {
      log.warn("Failed to cancel clip task", error);
    } finally {
      resetTaskState(eventId);
    }
  }

  function handleBeforeUnload(event: BeforeUnloadEvent) {
    if (!currentEventId) return;
    const message = "切片任务将在后台继续运行，可前往任务页面管理后台任务。";
    event.preventDefault();
    event.returnValue = message;
    return message;
  }

  onMount(() => window.addEventListener("beforeunload", handleBeforeUnload));
  onDestroy(() => {
    window.removeEventListener("beforeunload", handleBeforeUnload);
    clearListeners();
  });
</script>

<div class={variant === "compact" ? "contents" : "flex items-center gap-2"}>
    <button
      type="button"
      on:click={openConfirm}
      disabled={disabled || !archive || currentEventId !== null}
      class={variant === "compact"
        ? "inline-flex items-center gap-1.5 whitespace-nowrap rounded-lg border border-violet-200 bg-white px-2.5 py-1.5 text-xs font-medium text-violet-700 transition-colors hover:bg-violet-100 disabled:cursor-not-allowed disabled:opacity-50 dark:border-violet-700 dark:bg-gray-900 dark:text-violet-300 dark:hover:bg-violet-950/60"
        : "inline-flex items-center gap-2 rounded-lg bg-[#0A84FF] px-4 py-1.5 text-sm text-white transition-all duration-200 hover:bg-[#0A84FF]/90 disabled:cursor-not-allowed disabled:opacity-50"}
    >
      {#if currentEventId}
        <Loader2 class="h-4 w-4 animate-spin" />
      {:else if variant === "compact"}
        <Scissors class="h-3.5 w-3.5" />
      {/if}
      <span>{progressText}</span>
    </button>

    {#if currentEventId && variant === "primary"}
      <button
        type="button"
        on:click={cancelClip}
        class="rounded-lg px-4 py-1.5 text-sm text-red-500 transition-all duration-200 hover:bg-red-500/10"
      >
        取消
      </button>
    {/if}
</div>

{#if showConfirm}
  <div class="fixed inset-0 z-[100] flex items-center justify-center">
    <div
      class="absolute inset-0 bg-black/60 backdrop-blur-md"
      role="button"
      tabindex="0"
      aria-label="关闭对话框"
      on:click={closeConfirm}
      on:keydown={(event) => {
        if (event.key === "Escape" || event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          closeConfirm();
        }
      }}
    />

    <div
      role="dialog"
      aria-modal="true"
      class="relative mx-4 w-full max-w-md rounded-2xl border border-white/10 bg-[#1c1c1e] shadow-2xl ring-1 ring-black/5"
    >
      <div class="p-5">
        <h3 class="text-[17px] font-semibold text-white">确认生成切片</h3>
        <p class="mt-1 text-[13px] text-white/70">请确认以下设置后继续</p>

        <div class="mt-3 space-y-3">
          <div class="text-[13px] font-medium text-white/90">待合并选区列表</div>
          <div class="max-h-48 space-y-2 overflow-y-auto custom-scrollbar-light">
            {#each activeRanges as range, index}
              <div class="flex items-center justify-between rounded-lg border border-white/5 bg-[#2c2c2e] px-3 py-2 transition-colors hover:border-white/10">
                <div class="flex items-center space-x-3">
                  <div class="flex h-6 w-6 items-center justify-center rounded-full bg-[#0A84FF]/20 text-[11px] font-semibold text-[#0A84FF]">
                    {index + 1}
                  </div>
                  <div class="flex flex-col space-y-0.5">
                    <div class="text-[12px] text-white/90">
                      {formatTime(range.start)} → {formatTime(range.end)}
                    </div>
                    <div class="text-[11px] text-white/60">
                      时长: {formatDuration(range.end - range.start)}
                    </div>
                  </div>
                </div>
              </div>
            {:else}
              <div class="py-4 text-center text-[13px] text-white/60">
                没有已激活的选区，请选择或添加选区。
              </div>
            {/each}
          </div>
          <div class="mt-2 border-t border-white/10 pt-2 text-[15px] font-semibold text-white">
            总时长: {formatDuration(
              activeRanges.reduce(
                (total, range) => total + range.end - range.start,
                0,
              ),
            )}
          </div>
        </div>

        <div class="mt-3 space-y-3">
          <label for="archive-clip-note" class="mt-1 block text-[13px] text-white/80">
            切片备注（可选）
          </label>
          <input
            id="archive-clip-note"
            type="text"
            bind:value={clipNote}
            class="w-full rounded-lg border border-gray-800/50 bg-[#2c2c2e] px-3 py-2 text-white outline-none transition duration-200 placeholder-gray-500 focus:border-[#0A84FF]"
          />
        </div>

        <div class="mt-3 space-y-3">
          <label class="flex items-center gap-2.5">
            <input
              type="checkbox"
              bind:checked={danmuEnabled}
              class="h-4 w-4 rounded border-white/30 bg-[#2c2c2e] text-[#0A84FF] accent-[#0A84FF] focus:outline-none focus:ring-2 focus:ring-[#0A84FF]/40"
            />
            <span class="text-[13px] text-white/80">压制弹幕</span>
          </label>

          <label class="flex items-center gap-2.5">
            <input
              type="checkbox"
              bind:checked={fixEncoding}
              class="h-4 w-4 rounded border-white/30 bg-[#2c2c2e] text-[#0A84FF] accent-[#0A84FF] focus:outline-none focus:ring-2 focus:ring-[#0A84FF]/40"
            />
            <span class="text-[13px] text-white/80">修复编码（切片异常时使用）</span>
          </label>
        </div>

        {#if activeRanges.length > 1}
          <div class="mt-3 space-y-2">
            <div class="text-[13px] font-medium text-white/90">转场效果</div>
            <div class="grid grid-cols-4 gap-1.5">
              {#each transitionOptions as option}
                <button
                  type="button"
                  class="rounded-lg border px-2 py-1.5 text-[12px] transition-colors {transition === option.value
                    ? 'border-[#0A84FF] bg-[#0A84FF]/20 text-[#0A84FF]'
                    : 'border-white/5 bg-[#2c2c2e] text-white/70 hover:border-white/20'}"
                  on:click={() => (transition = option.value)}
                >
                  {option.label}
                </button>
              {/each}
            </div>
          </div>
        {/if}
      </div>

      <div class="flex items-center justify-end gap-2 rounded-b-2xl border-t border-white/10 bg-[#111113] px-5 py-3">
        <button
          type="button"
          on:click={closeConfirm}
          class="rounded-lg border border-white/20 px-3.5 py-2 text-[13px] text-white/90 transition-colors hover:bg-white/10"
        >
          取消
        </button>
        <button
          type="button"
          on:click={generateClip}
          disabled={activeRanges.length === 0}
          class="rounded-lg bg-[#0A84FF] px-3.5 py-2 text-[13px] text-white shadow-[inset_0_1px_0_rgba(255,255,255,.15)] transition-colors hover:bg-[#0A84FF]/90 disabled:cursor-not-allowed disabled:opacity-50"
        >
          确认生成
        </button>
      </div>
    </div>
  </div>
{/if}
