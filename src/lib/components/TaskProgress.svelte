<script lang="ts">
  import type { ProgressUpdate } from "../interface";
  import {
    formatClock,
    formatRemaining,
    progressPercent,
    remainingSeconds,
  } from "../task-progress";

  interface Props {
    /** Latest update of the task, or null before the first one arrives. */
    update: ProgressUpdate | null;
    /** Shown until the task reports anything. */
    placeholder?: string;
    class?: string;
  }

  let { update, placeholder = "准备中", class: className = "" }: Props = $props();

  let progress = $derived(update?.progress);
  let stage = $derived(progress?.stage ?? update?.content ?? placeholder);
  let percent = $derived(progressPercent(progress));

  let position = $derived.by(() => {
    if (!progress) return "";
    const withHours = (progress.total ?? progress.processed) >= 3600;
    const processed = formatClock(progress.processed, withHours);
    if (progress.total === undefined) return `已处理 ${processed}`;
    return `${processed} / ${formatClock(progress.total, withHours)}`;
  });

  let remaining = $derived.by(() => {
    const seconds = remainingSeconds(progress);
    if (seconds === null) return "";
    return seconds < 1 ? "即将完成" : `剩余约 ${formatRemaining(seconds)}`;
  });

  // Several ffmpeg jobs can run under one stage name. Remount the bar when it
  // restarts so the width transition never animates backwards.
  let restarts = 0;
  let lastPercent = 0;
  let barKey = $derived.by(() => {
    if (percent !== null && percent < lastPercent) restarts += 1;
    lastPercent = percent ?? 0;
    return `${stage}:${restarts}`;
  });
</script>

<div
  class="flex min-w-0 items-center gap-2 text-[11px] {className}"
  role="progressbar"
  aria-label={stage}
  aria-valuemin={0}
  aria-valuemax={100}
  aria-valuenow={percent ?? undefined}
  aria-valuetext={update?.content ?? placeholder}
  title={[stage, position, remaining].filter(Boolean).join(" · ")}
>
  <!-- Fixed side columns keep bars aligned when several tasks are listed. -->
  <span class="w-[5.5rem] shrink-0 truncate text-white/75">{stage}</span>
  <div class="h-1 min-w-[3rem] flex-1 overflow-hidden rounded-full bg-white/10">
    {#key barKey}
      {#if percent !== null}
        <div
          class="h-full rounded-full bg-[#0A84FF] transition-[width] duration-500 ease-linear"
          style:width="{percent}%"
        ></div>
      {:else}
        <div class="indeterminate h-full w-1/3 rounded-full bg-[#0A84FF]/80"></div>
      {/if}
    {/key}
  </div>
  <span class="w-[8.5rem] shrink-0 truncate text-right tabular-nums text-white/45">
    {#if percent !== null}
      <span class="font-medium text-[#4da3ff]">{percent}%</span>{remaining ? ` · ${remaining}` : ""}
    {:else}
      {position}
    {/if}
  </span>
</div>

<style>
  .indeterminate {
    animation: slide 1.2s ease-in-out infinite;
  }

  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(300%);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .indeterminate {
      width: 100%;
      animation: none;
      opacity: 0.4;
    }
  }
</style>
