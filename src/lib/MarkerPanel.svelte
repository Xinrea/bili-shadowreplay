<script lang="ts">
  import {
    BanOutline,
    CloseOutline,
    ForwardOutline,
    ClockOutline,
  } from "flowbite-svelte-icons";
  import type { Marker } from "./interface";
  import { createEventDispatcher } from "svelte";
  import { Tooltip } from "flowbite-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { save } from "@tauri-apps/plugin-dialog";
  import type { RecordItem } from "./db";
  const dispatch = createEventDispatcher();
  export let archive: RecordItem;
  export let markers: Marker[] = [];

  let realtime = false;

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
    dispatch("markerClick", marker);
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
    const path = await save({
      title: "导出标记列表",
      defaultPath: file_name,
    });
    if (!path) return;
    await invoke("export_to_file", { fileName: path, content: r });
  }
</script>

<div class="flex flex-col w-full h-screen text-white p-4 pr-0">
  <div class="mb-4 flex flex-row justify-between">
    <div class="flex">
      <span class="mr-1">标记列表</span>
      <button
        class="mr-1"
        on:click={() => {
          realtime = !realtime;
        }}><ClockOutline /></button
      >
      <Tooltip>切换时间形式</Tooltip>
      <button on:click={export_to_file}><ForwardOutline /></button>
      <Tooltip>导出为文件</Tooltip>
    </div>
    <button
      class="mr-2"
      on:click={() => {
        markers = [];
      }}><BanOutline /></button
    >
    <Tooltip>清空</Tooltip>
  </div>

  <div class="overflow-y-auto">
    {#each markers as marker, i}
      <div class="marker-entry">
        <div class="marker-control">
          <!-- svelte-ignore a11y-click-events-have-key-events -->
          <span
            class="offset"
            on:click={() => {
              dispatch_markerclick(marker);
            }}
            >{realtime
              ? format_realtime(marker.realtime)
              : format_duration(marker.offset)}</span
          >
          <button
            class="hover:bg-red-900"
            on:click={() => {
              // remove this entry
              markers = markers.filter((_, idx) => idx !== i);
            }}><CloseOutline /></button
          >
        </div>
        <input
          class="content w-full"
          bind:value={marker.content}
          on:change={(v) => {
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
