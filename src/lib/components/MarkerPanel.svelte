<script lang="ts">
  import {
    BanOutline,
    CloseOutline,
    ForwardOutline,
    ClockOutline,
  } from "flowbite-svelte-icons";
  import type { Marker } from "../interface";
  import { Tooltip } from "flowbite-svelte";
  import { invoke, TAURI_ENV } from "../invoker";
  import { save } from "@tauri-apps/plugin-dialog";
  import type { RecordItem } from "../db";
  interface Props {
    archive: RecordItem;
    markers?: Marker[];
    onMarkerClick?: (marker: Marker) => void;
  }

  let { archive, markers = $bindable([]), onMarkerClick }: Props = $props();

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

  async function export_to_file() {
    let r = "# 由 BiliShadowReplay 自动生成\n";
    r += `# ${archive.title} - 直播开始时间：${format_realtime(parseInt(archive.live_id) * 1000)}\n\n`;
    for (let i in markers) {
      r += `[${format_realtime(markers[i].realtime)}][${format_duration(markers[i].offset)}] ${
        markers[i].content
      }\n`;
    }
    let file_name = `[${archive.room_id}][${format_realtime(
      parseInt(archive.live_id)
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

<div class="flex flex-col w-full h-screen text-white p-4 pr-0">
  <div class="mb-4 flex flex-row justify-between">
    <div class="flex">
      <span class="mr-1">标记列表</span>
      <button
        class="mr-1"
        onclick={() => {
          realtime = !realtime;
        }}><ClockOutline /></button
      >
      <Tooltip>切换时间形式</Tooltip>
      <button onclick={export_to_file}><ForwardOutline /></button>
      <Tooltip>导出为文件</Tooltip>
    </div>
    <button
      class="mr-2"
      onclick={() => {
        markers = [];
      }}><BanOutline /></button
    >
    <Tooltip>清空</Tooltip>
  </div>

  <div class="overflow-y-auto sidebar-scrollbar">
    {#each markers as marker, i}
      <div class="marker-entry">
        <div class="marker-control">
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <span
            class="offset"
            role="button"
            tabindex="0"
            onclick={() => {
              dispatch_markerclick(marker);
            }}
            onkeydown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                dispatch_markerclick(marker);
              }
            }}
            >{realtime
              ? format_realtime(marker.realtime)
              : format_duration(marker.offset)}</span
          >
          <button
            class="hover:bg-red-900"
            onclick={() => {
              // remove this entry
              markers = markers.filter((_, idx) => idx !== i);
            }}><CloseOutline /></button
          >
        </div>
        <input
          class="content w-full"
          bind:value={marker.content}
          onchange={(v) => {
            if (marker.content == "") {
              marker.content = "[空标记点]";
            }
          }}
        />
      </div>
    {/each}
  </div>
</div>

<style>
  .marker-entry {
    display: flex;
    flex-direction: column;
    padding: 4px;
    border-top: 1px solid rgba(255, 255, 255, 0.1);
  }
  .marker-entry:first-child {
    border-top: none;
  }
  .marker-entry:hover {
    background-color: rgba(255, 255, 255, 0.1);
  }
  .marker-entry .offset {
    font-style: italic;
    cursor: pointer;
    margin-right: 6px;
    color: rgba(255, 255, 255, 0.5);
  }
  .marker-entry .content {
    background: transparent;
  }
  .marker-control {
    display: flex;
    padding-right: 4px;
    flex-direction: row;
    justify-content: space-between;
  }
</style>
