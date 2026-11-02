<script lang="ts">
  import { onMount } from "svelte";
  import type { Snippet } from "svelte";
  import { buildRoomCheckTimeline, roomCheckBucketLabel } from "../room-check-timeline";
  import type { RoomCheckSample } from "../interface";

  let { history, loading = false, statusBadge, metadata }: {
    history: RoomCheckSample[]; loading?: boolean; statusBadge?: Snippet; metadata?: Snippet;
  } = $props();
  let width = $state(0);
  let now = $state(Date.now());
  let coarse = $state(false);
  let selectedKey = $state<number | null>(null);
  const chartWidth = $derived(width || 700);
  const chart = $derived(buildRoomCheckTimeline(history, now, chartWidth, coarse));
  const selected = $derived(chart.bars.find((bar) => bar.key === selectedKey));

  onMount(() => {
    coarse = window.matchMedia("(pointer: coarse)").matches;
    const updateClock = () => { if (document.visibilityState !== "hidden") now = Date.now(); };
    const timer = setInterval(updateClock, 1000);
    document.addEventListener("visibilitychange", updateClock);
    return () => { clearInterval(timer); document.removeEventListener("visibilitychange", updateClock); };
  });
</script>

<figure class="check-timeline mt-4 min-w-0 border-t border-line pt-3" aria-label="房间检查时间线" aria-busy={loading}>
  <figcaption class="mb-2 flex flex-wrap items-center justify-between gap-x-4 gap-y-1 text-[11px]">
    <div class="flex items-center gap-2"><h3 class="text-[12px] font-medium text-fg-secondary">房间检查</h3>{#if statusBadge}{@render statusBadge()}{/if}</div>
    <div class="flex flex-wrap items-center gap-3 text-fg-muted">
      <span>最近 5 分钟{chart.count ? ` · ${chart.count} 次检查` : ""}</span>
      {#if chart.count}
        <span class="inline-flex items-center gap-1.5"><span class="h-1.5 w-1.5 rounded-full bg-success" aria-hidden="true"></span>成功 {chart.count - chart.failures}</span>
        {#if chart.failures}<span class="inline-flex items-center gap-1.5"><span class="text-danger-fg" aria-hidden="true">×</span>失败 {chart.failures}</span>{/if}
      {/if}
    </div>
  </figcaption>
  {#if metadata}<div class="mb-1 flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-fg-muted">{@render metadata()}</div>{/if}
  <div class="relative min-w-0" bind:clientWidth={width}>
    {#if chart.count}
      <svg class="timeline-svg" viewBox={`0 0 ${chartWidth} ${chart.height}`} role="img" aria-label={`最近五分钟共 ${chart.count} 次房间检查，${chart.failures} 次失败。柱高表示耗时，下方圆点表示结果。`}>
        <text x="0" y="11" class="axis-label">{chart.unit}</text>
        <rect x={chart.left} y={chart.top} width={chart.plotWidth} height={chart.bottom - chart.top} class="plot-frame" />
        {#each chart.ticks as tick (tick.label)}
          <line x1={chart.left} x2={chart.right} y1={tick.y} y2={tick.y} class="grid-line" />
          <text x={chart.left - 9} y={tick.y + 3.5} text-anchor="end">{tick.label}</text>
        {/each}
        <line x1={chart.left} x2={chart.right} y1={chart.resultY} y2={chart.resultY} class="event-line" />
        {#each chart.bars as bar (bar.key)}
          <g class="check-mark" style={`transform:translateX(${bar.x}px)`} data-check-at={bar.checked_at} data-check-failed={bar.failures > 0}>
            <rect x={-bar.width / 2} y={bar.y} width={bar.width} height={bar.height} rx="2" class="check-bar" class:failed={bar.failures > 0} class:selected={selectedKey === bar.key} />
            {#if bar.failures}<text x="0" y={bar.y + Math.min(12, bar.height - 1)} text-anchor="middle" class="failure-cross">×</text>{/if}
            <circle cx="0" cy={chart.resultY} r={selectedKey === bar.key ? 3.5 : 2.75} class="result-dot" class:failed={bar.failures > 0} />
          </g>
        {/each}
        <text x={chart.left} y={chart.labelY}>−5 分钟</text>
        <text x={(chart.left + chart.right) / 2} y={chart.labelY} text-anchor="middle">−2.5 分钟</text>
        <text x={chart.right} y={chart.labelY} text-anchor="end">现在</text>
      </svg>
      {#each chart.bars as bar (bar.key)}
        <button
          type="button"
          class="timeline-target absolute top-0 rounded-sm focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent-fg"
          style={`left:${bar.hitLeft}px;width:${bar.hitWidth}px;height:${chart.resultY + 9}px`}
          aria-label={roomCheckBucketLabel(bar)}
          onpointerenter={() => selectedKey = bar.key}
          onpointerleave={(event) => { if (document.activeElement !== event.currentTarget && selectedKey === bar.key) selectedKey = null; }}
          onfocus={() => selectedKey = bar.key}
          onblur={() => { if (selectedKey === bar.key) selectedKey = null; }}
          onclick={() => selectedKey = bar.key}
        ></button>
      {/each}
      {#if selected}
        <div role="tooltip" class="pointer-events-none absolute right-1 top-0 z-10 max-w-[min(360px,calc(100%_-_8px))] rounded-[7px] border border-line bg-surface px-2.5 py-2 text-[11px] leading-5 text-fg shadow-lg">
          <div class="flex flex-wrap items-center gap-x-3 gap-y-0.5 tabular-nums">
            <span class="text-fg-muted">{new Date(selected.checked_at).toLocaleTimeString()}</span>
            <span class:text-danger-fg={selected.failures > 0} class:text-success-fg={!selected.failures}>{selected.count > 1 ? `${selected.count} 次检查，${selected.failures} 次失败` : selected.failures ? "检查失败" : "检查成功"}</span>
            <span>{selected.count > 1 ? "最高耗时 " : ""}{selected.duration_ms} ms</span>
          </div>
          {#if selected.error !== null}<p class="mt-0.5 line-clamp-3 select-text break-words text-danger-fg">{selected.error}</p>{/if}
        </div>
      {/if}
    {:else}
      <div class="flex h-[124px] items-center justify-center text-center text-[12px] text-fg-muted">{loading ? "正在加载检查记录…" : "最近 5 分钟暂无检查记录，下一次检查完成后会自动显示。"}</div>
    {/if}
  </div>
</figure>

<style>
  .timeline-svg { display: block; width: 100%; overflow: visible; }
  .timeline-svg text { fill: rgb(var(--c-fg-muted)); font-size: 11px; font-variant-numeric: tabular-nums; }
  .timeline-svg .axis-label { fill: rgb(var(--c-fg-secondary)); }
  .plot-frame, .grid-line, .event-line { fill: none; stroke: rgb(var(--c-line)); stroke-width: 0.7; }
  .check-mark { transition: transform 500ms linear; }
  .check-bar { fill: rgb(var(--c-accent)); opacity: 0.72; }
  .check-bar.selected { opacity: 1; }
  .result-dot { fill: rgb(var(--c-success)); }
  .check-bar.failed, .result-dot.failed { fill: rgb(var(--c-danger)); opacity: 1; }
  .timeline-svg .failure-cross { fill: rgb(var(--c-surface)); font-weight: 500; }
  .timeline-target { transition: left 500ms linear, width 500ms linear; }
  @media (prefers-reduced-motion: reduce) {
    .check-mark, .timeline-target { transition: none; }
  }
</style>
