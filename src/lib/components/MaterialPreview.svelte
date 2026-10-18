<script lang="ts">
  import { Clapperboard, Download, Film } from "lucide-svelte";
  import { onMount } from "svelte";
  import { get_static_url, invoke, set_title, TAURI_ENV } from "../invoker";
  import { onWindowClose, releaseMedia } from "../window-chrome";
  import type { DanmuEntry, VideoItem } from "../interface";
  import TitleBar from "./TitleBar.svelte";

  interface Props {
    materialId: number;
  }

  let { materialId }: Props = $props();

  let material: VideoItem | null = $state(null);
  let url = $state("");
  let danmus: DanmuEntry[] = $state([]);
  let error = $state("");
  let creating = $state(false);
  let video: HTMLVideoElement = $state();
  let currentTime = $state(0);
  let list: HTMLElement = $state();

  let source = $derived.by(() => {
    try {
      return material?.source ? JSON.parse(material.source) : null;
    } catch {
      return null;
    }
  });
  let activeIndex = $derived.by(() => {
    let index = -1;
    while (index + 1 < danmus.length && danmus[index + 1].ts / 1000 <= currentTime) index++;
    return index;
  });

  onMount(() => onWindowClose(releaseMedia));

  onMount(async () => {
    try {
      material = await invoke<VideoItem>("get_video", { id: materialId });
      url = await get_static_url("output", material.file);
      await set_title(material.title || material.file);
      danmus = (await invoke<DanmuEntry[]>("get_material_danmu", { id: materialId })).filter(
        (entry) => entry.type === "danmu" || entry.type === "super_chat",
      );
    } catch (cause) {
      error = String(cause);
    }
  });

  // Follow playback in the danmaku list.
  $effect(() => {
    if (activeIndex < 0 || !list) return;
    list.querySelector(`[data-index="${activeIndex}"]`)?.scrollIntoView({ block: "nearest" });
  });

  async function createProject() {
    if (!material || creating) return;
    creating = true;
    try {
      const project = await invoke<{ id: number }>("create_project", {
        title: "",
        roomId: material.room_id,
        platform: material.platform ?? "",
        materialIds: [material.id],
      });
      await invoke("open_project", { projectId: project.id });
    } catch (cause) {
      alert("新建切片失败：" + cause);
    } finally {
      creating = false;
    }
  }

  function download() {
    if (!material) return;
    const a = document.createElement("a");
    a.href = url;
    a.download = material.file;
    a.click();
  }

  function formatTime(seconds: number) {
    const total = Math.max(0, Math.floor(seconds));
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const secs = total % 60;
    const tail = `${String(minutes).padStart(2, "0")}:${String(secs).padStart(2, "0")}`;
    return hours > 0 ? `${hours}:${tail}` : tail;
  }

  function formatSize(size: number) {
    if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KiB`;
    if (size < 1024 * 1024 * 1024) return `${(size / 1024 / 1024).toFixed(1)} MiB`;
    return `${(size / 1024 / 1024 / 1024).toFixed(2)} GiB`;
  }
</script>

<main class="flex h-screen flex-col bg-[#1c1c1e] text-white">
  <TitleBar tone="dark" title={material ? material.title || material.file : "素材预览"}>
    {#snippet actions()}
      {#if !TAURI_ENV && material}
        <button class="titlebar-icon-button" title="下载素材" aria-label="下载素材" onclick={download}>
          <Download class="h-4 w-4" />
        </button>
      {/if}
    {/snippet}
  </TitleBar>

  {#if error}
    <div class="flex flex-1 items-center justify-center text-sm text-red-300">加载素材失败：{error}</div>
  {:else}
    <div class="flex min-h-0 flex-1">
      <div class="flex min-w-0 flex-1 items-center justify-center bg-black">
        {#if url}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video
            bind:this={video}
            src={url}
            controls
            class="max-h-full max-w-full"
            ontimeupdate={() => (currentTime = video.currentTime)}
          ></video>
        {/if}
      </div>

      <aside class="flex w-72 shrink-0 flex-col border-l border-[#222a35] bg-[#171c25]">
        {#if material}
          <div class="space-y-2 border-b border-[#222a35] p-4 text-[12px]">
            <div class="flex items-center gap-2 text-[13px] font-medium">
              <Film class="h-4 w-4 text-violet-300" />
              <span class="truncate" title={material.file}>{material.title || material.file}</span>
            </div>
            <dl class="grid grid-cols-[64px_1fr] gap-x-2 gap-y-1 text-[11px]">
              <dt class="text-[#718095]">直播间</dt>
              <dd class="truncate">{material.room_id || "—"}</dd>
              {#if material.live_id}
                <dt class="text-[#718095]">来源录播</dt>
                <dd class="truncate" title={material.live_id}>{material.live_id}</dd>
              {/if}
              {#if source?.ranges?.length}
                <dt class="text-[#718095]">录播区间</dt>
                <dd>
                  {#each source.ranges as range}
                    <span class="block font-mono">{formatTime(range.start)} – {formatTime(range.end)}</span>
                  {/each}
                </dd>
              {/if}
              <dt class="text-[#718095]">时长</dt>
              <dd>{formatTime(material.duration || material.length)}</dd>
              <dt class="text-[#718095]">大小</dt>
              <dd>{formatSize(material.size)}</dd>
              <dt class="text-[#718095]">创建时间</dt>
              <dd class="truncate">{new Date(material.created_at).toLocaleString()}</dd>
              {#if material.note}
                <dt class="text-[#718095]">备注</dt>
                <dd class="break-words">{material.note}</dd>
              {/if}
            </dl>
            <button
              type="button"
              class="mt-2 inline-flex h-8 w-full items-center justify-center gap-1.5 rounded-lg bg-[#0a84ff] text-[12px] font-medium hover:bg-[#0a84ff]/90 disabled:opacity-60"
              disabled={creating}
              onclick={createProject}
            >
              <Clapperboard class="h-3.5 w-3.5" />
              用此素材新建切片
            </button>
          </div>
        {/if}

        <div class="flex items-center justify-between px-4 py-2 text-[11px] text-[#9aa6b8]">
          <span>弹幕</span>
          <span>{danmus.length} 条</span>
        </div>
        <div bind:this={list} class="sidebar-scrollbar flex-1 overflow-y-auto px-2 pb-2">
          {#each danmus as entry, index}
            <button
              type="button"
              data-index={index}
              class="flex w-full gap-2 rounded px-2 py-1 text-left text-[11px] hover:bg-white/5"
              class:bg-white={index === activeIndex}
              class:bg-opacity-10={index === activeIndex}
              onclick={() => {
                if (video) video.currentTime = entry.ts / 1000;
              }}
            >
              <span class="shrink-0 font-mono text-[#6f7b8d]">{formatTime(entry.ts / 1000)}</span>
              <span class="min-w-0 break-words" class:text-amber-300={entry.type === "super_chat"}>
                {entry.content}
              </span>
            </button>
          {:else}
            <p class="px-2 py-6 text-center text-[11px] text-[#718095]">
              素材没有保存弹幕（导入的视频或旧版切片没有弹幕）
            </p>
          {/each}
        </div>
      </aside>
    </div>
  {/if}
</main>
