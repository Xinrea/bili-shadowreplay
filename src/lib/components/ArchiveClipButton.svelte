<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { Scissors } from "lucide-svelte";
  import type { RecordItem } from "../db";
  import {
    clipRange,
    generateEventId,
    type ClipRangeParams,
    type Range,
    type VideoItem,
  } from "../interface";
  import { announceTask, openTaskCenter, settleTask, wasCancelled } from "../stores/tasks";

  // Progress is shown by <TaskCenter />, which the host window must mount.

  interface Props {
    archive?: RecordItem | null;
    ranges?: Range[];
    initialNote?: string;
    localOffset?: number | null;
    captureCover?: boolean;
    variant?: "primary" | "compact";
    disabled?: boolean;
    onGenerated?: (video: VideoItem) => void;
    onFailed?: (message: string) => void;
  }

  let {
    archive = null,
    ranges = [],
    initialNote = "",
    localOffset = null,
    captureCover = false,
    variant = "primary",
    disabled = false,
    onGenerated,
    onFailed
  }: Props = $props();

  const transitionOptions = [
    { value: "none", label: "无" },
    { value: "fade", label: "淡入淡出" },
    { value: "dissolve", label: "溶解" },
    { value: "wipeleft", label: "向左擦除" },
    { value: "wiperight", label: "向右擦除" },
    { value: "slideup", label: "向上滑动" },
    { value: "slidedown", label: "向下滑动" },
  ];

  let showConfirm = $state(false);
  let clipNote = $state("");
  let danmuEnabled = $state(false);
  let fixEncoding = $state(false);
  let transition = $state("none");
  /** Clip requests still awaiting a result; several may run at once. */
  let inFlight = 0;

  let activeRanges = $derived(ranges.filter((range) => range.activated !== false));

  function openConfirm() {
    if (!archive || disabled) return;
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

  async function generateClip() {
    if (!archive || activeRanges.length === 0) return;
    showConfirm = false;
    const eventId = generateEventId();
    const params: ClipRangeParams = {
      title: archive.title,
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
    };
    // The form now belongs to the next clip.
    transition = "none";
    announceTask({
      id: eventId,
      task_type: "clip_range",
      metadata: JSON.stringify({ params: { ...params, cover: "" } }),
    });
    openTaskCenter(eventId);

    inFlight += 1;
    try {
      const video = await clipRange(eventId, params);
      onGenerated?.(video as VideoItem);
    } catch (error) {
      const message = String(error);
      // Cancelled from the task center, or elsewhere such as the task page.
      // Desktop and web invoke wrappers prepend context to the backend error.
      if (!wasCancelled(eventId) && !message.endsWith("切片任务已取消")) {
        alert("切片生成失败：" + message);
        onFailed?.(message);
      }
    } finally {
      inFlight -= 1;
      settleTask(eventId);
    }
  }

  function handleBeforeUnload(event: BeforeUnloadEvent) {
    if (inFlight === 0) return;
    const message = "切片任务将在后台继续运行，可前往任务页面管理后台任务。";
    event.preventDefault();
    event.returnValue = message;
    return message;
  }

  onMount(() => window.addEventListener("beforeunload", handleBeforeUnload));
  onDestroy(() => window.removeEventListener("beforeunload", handleBeforeUnload));
</script>

<button
  type="button"
  onclick={openConfirm}
  disabled={disabled || !archive}
  class={variant === "compact"
    ? "ui-btn ui-btn-sm bg-surface text-accent-fg"
    : "ui-btn ui-btn-primary"}
>
  {#if variant === "compact"}
    <Scissors class="h-3.5 w-3.5" />
  {/if}
  <span>生成切片</span>
</button>

{#if showConfirm}
  <div class="fixed inset-0 z-[100] flex items-center justify-center">
    <div
      class="absolute inset-0 bg-black/50 backdrop-blur-sm"
      role="button"
      tabindex="0"
      aria-label="关闭对话框"
      onclick={closeConfirm}
      onkeydown={(event) => {
        if (event.key === "Escape" || event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          closeConfirm();
        }
      }}
></div>

    <div
      role="dialog"
      aria-modal="true"
      class="ui-modal relative mx-4 w-full max-w-md text-left"
    >
      <div class="ui-modal-body custom-scrollbar-light">
        <h3 class="text-[15px] font-semibold text-fg">确认生成切片</h3>
        <p class="mt-1 text-[12px] text-fg-muted">请确认以下设置后继续</p>

        <div class="mt-4 space-y-2">
          <div class="ui-label">待合并选区列表</div>
          <div class="max-h-48 space-y-1.5 overflow-y-auto custom-scrollbar-light">
            {#each activeRanges as range, index}
              <div class="flex items-center justify-between rounded-[8px] border border-line bg-control px-3 py-2 transition-colors hover:border-line-strong">
                <div class="flex items-center space-x-3">
                  <div class="flex h-6 w-6 items-center justify-center rounded-full bg-accent-soft text-[11px] font-semibold tabular-nums text-accent-fg">
                    {index + 1}
                  </div>
                  <div class="flex flex-col space-y-0.5">
                    <div class="text-[12px] tabular-nums text-fg">
                      {formatTime(range.start)} → {formatTime(range.end)}
                    </div>
                    <div class="text-[11px] text-fg-muted">
                      时长: {formatDuration(range.end - range.start)}
                    </div>
                  </div>
                </div>
              </div>
            {:else}
              <div class="py-4 text-center text-[12px] text-fg-muted">
                没有已激活的选区，请选择或添加选区。
              </div>
            {/each}
          </div>
          <div class="border-t border-line pt-2.5 text-[14px] font-semibold text-fg">
            总时长: {formatDuration(
              activeRanges.reduce(
                (total, range) => total + range.end - range.start,
                0,
              ),
            )}
          </div>
        </div>

        <div class="mt-4 space-y-1.5">
          <label for="archive-clip-note" class="block text-[13px] text-fg-secondary">
            切片备注（可选）
          </label>
          <input
            id="archive-clip-note"
            type="text"
            bind:value={clipNote}
            class="ui-input w-full"
          />
        </div>

        <div class="mt-4 space-y-2.5">
          <label class="flex cursor-pointer items-center gap-2.5">
            <input
              type="checkbox"
              bind:checked={danmuEnabled}
              class="ui-checkbox"
            />
            <span class="text-[13px] text-fg-secondary">压制弹幕</span>
          </label>

          <label class="flex cursor-pointer items-center gap-2.5">
            <input
              type="checkbox"
              bind:checked={fixEncoding}
              class="ui-checkbox"
            />
            <span class="text-[13px] text-fg-secondary">修复编码（切片异常时使用）</span>
          </label>
        </div>

        {#if activeRanges.length > 1}
          <div class="mt-4 space-y-2">
            <div class="ui-label">转场效果</div>
            <div class="grid grid-cols-4 gap-1.5">
              {#each transitionOptions as option}
                <button
                  type="button"
                  class="ui-btn ui-btn-sm {transition === option.value
                    ? 'ui-btn-active'
                    : ''}"
                  onclick={() => (transition = option.value)}
                >
                  {option.label}
                </button>
              {/each}
            </div>
          </div>
        {/if}
      </div>

      <div class="ui-modal-footer">
        <button
          type="button"
          onclick={closeConfirm}
          class="ui-btn"
        >
          取消
        </button>
        <button
          type="button"
          onclick={generateClip}
          disabled={activeRanges.length === 0}
          class="ui-btn ui-btn-primary"
        >
          确认生成
        </button>
      </div>
    </div>
  </div>
{/if}
