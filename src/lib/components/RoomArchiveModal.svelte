<script lang="ts">
  import { onMount } from "svelte";
  import { History, RefreshCw, X } from "lucide-svelte";
  import { loadRoomArchives } from "../archives";
  import type { RecordItem } from "../db";
  import type { RecorderInfo } from "../interface";
  import { trapFocus } from "../focus-trap";
  import { pageVisibility } from "../page-visibility";
  import { platformLabel } from "../platform";
  import { createStateRefresh, watchStateChanges } from "../state-refresh";
  import ArchiveList from "./ArchiveList.svelte";

  interface Props { room: RecorderInfo; onclose: () => void; }
  let { room, onclose }: Props = $props();
  let archives: RecordItem[] = $state([]);
  let loading = $state(true);
  let loadError = $state("");
  let dialogOpen = $state(false);
  let disposed = false;
  const pageVisible = pageVisibility();

  async function refreshArchives() {
    loading = true;
    try {
      const result = await loadRoomArchives(room.room_info, () => !disposed);
      if (disposed) return;
      archives = result;
      loadError = "";
    } catch (error) {
      if (!disposed) loadError = `录播记录加载失败：${String(error)}`;
    } finally {
      if (!disposed) loading = false;
    }
  }

  const stateRefresh = createStateRefresh(refreshArchives, 1000);
  const refresh = () => stateRefresh.run();
  onMount(() => {
    const stop = watchStateChanges(stateRefresh, ["records-changed"]);
    return () => { disposed = true; stop(); };
  });
</script>

<svelte:window onkeydown={(event) => {
  if (event.key === "Escape" && pageVisible() && !dialogOpen && !event.defaultPrevented && !event.isComposing) {
    event.preventDefault();
    event.stopImmediatePropagation();
    onclose();
  }
}} />

<div class="ui-modal-backdrop">
  <button class="absolute inset-0" type="button" aria-label="关闭直播间录播历史" onclick={() => { if (!dialogOpen) onclose(); }}></button>
  <div use:trapFocus={{ active: pageVisible() }} role="dialog" aria-modal="true" aria-labelledby="room-archive-title" tabindex="-1" class="ui-modal relative h-[80vh] w-[1040px] max-w-full">
    <div class="ui-modal-header">
      <div class="flex min-w-0 items-center gap-2">
        <History class="h-4 w-4 shrink-0 text-fg-muted" />
        <h2 id="room-archive-title" class="shrink-0">录播历史</h2>
        <span class="truncate text-[12px] font-normal text-fg-muted" title={`${room.user_info.user_name} · ${platformLabel(room.room_info.platform)} #${room.room_info.room_id}`}>
          {room.user_info.user_name || platformLabel(room.room_info.platform)} · {room.room_info.room_id}
        </span>
      </div>
      <div class="flex shrink-0 items-center gap-1.5">
        <button class="ui-btn ui-btn-ghost ui-btn-sm" type="button" onclick={refresh} disabled={loading}>
          <RefreshCw class={`h-3.5 w-3.5 ${loading ? "animate-spin motion-reduce:animate-none" : ""}`} />刷新
        </button>
        <button class="ui-btn ui-btn-ghost ui-btn-sm ui-btn-icon -mr-1.5" type="button" aria-label="关闭直播间录播历史窗口" onclick={onclose}><X class="h-4 w-4" /></button>
      </div>
    </div>

    <ArchiveList {archives} rooms={[room]} {loading} error={loadError} compact bind:dialogOpen onrefresh={refresh} />
  </div>
</div>
