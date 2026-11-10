<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "../invoker";
  import type { DiskInfo } from "../interface";
  import { formatSize } from "../size-format";
  import { createStateRefresh, watchStateChanges } from "../state-refresh";

  let disks: DiskInfo[] = $state([]);
  let loading = $state(true);
  let error = $state("");
  let disposed = false;
  const refresh = createStateRefresh(loadDisks);

  async function loadDisks() {
    loading = true;
    error = "";
    try {
      const snapshot = await invoke<DiskInfo[]>("get_disk_info");
      if (!disposed) disks = snapshot;
    } catch {
      if (!disposed) {
        disks = [];
        error = "磁盘信息暂不可用";
      }
    } finally {
      if (!disposed) loading = false;
    }
  }

  // Independent of the summary snapshot: slow or failed disk probes must not
  // block recording counts, accounts or the archive list.
  onMount(() => {
    const stop = watchStateChanges(refresh, ["records-changed"]);
    return () => { disposed = true; stop(); };
  });
</script>

<section class="disk-section space-y-3" aria-labelledby="local-disks-title" aria-busy={loading}>
  <h2 id="local-disks-title" class="ui-section-title">本机磁盘</h2>
  {#if error}
    <p class="ui-card px-4 py-4 text-[12px] text-fg-muted" role="status">{error}</p>
  {:else if loading && disks.length === 0}
    <p class="ui-card px-4 py-4 text-[12px] text-fg-muted" role="status">读取中…</p>
  {:else if disks.length === 0}
    <p class="ui-card px-4 py-4 text-[12px] text-fg-muted" role="status">暂无磁盘信息</p>
  {:else}
    <ul class="disk-list ui-card">
      {#each disks as disk (disk.id)}
        {@const used = Math.max(0, disk.total - disk.free)}
        {@const percent = disk.total > 0 ? Math.min(100, Math.round(used / disk.total * 100)) : 0}
        <li class="disk-item">
          <div class="flex min-w-0 items-baseline justify-between gap-3">
            <p class="disk-path select-text text-[13px] font-medium text-fg"
              title={disk.mount_points.join("\n")}
              aria-label={`${disk.name}：${disk.mount_points.join("，")}`}>{disk.name}</p>
            <span class="shrink-0 text-[12px] tabular-nums text-fg-muted" aria-hidden="true">{percent}%</span>
          </div>
          <progress class="disk-progress" class:near-full={percent >= 90}
            max="100" value={percent} aria-label={`${disk.name} 已用空间`}
            aria-valuetext={`已用 ${formatSize(used)}，总量 ${formatSize(disk.total)}，可用 ${formatSize(disk.free)}`}>
            {percent}%
          </progress>
          <div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1 text-[11px] text-fg-muted tabular-nums">
            <p><span class="text-[14px] font-semibold text-fg-secondary">{formatSize(disk.free)}</span> 可用</p>
            <p>共 {formatSize(disk.total)}</p>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .disk-section {
    container-type: inline-size;
  }
  .disk-list {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    overflow: hidden;
  }
  .disk-item {
    min-width: 0;
    padding: 14px 16px;
  }
  .disk-item + .disk-item {
    border-top: 1px solid rgb(var(--c-line));
  }
  .disk-path {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .disk-progress {
    display: block;
    width: 100%;
    height: 4px;
    margin: 10px 0;
    overflow: hidden;
    border: 0;
    border-radius: 999px;
    appearance: none;
    background: rgb(var(--c-sunken));
    color: rgb(var(--c-accent));
  }
  .disk-progress.near-full { color: rgb(var(--c-warning)); }
  .disk-progress::-webkit-progress-bar { background: rgb(var(--c-sunken)); }
  .disk-progress::-webkit-progress-value { background: currentColor; border-radius: 999px; }
  .disk-progress::-moz-progress-bar { background: currentColor; border-radius: 999px; }

  @container (min-width: 600px) {
    .disk-list {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .disk-item + .disk-item {
      border-top: 0;
    }
    .disk-item:nth-child(even) {
      border-left: 1px solid rgb(var(--c-line));
    }
    .disk-item:nth-child(n + 3) {
      border-top: 1px solid rgb(var(--c-line));
    }
    .disk-item:last-child:nth-child(odd) {
      grid-column: 1 / -1;
    }
  }
</style>
