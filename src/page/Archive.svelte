<script lang="ts">
  import { onMount } from "svelte";
  import { RefreshCw } from "lucide-svelte";
  import { loadRoomArchives } from "../lib/archives";
  import type { RecordItem } from "../lib/db";
  import type { RecorderInfo, RecorderList } from "../lib/interface";
  import { invoke } from "../lib/invoker";
  import { createStateRefresh, watchStateChanges } from "../lib/state-refresh";
  import { registerCommands } from "../lib/stores/commands";
  import ArchiveList from "../lib/components/ArchiveList.svelte";
  import PageTitlebar from "../lib/components/PageTitlebar.svelte";

  let archives: RecordItem[] = $state([]);
  let rooms: RecorderInfo[] = $state([]);
  let loading = $state(true);
  let loadError = $state("");
  let disposed = false;

  async function fetchArchives() {
    loading = true;
    try {
      const recorders = await invoke<RecorderList>("get_recorder_list");
      const snapshots = await Promise.all(recorders.recorders.map((room) =>
        loadRoomArchives(room.room_info, () => !disposed)));
      if (disposed) return;
      archives = snapshots.flat();
      rooms = recorders.recorders;
      loadError = "";
    } catch (error) {
      if (!disposed) loadError = `录播记录加载失败：${String(error)}`;
    } finally {
      if (!disposed) loading = false;
    }
  }

  const stateRefresh = createStateRefresh(fetchArchives);
  const loadArchives = () => stateRefresh.run();

  onMount(() => {
    const stop = watchStateChanges(stateRefresh, ["records-changed", "recorders-changed"]);
    return () => { disposed = true; stop(); };
  });
  onMount(() => registerCommands([{
    id: "archive:refresh", group: "action", title: "刷新录播列表", detail: "录播",
    icon: RefreshCw, keywords: ["refresh archives", "reload"], page: "录播", run: loadArchives,
  }]));
</script>

<PageTitlebar title="录播档案">
  {#snippet subtitle()}
    <span class="truncate text-[12px] text-fg-muted">管理所有直播间的录播记录，可以查看、播放和管理历史直播内容。</span>
  {/snippet}
  <button class="titlebar-icon-button" title="刷新" aria-label="刷新" onclick={loadArchives} disabled={loading}>
    <RefreshCw class={`h-4 w-4 ${loading ? "animate-spin motion-reduce:animate-none" : ""}`} />
  </button>
</PageTitlebar>

<ArchiveList {archives} {rooms} {loading} error={loadError} filterRooms onrefresh={loadArchives} />
