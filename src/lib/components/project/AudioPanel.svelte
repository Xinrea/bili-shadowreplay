<script lang="ts" module>
  export interface AudioEntry {
    id: number;
    name: string;
    /** Seconds. */
    duration: number;
    url: string;
    /** Relative to the output directory. */
    file: string;
    createdAt: string;
  }
</script>

<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { Loader2, Music, Pause, Pencil, Play, Plus, Search, Trash2, Upload } from "lucide-svelte";
  import { onDestroy } from "svelte";
  import { ENDPOINT, invoke, TAURI_ENV } from "../../invoker";
  import { formatTimecode } from "../../time-format";
  import { AUDIO_DRAG_TYPE } from "./ProjectTimeline.svelte";

  // The audio library: music, voiceover and other sound files shared by every
  // project, to drag onto the audio tracks or add at the playhead.

  interface Props {
    files: AudioEntry[];
    /** Add it to the audio tracks at the playhead. */
    onAdd: (file: AudioEntry) => void;
  }

  let { files, onAdd }: Props = $props();

  const AUDIO_EXTENSIONS = ["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus", "wma", "aif", "aiff"];
  const VIDEO_EXTENSIONS = ["mp4", "mov", "mkv", "webm", "flv", "m4v"];

  let search = $state("");
  /** What an import is doing, or empty when none is running. */
  let importing = $state("");
  let fileInput: HTMLInputElement = $state();
  let player: HTMLAudioElement = $state();
  let playingId: number | null = $state(null);

  let shown = $derived.by(() => {
    const keyword = search.trim().toLowerCase();
    return files.filter((file) => !keyword || file.name.toLowerCase().includes(keyword));
  });

  /** A file's name without its extension. */
  function stem(name: string) {
    const dot = name.lastIndexOf(".");
    return dot > 0 ? name.slice(0, dot) : name;
  }

  async function importPaths(paths: string[]) {
    for (const [index, path] of paths.entries()) {
      importing = paths.length > 1 ? `正在导入 ${index + 1}/${paths.length}` : "正在导入";
      try {
        await invoke("import_audio_file", { filePath: path });
      } catch (error) {
        alert(`导入「${path.split(/[\\/]/).pop()}」失败：${error}`);
      }
    }
  }

  /** Send `file` to the server, which keeps it until it is imported. */
  function upload(file: File, progress: (percent: number) => void): Promise<string> {
    return new Promise((resolve, reject) => {
      const form = new FormData();
      form.append("file", file);
      const request = new XMLHttpRequest();
      request.upload.addEventListener("progress", (event) => {
        if (event.lengthComputable) progress(Math.round((event.loaded / event.total) * 100));
      });
      request.addEventListener("load", () => {
        try {
          const response = JSON.parse(request.responseText);
          if (request.status === 200 && response.code === 0 && response.data) resolve(response.data.filePath);
          else reject(response.message || `HTTP ${request.status}`);
        } catch {
          reject(`HTTP ${request.status}`);
        }
      });
      request.addEventListener("error", () => reject("网络错误"));
      request.open("POST", `${ENDPOINT}/api/upload_file`);
      request.send(form);
    });
  }

  async function importUploads(list: File[]) {
    for (const [index, file] of list.entries()) {
      const count = list.length > 1 ? ` ${index + 1}/${list.length}` : "";
      try {
        const path = await upload(file, (percent) => (importing = `正在上传${count} ${percent}%`));
        importing = `正在导入${count}`;
        await invoke("import_audio_file", { filePath: path, name: stem(file.name) });
      } catch (error) {
        alert(`导入「${file.name}」失败：${error}`);
      }
    }
  }

  async function startImport() {
    if (importing) return;
    if (!TAURI_ENV) {
      fileInput?.click();
      return;
    }
    const selected = await open({
      multiple: true,
      filters: [
        { name: "音频文件", extensions: AUDIO_EXTENSIONS },
        { name: "视频文件（只取声音）", extensions: VIDEO_EXTENSIONS },
      ],
    });
    if (!selected) return;
    try {
      await importPaths(Array.isArray(selected) ? selected : [selected]);
    } finally {
      importing = "";
    }
  }

  async function pickedFiles(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const list = [...(input.files ?? [])];
    input.value = "";
    if (list.length === 0) return;
    try {
      await importUploads(list);
    } finally {
      importing = "";
    }
  }

  function togglePreview(file: AudioEntry) {
    if (!player) return;
    if (playingId === file.id) {
      player.pause();
      playingId = null;
      return;
    }
    player.src = file.url;
    playingId = file.id;
    player.play().catch(() => (playingId = null));
  }

  async function rename(file: AudioEntry) {
    const name = prompt("重命名音频", file.name)?.trim();
    if (!name || name === file.name) return;
    try {
      await invoke("rename_audio_file", { id: file.id, name });
    } catch (error) {
      alert("重命名失败：" + error);
    }
  }

  async function remove(file: AudioEntry) {
    let users: { id: number; title: string }[] = [];
    try {
      users = await invoke("get_audio_usage", { id: file.id });
    } catch (error) {
      alert("读取音频的使用情况失败：" + error);
      return;
    }
    const titles = users.map((project) => `「${project.title || "未命名切片"}」`).join("、");
    const message = users.length
      ? `「${file.name}」正在被 ${users.length} 个切片使用：${titles}。删除后会从这些切片的音频轨上移除，确定删除吗？`
      : `确定删除「${file.name}」吗？`;
    if (!confirm(message)) return;
    if (playingId === file.id) {
      player?.pause();
      playingId = null;
    }
    try {
      await invoke("delete_audio_file", { id: file.id });
    } catch (error) {
      alert("删除失败：" + error);
    }
  }

  function dragStart(event: DragEvent, file: AudioEntry) {
    event.dataTransfer?.setData(AUDIO_DRAG_TYPE, String(file.id));
    if (event.dataTransfer) event.dataTransfer.effectAllowed = "copy";
  }

  onDestroy(() => player?.pause());
</script>

<aside class="library" aria-label="音频库">
  <div class="library-head">
    <div class="flex items-center justify-between">
      <h2>音频库</h2>
      <button type="button" class="import" disabled={!!importing} onclick={() => void startImport()}>
        {#if importing}
          <Loader2 class="h-3.5 w-3.5 animate-spin" />
          {importing}
        {:else}
          <Upload class="h-3.5 w-3.5" />
          导入
        {/if}
      </button>
    </div>
    <label class="search">
      <Search class="h-3.5 w-3.5 shrink-0 text-[#6f7b8d]" />
      <input bind:value={search} placeholder="搜索音频" data-hotkey-block />
    </label>
  </div>

  <div class="library-list sidebar-scrollbar">
    {#each shown as file (file.id)}
      <div
        class="item"
        role="listitem"
        draggable="true"
        ondragstart={(event) => dragStart(event, file)}
        title="拖到音频轨道上，或点击 ＋ 在播放头处添加"
      >
        <button
          type="button"
          class="thumb"
          class:playing={playingId === file.id}
          title={playingId === file.id ? "停止试听" : "试听"}
          aria-label={playingId === file.id ? "停止试听" : "试听"}
          onclick={() => togglePreview(file)}
        >
          {#if playingId === file.id}
            <Pause class="h-3.5 w-3.5" />
          {:else}
            <Play class="h-3.5 w-3.5" />
          {/if}
        </button>
        <div class="meta">
          <span class="name" title={file.name}>{file.name}</span>
          <span class="length">{formatTimecode(file.duration, 0)}</span>
        </div>
        <button type="button" class="action" title="重命名" aria-label="重命名" onclick={() => void rename(file)}>
          <Pencil class="h-3 w-3" />
        </button>
        <button type="button" class="action danger" title="删除" aria-label="删除" onclick={() => void remove(file)}>
          <Trash2 class="h-3 w-3" />
        </button>
        <button type="button" class="action add" title="在播放头处添加到音频轨道" onclick={() => onAdd(file)}>
          <Plus class="h-3.5 w-3.5" />
        </button>
      </div>
    {:else}
      <div class="empty">
        {#if files.length === 0}
          <Music class="mx-auto mb-2 h-5 w-5" />
          导入背景音乐、配音或音效，所有切片都能使用；也可以把素材拖到音频轨道上，只用它的声音
        {:else}
          没有匹配的音频
        {/if}
      </div>
    {/each}
  </div>

  <audio bind:this={player} onended={() => (playingId = null)}></audio>
  <input
    bind:this={fileInput}
    type="file"
    multiple
    accept={["audio/*", ...AUDIO_EXTENSIONS.map((extension) => `.${extension}`), "video/*"].join(",")}
    class="hidden"
    onchange={pickedFiles}
  />
</aside>

<style>
  .library {
    display: flex;
    width: 248px;
    flex-shrink: 0;
    flex-direction: column;
    border-right: 1px solid #222a35;
    background: #171c25;
    color: #dce5f3;
  }

  .library-head {
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

  .import {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border-radius: 6px;
    background: rgb(10 132 255 / 16%);
    padding: 3px 8px;
    color: #7cc4ff;
    font-size: 11px;
  }

  .import:hover:not(:disabled) {
    background: rgb(10 132 255 / 26%);
  }

  .import:disabled {
    opacity: 0.7;
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

  .library-list {
    flex: 1;
    overflow-y: auto;
    padding: 6px;
  }

  .item {
    display: flex;
    align-items: center;
    gap: 6px;
    border-radius: 8px;
    padding: 6px;
    cursor: grab;
  }

  .item:hover {
    background: rgb(255 255 255 / 4%);
  }

  .thumb {
    display: flex;
    width: 32px;
    height: 32px;
    flex-shrink: 0;
    align-items: center;
    justify-content: center;
    border: 1px solid #293442;
    border-radius: 6px;
    background: rgb(52 211 153 / 10%);
    color: #6ee7b7;
  }

  .thumb.playing {
    border-color: rgb(52 211 153 / 60%);
    background: rgb(52 211 153 / 22%);
  }

  .meta {
    display: flex;
    min-width: 0;
    flex: 1;
    flex-direction: column;
    gap: 2px;
  }

  .name {
    overflow: hidden;
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .length {
    color: #718095;
    font-family: ui-monospace, monospace;
    font-size: 10px;
  }

  .action {
    display: inline-flex;
    flex-shrink: 0;
    border-radius: 6px;
    padding: 4px;
    color: #8c98aa;
  }

  .item:not(:hover) .action:not(.add) {
    visibility: hidden;
  }

  .action:hover {
    background: rgb(10 132 255 / 20%);
    color: #55b9ff;
  }

  .action.danger:hover {
    background: rgb(248 113 113 / 16%);
    color: #fca5a5;
  }

  .empty {
    padding: 24px 12px;
    color: #718095;
    font-size: 11px;
    line-height: 1.6;
    text-align: center;
  }
</style>
