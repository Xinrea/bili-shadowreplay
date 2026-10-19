<script lang="ts" module>
  export interface BinMaterial {
    id: number;
    name: string;
    note: string;
    roomId: string;
    coverUrl: string;
    duration: number;
    createdAt: string;
  }
</script>

<script lang="ts">
  import { Film, Plus, Search } from "lucide-svelte";
  import { MATERIAL_DRAG_TYPE } from "./ProjectTimeline.svelte";

  interface Props {
    materials: BinMaterial[];
    roomId: string;
    onAdd: (material: BinMaterial) => void;
    onPreview: (material: BinMaterial) => void;
  }

  let { materials, roomId, onAdd, onPreview }: Props = $props();

  // Without a room (a new empty project) "room" shows every material.
  let scope: "room" | "all" = $state("room");
  let search = $state("");

  let shown = $derived.by(() => {
    const keyword = search.trim().toLowerCase();
    return materials
      .filter((material) => scope === "all" || !roomId || material.roomId === roomId)
      .filter(
        (material) =>
          !keyword ||
          material.name.toLowerCase().includes(keyword) ||
          material.note.toLowerCase().includes(keyword),
      )
      .sort((a, b) => new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime());
  });

  function formatDuration(seconds: number) {
    const total = Math.round(seconds);
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const secs = total % 60;
    return hours > 0
      ? `${hours}:${String(minutes).padStart(2, "0")}:${String(secs).padStart(2, "0")}`
      : `${minutes}:${String(secs).padStart(2, "0")}`;
  }

  function dragStart(event: DragEvent, material: BinMaterial) {
    event.dataTransfer?.setData(MATERIAL_DRAG_TYPE, String(material.id));
    if (event.dataTransfer) event.dataTransfer.effectAllowed = "copy";
  }
</script>

<aside class="bin" aria-label="素材库">
  <div class="bin-head">
    <h2>素材库</h2>
    <div class="scope" role="tablist">
      <button type="button" class:active={scope === "room"} disabled={!roomId} onclick={() => (scope = "room")}>
        本直播间
      </button>
      <button type="button" class:active={scope === "all"} onclick={() => (scope = "all")}>全部</button>
    </div>
    <label class="search">
      <Search class="h-3.5 w-3.5 shrink-0 text-[#6f7b8d]" />
      <input bind:value={search} placeholder="搜索素材" data-hotkey-block />
    </label>
  </div>

  <div class="bin-list sidebar-scrollbar">
    {#each shown as material (material.id)}
      <div
        class="item"
        role="listitem"
        draggable="true"
        ondragstart={(event) => dragStart(event, material)}
        title="拖到时间轴上插入，或点击 ＋ 添加到末尾"
      >
        <button type="button" class="thumb" onclick={() => onPreview(material)} title="预览素材">
          {#if material.coverUrl}
            <img src={material.coverUrl} alt="" draggable="false" />
          {:else}
            <Film class="h-4 w-4 text-[#6f7b8d]" />
          {/if}
          <span class="length">{formatDuration(material.duration)}</span>
        </button>
        <div class="meta">
          <span class="name" title={material.name}>{material.name}</span>
          <span class="note" title={material.note}>{material.note || material.roomId}</span>
        </div>
        <button type="button" class="add" title="添加到时间轴末尾" onclick={() => onAdd(material)}>
          <Plus class="h-3.5 w-3.5" />
        </button>
      </div>
    {:else}
      <div class="empty">
        {materials.length === 0 ? "还没有素材，请先从录播生成或导入视频" : "没有匹配的素材"}
      </div>
    {/each}
  </div>
</aside>

<style>
  .bin {
    display: flex;
    width: 248px;
    flex-shrink: 0;
    flex-direction: column;
    border-right: 1px solid #222a35;
    background: #171c25;
    color: #dce5f3;
  }

  .bin-head {
    display: flex;
    flex-direction: column;
    gap: 8px;
    border-bottom: 1px solid #222a35;
    padding: 12px;
  }

  h2 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
  }

  .scope {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
    border-radius: 8px;
    background: #0f131a;
    padding: 3px;
  }

  .scope button {
    border-radius: 6px;
    padding: 3px 0;
    color: #8c98aa;
    font-size: 11px;
  }

  .scope button.active {
    background: #283244;
    color: #edf4ff;
  }

  .scope button:disabled {
    opacity: 0.4;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    border: 1px solid #293442;
    border-radius: 8px;
    background: #0f131a;
    padding: 5px 8px;
  }

  .search input {
    min-width: 0;
    flex: 1;
    border: 0;
    background: transparent;
    padding: 0;
    color: #e9eef8;
    font-size: 12px;
    outline: none;
  }

  .search input:focus {
    box-shadow: none;
  }

  .bin-list {
    flex: 1;
    overflow-y: auto;
    padding: 6px;
  }

  .item {
    display: flex;
    align-items: center;
    gap: 8px;
    border-radius: 8px;
    padding: 6px;
    cursor: grab;
  }

  .item:hover {
    background: rgb(255 255 255 / 4%);
  }

  .thumb {
    position: relative;
    display: flex;
    width: 64px;
    height: 36px;
    flex-shrink: 0;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    border: 1px solid #293442;
    border-radius: 6px;
    background: #0c1016;
  }

  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .length {
    position: absolute;
    right: 2px;
    bottom: 2px;
    border-radius: 3px;
    background: rgb(0 0 0 / 70%);
    padding: 0 3px;
    color: #f3f4f6;
    font-family: ui-monospace, monospace;
    font-size: 9px;
  }

  .meta {
    display: flex;
    min-width: 0;
    flex: 1;
    flex-direction: column;
    gap: 2px;
  }

  .name,
  .note {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .name {
    font-size: 11px;
  }

  .note {
    color: #718095;
    font-size: 10px;
  }

  .add {
    display: inline-flex;
    flex-shrink: 0;
    border-radius: 6px;
    padding: 4px;
    color: #8c98aa;
  }

  .add:hover {
    background: rgb(10 132 255 / 20%);
    color: #55b9ff;
  }

  .empty {
    padding: 24px 12px;
    color: #718095;
    font-size: 11px;
    text-align: center;
  }
</style>
