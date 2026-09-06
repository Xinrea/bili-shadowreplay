<script lang="ts">
  import { Clock, Download, Eraser, X } from "lucide-svelte";
  import type { Marker } from "../interface";
  import { invoke, TAURI_ENV } from "../invoker";
  import { save } from "@tauri-apps/plugin-dialog";
  import type { RecordItem } from "../db";

  interface Props {
    archive?: RecordItem | null;
    markers?: Marker[];
    embedded?: boolean;
    onMarkerClick?: (marker: Marker) => void;
  }

  let {
    archive,
    markers = $bindable([]),
    embedded = false,
    onMarkerClick,
  }: Props = $props();

  let realtime = $state(false);

  function format_duration(duration: number) {
    const hours = Math.floor(duration / 3600);
    const minutes = Math.floor((duration % 3600) / 60);
    const seconds = Math.floor(duration % 60);
    return `${hours.toString().padStart(2, "0")}:${minutes.toString().padStart(2, "0")}:${seconds.toString().padStart(2, "0")}`;
  }

  function format_realtime(ts: number) {
    const d = new Date(ts * 1000);
    return d.toLocaleString();
  }

  function dispatch_markerclick(marker: Marker) {
    onMarkerClick?.(marker);
  }

  function remove_marker(index: number) {
    markers = markers.filter((_, markerIndex) => markerIndex !== index);
  }

  function clear_markers() {
    markers = [];
  }

  function commit_marker_content(marker: Marker) {
    if (marker.content.trim() === "") {
      marker.content = "[空标记点]";
      markers = [...markers];
    }
  }

  async function export_to_file() {
    if (!archive) return;
    let r = "# 由 BiliShadowReplay 自动生成\n";
    r += `# ${archive.title} - 直播开始时间：${format_realtime(parseInt(archive.live_id) * 1000)}\n\n`;
    for (let i in markers) {
      r += `[${format_realtime(markers[i].realtime)}][${format_duration(markers[i].offset)}] ${
        markers[i].content
      }\n`;
    }
    let file_name = `[${archive.room_id}][${format_realtime(
      parseInt(archive.live_id),
    )
      .split(" ")[0]
      .replaceAll("/", "-")}]${archive.title}.txt`;
    console.log("export to file", file_name);
    if (TAURI_ENV) {
      const path = await save({
        title: "导出标记列表",
        defaultPath: file_name,
      });
      if (!path) return;
      await invoke("export_to_file", { fileName: path, content: r });
    } else {
      const a = document.createElement("a");
      a.href = "data:text/plain;charset=utf-8," + encodeURIComponent(r);
      a.download = file_name;
      a.click();
    }
  }
</script>

<section class="marker-panel" class:embedded class:standalone={!embedded}>
  <div class="marker-heading">
    <div>
      <h3>标记列表</h3>
      <span>{markers.length} 个标记点</span>
    </div>
    <div class="marker-tools">
      <button
        type="button"
        class:active={realtime}
        title={realtime ? "切换为相对时间" : "切换为真实时间"}
        onclick={() => {
          realtime = !realtime;
        }}
      >
        <Clock size={13} />
      </button>
      <button
        type="button"
        title="导出为文件"
        disabled={!archive || markers.length === 0}
        onclick={export_to_file}
      >
        <Download size={13} />
      </button>
      <button
        type="button"
        class="danger"
        title="清空全部标记"
        disabled={markers.length === 0}
        onclick={clear_markers}
      >
        <Eraser size={13} />
      </button>
    </div>
  </div>

  <div class="marker-list">
    {#each markers as marker, i}
      <article class="marker-entry">
        <button
          type="button"
          class="marker-time"
          title="跳转到标记位置"
          onclick={() => dispatch_markerclick(marker)}
        >
          {realtime
            ? format_realtime(marker.realtime)
            : format_duration(marker.offset)}
        </button>
        <input
          class="marker-content"
          bind:value={marker.content}
          onchange={() => commit_marker_content(marker)}
          onblur={() => commit_marker_content(marker)}
        />
        <button
          type="button"
          class="marker-remove"
          title="删除标记"
          onclick={() => remove_marker(i)}
        >
          <X size={13} />
        </button>
      </article>
    {:else}
      <div class="empty-state">
        <strong>还没有标记</strong>
        <span>在下方时间线点击“添加标记”，或使用快捷键 P</span>
      </div>
    {/each}
  </div>
</section>

<style>
  .marker-panel {
    display: flex;
    min-height: 0;
    flex: 1;
    flex-direction: column;
    color: #e9eef8;
  }

  .marker-panel.standalone {
    height: 100vh;
    padding: 16px;
  }

  .marker-panel.embedded {
    height: 100%;
  }

  .marker-heading {
    display: flex;
    flex: 0 0 auto;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
  }

  .marker-heading div:first-child {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .marker-heading h3 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
  }

  .marker-heading span {
    color: #8490a3;
    font-size: 10px;
  }

  .marker-tools {
    display: flex;
    gap: 4px;
  }

  .marker-tools button {
    display: inline-flex;
    width: 26px;
    height: 26px;
    align-items: center;
    justify-content: center;
    border: 1px solid #303947;
    border-radius: 7px;
    background: #202733;
    color: #8c98aa;
  }

  .marker-tools button:hover:not(:disabled) {
    border-color: #3d4a5c;
    color: #d5deea;
  }

  .marker-tools button.active {
    border-color: #3d8fd1;
    background: rgb(10 132 255 / 14%);
    color: #7ec8ff;
  }

  .marker-tools button.danger:hover:not(:disabled) {
    border-color: #7a3a45;
    background: rgb(239 86 104 / 12%);
    color: #ff8591;
  }

  .marker-tools button:disabled {
    cursor: not-allowed;
    opacity: 0.4;
  }

  .marker-list {
    display: flex;
    min-height: 0;
    flex: 1;
    flex-direction: column;
    margin-top: 12px;
    overflow-y: auto;
    border-top: 1px solid #242b36;
  }

  .marker-entry {
    display: flex;
    min-height: 34px;
    align-items: center;
    gap: 8px;
    border-bottom: 1px solid #1c222c;
    padding: 4px 2px;
  }

  .marker-entry:hover {
    background: rgb(255 255 255 / 3.5%);
  }

  .marker-time {
    flex: 0 0 auto;
    max-width: 118px;
    overflow: hidden;
    border: 0;
    background: transparent;
    padding: 0;
    color: #5f6b7d;
    font-size: 9px;
    font-variant-numeric: tabular-nums;
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .marker-time:hover {
    color: #7ec8ff;
  }

  .marker-content {
    min-width: 0;
    flex: 1;
    height: 26px;
    border: 1px solid transparent;
    border-radius: 6px;
    outline: none;
    background: transparent;
    padding: 0 6px;
    color: #c5cedb;
    font-size: 11px;
  }

  .marker-content:hover {
    border-color: #2d3745;
    background: #171c25;
  }

  .marker-content:focus {
    border-color: #0a84ff;
    background: #171c25;
    color: #eef4fd;
  }

  .marker-remove {
    display: inline-flex;
    width: 22px;
    height: 22px;
    flex: 0 0 22px;
    align-items: center;
    justify-content: center;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: #687589;
  }

  .marker-remove:hover {
    background: rgb(239 86 104 / 12%);
    color: #ff8591;
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    gap: 4px;
    border-radius: 10px;
    background: #202733;
    margin-top: 0;
    padding: 18px 14px;
    color: #8490a3;
  }

  .empty-state strong {
    color: #d5deea;
    font-size: 12px;
    font-weight: 600;
  }

  .empty-state span {
    font-size: 10px;
  }
</style>
