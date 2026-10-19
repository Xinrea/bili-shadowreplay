<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { Clapperboard, FileVideo, Film, FolderOpen, Loader2, RefreshCw, Search, X } from "lucide-svelte";
  import { onMount } from "svelte";
  import { ENDPOINT, get_static_url, invoke, TAURI_ENV } from "../../invoker";
  import type { ProjectItem, UploadSource, VideoItem } from "../../interface";
  import type { UploadPick } from "../../uploads";

  interface Props {
    onPick: (pick: UploadPick) => Promise<void>;
    onClose: () => void;
  }

  let { onPick, onClose }: Props = $props();

  const VIDEO_EXTENSIONS = ["mp4", "mkv", "avi", "mov", "wmv", "flv", "m4v", "webm"];
  const tabs = [
    { value: "material", label: "素材", icon: Film },
    { value: "project", label: "切片", icon: Clapperboard },
    { value: "file", label: "视频文件", icon: FileVideo },
  ] as const;

  let tab: UploadSource = $state("project");
  let search = $state("");
  let loading = $state(true);
  let materials: (VideoItem & { coverUrl: string })[] = $state([]);
  let projects: (ProjectItem & { coverUrl: string })[] = $state([]);
  let selectedMaterial: number | null = $state(null);
  let selectedProject: number | null = $state(null);
  let file = $state("");
  let fileSize = $state(0);
  let uploadProgress: number | null = $state(null);
  let fileInput: HTMLInputElement | undefined = $state();
  let creating = $state(false);

  let keyword = $derived(search.trim().toLowerCase());
  let shownMaterials = $derived(
    materials.filter(
      (material) =>
        !keyword ||
        material.title.toLowerCase().includes(keyword) ||
        material.file.toLowerCase().includes(keyword) ||
        material.note.toLowerCase().includes(keyword),
    ),
  );
  let shownProjects = $derived(
    projects.filter(
      (project) =>
        !keyword ||
        project.title.toLowerCase().includes(keyword) ||
        project.note.toLowerCase().includes(keyword),
    ),
  );
  let pick: UploadPick | null = $derived.by(() => {
    if (tab === "material") {
      const item = materials.find((material) => material.id === selectedMaterial);
      return item ? { source: "material", item } : null;
    }
    if (tab === "project") {
      const item = projects.find((project) => project.id === selectedProject);
      return item ? { source: "project", item } : null;
    }
    return file ? { source: "file", file } : null;
  });

  onMount(async () => {
    try {
      const [videoList, projectList] = await Promise.all([
        invoke<VideoItem[]>("get_all_videos"),
        invoke<ProjectItem[]>("get_projects"),
      ]);
      materials = await Promise.all(
        videoList
          .sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at))
          .map(async (material) => ({
            ...material,
            coverUrl: material.cover ? await get_static_url("output", material.cover) : "",
          })),
      );
      projects = await Promise.all(
        projectList.map(async (project) => ({
          ...project,
          coverUrl: project.display_cover ? await get_static_url("output", project.display_cover) : "",
        })),
      );
    } catch (error) {
      alert("读取素材和切片失败：" + error);
    } finally {
      loading = false;
    }
  });

  async function chooseFile() {
    if (!TAURI_ENV) {
      fileInput?.click();
      return;
    }
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: "视频文件", extensions: VIDEO_EXTENSIONS }],
      });
      if (typeof selected !== "string") return;
      file = selected;
      fileSize = await invoke<number>("get_file_size", { filePath: selected }).catch(() => 0);
    } catch (error) {
      alert("选择文件失败：" + error);
    }
  }

  /** The web client cannot read local paths, so the file goes to the server first. */
  function sendFile(selected: File) {
    const data = new FormData();
    data.append("file", selected);
    const request = new XMLHttpRequest();
    uploadProgress = 0;
    request.upload.addEventListener("progress", (event) => {
      if (event.lengthComputable) uploadProgress = Math.round((event.loaded / event.total) * 100);
    });
    request.addEventListener("load", () => {
      uploadProgress = null;
      try {
        const response = JSON.parse(request.responseText);
        if (request.status !== 200 || response.code !== 0) throw new Error(response.message || `HTTP ${request.status}`);
        file = response.data.filePath;
        fileSize = selected.size;
      } catch (error) {
        alert("上传文件失败：" + error);
      }
    });
    request.addEventListener("error", () => {
      uploadProgress = null;
      alert("上传文件失败：网络错误");
    });
    request.open("POST", `${ENDPOINT}/api/upload_file`);
    request.send(data);
  }

  async function confirm() {
    if (!pick || creating) return;
    creating = true;
    try {
      await onPick(pick);
    } finally {
      creating = false;
    }
  }

  function formatDuration(seconds: number) {
    const total = Math.max(0, Math.round(seconds));
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const secs = String(total % 60).padStart(2, "0");
    return hours > 0 ? `${hours}:${String(minutes).padStart(2, "0")}:${secs}` : `${minutes}:${secs}`;
  }

  function formatSize(bytes: number) {
    if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
    if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
    return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  }

  function fileName(path: string) {
    return path.split(/[\\/]/).pop() ?? path;
  }
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
    }
  }}
/>

{#snippet sourceRow(selected: boolean, onSelect: () => void, coverUrl: string, title: string, detail: string, badge: string, posted: string)}
  <button
    type="button"
    class="flex w-full items-center gap-3 rounded-[8px] border px-2.5 py-2 text-left transition-colors {selected
      ? 'border-accent/70 bg-accent-soft/50'
      : 'border-transparent hover:bg-control-hover/60'}"
    aria-pressed={selected}
    onclick={onSelect}
    ondblclick={() => {
      onSelect();
      void confirm();
    }}
  >
    <span class="flex h-9 w-16 shrink-0 items-center justify-center overflow-hidden rounded-[6px] border border-line bg-sunken">
      {#if coverUrl}
        <img src={coverUrl} alt="" class="h-full w-full object-cover" />
      {:else}
        <FileVideo class="h-4 w-4 text-fg-faint" />
      {/if}
    </span>
    <span class="min-w-0 flex-1">
      <span class="block truncate text-[12px] font-medium text-fg" {title}>{title}</span>
      <span class="block truncate text-[11px] text-fg-muted">{detail}</span>
    </span>
    {#if posted}
      <span class="ui-badge ui-badge-success">已投稿</span>
    {/if}
    {#if badge}
      <span class="ui-badge">{badge}</span>
    {/if}
  </button>
{/snippet}

<div class="ui-modal-backdrop">
  <div class="ui-modal h-[min(600px,90vh)] w-[620px] max-w-full" role="dialog" aria-modal="true" aria-labelledby="upload-source-title">
    <div class="ui-modal-header">
      <h3 id="upload-source-title">新建投稿</h3>
      <button type="button" class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost -mr-1.5" aria-label="关闭" onclick={onClose}>
        <X class="h-4 w-4" />
      </button>
    </div>

    <div class="flex shrink-0 items-center justify-between gap-3 border-b border-line px-[18px] py-3">
      <div class="ui-tabs" role="tablist" aria-label="投稿来源">
        {#each tabs as item (item.value)}
          <button type="button" role="tab" class="ui-tab" aria-selected={tab === item.value} onclick={() => (tab = item.value)}>
            <item.icon class="h-3.5 w-3.5" />
            {item.label}
          </button>
        {/each}
      </div>
      {#if tab !== "file"}
        <div class="relative">
          <Search class="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-fg-faint" />
          <input class="ui-input w-48 pl-8" placeholder="搜索标题或备注" bind:value={search} />
        </div>
      {/if}
    </div>

    <div class="ui-modal-body custom-scrollbar-light p-2">
      {#if tab === "file"}
        <div class="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
          <FileVideo class="h-10 w-10 text-fg-faint" />
          {#if file}
            <div class="max-w-full space-y-1">
              <p class="truncate text-[13px] font-medium text-fg" title={file}>{fileName(file)}</p>
              <p class="break-all font-mono text-[11px] text-fg-muted">{file}</p>
              {#if fileSize}
                <p class="text-[11px] text-fg-muted">{formatSize(fileSize)}</p>
              {/if}
            </div>
          {:else}
            <div class="space-y-1">
              <p class="text-[13px] font-medium text-fg">选择要投稿的视频文件</p>
              <p class="ui-hint">
                {TAURI_ENV ? "投稿时直接使用该文件，不会复制到素材库。" : "文件会先上传到服务器，再从服务器投稿。"}
              </p>
            </div>
          {/if}
          {#if uploadProgress !== null}
            <div class="w-64 space-y-1.5">
              <div class="h-1.5 overflow-hidden rounded-full bg-sunken">
                <div class="h-full rounded-full bg-accent transition-all" style="width: {uploadProgress}%"></div>
              </div>
              <p class="text-[11px] tabular-nums text-fg-muted">上传中 {uploadProgress}%</p>
            </div>
          {:else}
            <button type="button" class="ui-btn" onclick={chooseFile}>
              <FolderOpen class="h-3.5 w-3.5" />
              {file ? "重新选择" : "选择文件"}
            </button>
          {/if}
          {#if !TAURI_ENV}
            <input
              bind:this={fileInput}
              type="file"
              accept={VIDEO_EXTENSIONS.map((extension) => "." + extension).join(",") + ",video/*"}
              class="hidden"
              onchange={(event) => {
                const selected = event.currentTarget.files?.[0];
                event.currentTarget.value = "";
                if (selected) sendFile(selected);
              }}
            />
          {/if}
        </div>
      {:else if loading}
        <div class="flex h-full items-center justify-center gap-2 text-[12px] text-fg-muted">
          <RefreshCw class="h-4 w-4 animate-spin" />
          加载中...
        </div>
      {:else if tab === "material"}
        {#if shownMaterials.length === 0}
          <div class="flex h-full items-center justify-center text-[12px] text-fg-muted">
            {materials.length ? "没有找到匹配的素材" : "还没有素材，可在录播页截取或在素材页导入"}
          </div>
        {:else}
          <div class="space-y-0.5">
            {#each shownMaterials as material (material.id)}
              {@render sourceRow(
                selectedMaterial === material.id,
                () => (selectedMaterial = material.id),
                material.coverUrl,
                material.title || fileName(material.file),
                [material.room_id, formatDuration(material.duration || material.length), material.note].filter(Boolean).join(" · "),
                "",
                material.bvid,
              )}
            {/each}
          </div>
        {/if}
      {:else if shownProjects.length === 0}
        <div class="flex h-full items-center justify-center text-[12px] text-fg-muted">
          {projects.length ? "没有找到匹配的切片" : "还没有切片，可在切片页新建"}
        </div>
      {:else}
        <div class="space-y-0.5">
          {#each shownProjects as project (project.id)}
            {@render sourceRow(
              selectedProject === project.id,
              () => (selectedProject = project.id),
              project.coverUrl,
              project.title || "未命名切片",
              [project.room_id, formatDuration(project.duration), `${project.clip_count} 段`].filter(Boolean).join(" · "),
              project.needs_render ? "投稿时生成视频" : "使用素材文件",
              project.bvid,
            )}
          {/each}
        </div>
      {/if}
    </div>

    <div class="ui-modal-footer">
      <button type="button" class="ui-btn" onclick={onClose}>取消</button>
      <button type="button" class="ui-btn ui-btn-primary" disabled={!pick || creating} onclick={confirm}>
        {#if creating}
          <Loader2 class="h-3.5 w-3.5 animate-spin" />
        {/if}
        下一步
      </button>
    </div>
  </div>
</div>
