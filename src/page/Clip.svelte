<script lang="ts">
  import { createStateRefresh, watchStateChanges } from "../lib/state-refresh";
  import { invoke, TAURI_ENV, get_static_url } from "../lib/invoker";
  import type { ProjectExportState, ProjectItem } from "../lib/interface";
  import PageTitlebar from "../lib/components/PageTitlebar.svelte";
  import { onMount } from "svelte";
  import {
    Calendar,
    Clapperboard,
    Clock,
    Download,
    Pencil,
    Plus,
    RefreshCw,
    Search,
    Trash2,
    Upload,
  } from "lucide-svelte";
  import { registerCommands } from "../lib/stores/commands";

  interface ProjectRow extends ProjectItem {
    coverUrl: string;
  }

  let projects: ProjectRow[] = $state([]);
  let loading = $state(true);
  let selectedRoomId: string | null = $state(null);
  let search = $state("");
  let sortBy: "updated_at" | "created_at" | "title" | "duration" = $state("updated_at");
  let projectToDelete: ProjectRow | null = $state(null);
  let creating = $state(false);

  let roomIds = $derived([...new Set(projects.map((project) => project.room_id))].filter(Boolean).sort());
  let filtered = $derived.by(() => {
    const keyword = search.trim().toLowerCase();
    return projects
      .filter((project) => selectedRoomId === null || project.room_id === selectedRoomId)
      .filter(
        (project) =>
          !keyword ||
          project.title.toLowerCase().includes(keyword) ||
          project.note.toLowerCase().includes(keyword),
      )
      .sort((a, b) => {
        switch (sortBy) {
          case "title":
            return a.title.localeCompare(b.title);
          case "duration":
            return b.duration - a.duration;
          default:
            return new Date(b[sortBy]).getTime() - new Date(a[sortBy]).getTime();
        }
      });
  });

  const stateRefresh = createStateRefresh(async () => {
    try {
      const list = await invoke<ProjectItem[]>("get_projects");
      projects = await Promise.all(
        list.map(async (project) => ({
          ...project,
          coverUrl: project.display_cover ? await get_static_url("output", project.display_cover) : "",
        })),
      );
    } finally {
      loading = false;
    }
  });

  onMount(() => watchStateChanges(stateRefresh, ["projects-changed", "videos-changed"]));

  onMount(() =>
    registerCommands([
      {
        id: "project:new",
        group: "action",
        title: "新建切片",
        detail: "切片",
        icon: Plus,
        keywords: ["new project", "create clip"],
        page: "切片",
        run: createProject,
      },
      {
        id: "project:refresh",
        group: "action",
        title: "刷新切片列表",
        detail: "切片",
        icon: RefreshCw,
        keywords: ["refresh projects", "reload"],
        page: "切片",
        run: () => stateRefresh.run(),
      },
    ]),
  );

  async function createProject() {
    if (creating) return;
    creating = true;
    try {
      const project = await invoke<ProjectItem>("create_project", {
        title: "",
        roomId: selectedRoomId ?? "",
        platform: projects.find((item) => item.room_id === selectedRoomId)?.platform ?? "",
        materialIds: [],
      });
      await openProject(project.id);
    } catch (error) {
      alert("新建切片失败：" + error);
    } finally {
      creating = false;
    }
  }

  async function openProject(id: number) {
    try {
      await invoke("open_project", { projectId: id });
    } catch (error) {
      console.error("Failed to open project:", error);
    }
  }

  async function deleteProject(project: ProjectRow) {
    try {
      await invoke("delete_project", { id: project.id });
      projectToDelete = null;
      await stateRefresh.run();
    } catch (error) {
      alert("删除切片失败：" + error);
    }
  }

  async function downloadExport(project: ProjectRow) {
    const a = document.createElement("a");
    a.href = await get_static_url("output", project.output);
    a.download = `${project.title || "切片"}.mp4`;
    a.click();
  }

  function formatDuration(seconds: number) {
    const total = Math.max(0, Math.round(seconds));
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const secs = total % 60;
    return hours > 0
      ? `${hours}:${minutes.toString().padStart(2, "0")}:${secs.toString().padStart(2, "0")}`
      : `${minutes}:${secs.toString().padStart(2, "0")}`;
  }

  const exportLabels: Record<ProjectExportState, { label: string; tone: string }> = {
    material: { label: "无需导出", tone: "text-fg-muted" },
    none: { label: "未导出", tone: "text-fg-faint" },
    stale: { label: "需重新导出", tone: "text-warning-fg" },
    ready: { label: "已导出", tone: "text-success-fg" },
  };
</script>

<PageTitlebar title="切片">
  {#snippet subtitle()}
    <span class="truncate text-[12px] text-fg-muted">
      切片是由一个或多个素材组成的剪辑工程，可裁剪、分割、添加字幕和压制弹幕，导出后投稿。
    </span>
  {/snippet}
  <button
    class="titlebar-icon-button"
    title="刷新"
    aria-label="刷新"
    onclick={() => stateRefresh.run()}
  >
    <RefreshCw class="h-4 w-4" />
  </button>
  <button
    class="inline-flex h-7 items-center gap-1.5 whitespace-nowrap rounded-[7px] bg-accent px-2.5 text-[12px] font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-60"
    disabled={creating}
    onclick={createProject}
  >
    <Plus class="h-3.5 w-3.5" />
    新建切片
  </button>
</PageTitlebar>

<div class="flex-1 overflow-auto bg-canvas p-5 custom-scrollbar-light">
  <div class="space-y-4">
    <div class="ui-card px-3 py-2.5">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div class="flex items-center gap-2">
          <select bind:value={selectedRoomId} class="ui-input min-w-[160px]">
            <option value={null}>所有直播间</option>
            {#each roomIds as roomId}
              <option value={roomId}>{roomId}</option>
            {/each}
          </select>
          <div class="relative">
            <Search class="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-fg-faint" />
            <input
              class="ui-input w-56 pl-8"
              placeholder="搜索标题或备注"
              bind:value={search}
            />
          </div>
        </div>
        <div class="flex items-center gap-1.5">
          <span class="mr-0.5 text-[12px] text-fg-muted">排序:</span>
          {#each [["updated_at", "最近编辑"], ["created_at", "创建时间"], ["title", "标题"], ["duration", "时长"]] as [value, label]}
            <button
              class="ui-btn ui-btn-sm {sortBy === value ? 'ui-btn-active' : ''}"
              onclick={() => (sortBy = value as typeof sortBy)}
            >
              {label}
            </button>
          {/each}
        </div>
      </div>
    </div>

    <div class="ui-card overflow-hidden">
      {#if loading}
        <div class="flex flex-col items-center justify-center space-y-3 p-12 text-[12px] text-fg-muted">
          <RefreshCw class="h-6 w-6 animate-spin text-fg-faint" />
          <span>加载中...</span>
        </div>
      {:else if filtered.length === 0}
        <div class="flex flex-col items-center justify-center space-y-2 p-12 text-fg-muted">
          <Clapperboard class="h-10 w-10 text-fg-faint" />
          <h3 class="text-[14px] font-semibold text-fg">暂无切片</h3>
          <p class="text-[12px]">
            {projects.length > 0
              ? "没有找到匹配的切片"
              : "在素材页选择素材新建切片，或生成素材时勾选「同时新建切片」"}
          </p>
        </div>
      {:else}
        <div class="overflow-x-auto custom-scrollbar-light">
          <table class="w-full table-fixed whitespace-nowrap">
            <thead>
              <tr class="border-b border-line bg-control/60">
                <th class="w-80 px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted">切片</th>
                <th class="w-28 px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted">直播间</th>
                <th class="w-32 px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted">内容</th>
                <th class="w-36 px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted">状态</th>
                <th class="w-40 px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted">最近编辑</th>
                <th class="w-28 px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted">操作</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-line">
              {#each filtered as project (project.id)}
                <tr class="group transition-colors hover:bg-control-hover/50">
                  <td class="px-4 py-2.5">
                    <button
                      class="flex w-full items-center gap-3 text-left"
                      title="编辑切片"
                      onclick={() => openProject(project.id)}
                    >
                      <div class="flex h-10 w-16 flex-shrink-0 items-center justify-center overflow-hidden rounded-[6px] border border-line bg-sunken">
                        {#if project.coverUrl}
                          <img src={project.coverUrl} alt="封面" class="h-full w-full object-cover" />
                        {:else}
                          <Clapperboard class="h-4 w-4 text-fg-faint" />
                        {/if}
                      </div>
                      <div class="min-w-0 flex-1">
                        <p class="truncate text-[12px] font-medium text-fg" title={project.title}>
                          {project.title || "未命名切片"}
                        </p>
                        <p class="truncate text-[11px] text-fg-muted" title={project.note}>
                          {project.note || "无备注"}
                        </p>
                      </div>
                    </button>
                  </td>
                  <td class="px-4 py-2.5">
                    <span class="truncate text-[12px] text-fg-secondary">{project.room_id || "—"}</span>
                  </td>
                  <td class="px-4 py-2.5">
                    <div class="flex items-center gap-2 text-[12px] text-fg-secondary">
                      <Clock class="table-icon text-fg-faint" />
                      <span class="tabular-nums">{formatDuration(project.duration)}</span>
                      <span class="text-fg-faint">· {project.clip_count} 段</span>
                    </div>
                  </td>
                  <td class="px-4 py-2.5">
                    <div class="flex flex-col gap-0.5 text-[12px]">
                      <span class={exportLabels[project.export_state].tone}>
                        {exportLabels[project.export_state].label}
                      </span>
                      {#if project.bvid}
                        <a
                          href={`https://www.bilibili.com/video/${project.bvid}`}
                          target="_blank"
                          rel="noopener noreferrer"
                          class="inline-flex items-center gap-1 truncate text-[11px] text-accent-fg hover:underline"
                        >
                          <Upload class="h-3 w-3" />
                          {project.bvid}
                        </a>
                      {/if}
                    </div>
                  </td>
                  <td class="px-4 py-2.5">
                    <div class="flex items-center gap-2 text-[12px] text-fg-secondary">
                      <Calendar class="table-icon text-fg-faint" />
                      <span class="truncate">{new Date(project.updated_at).toLocaleString()}</span>
                    </div>
                  </td>
                  <td class="px-4 py-2.5">
                    <div class="flex items-center gap-1">
                      <button
                        class="action-button hover:bg-accent-soft"
                        title="编辑"
                        onclick={() => openProject(project.id)}
                      >
                        <Pencil class="h-3.5 w-3.5 text-accent-fg" />
                      </button>
                      {#if !TAURI_ENV && project.export_state === "ready"}
                        <button
                          class="action-button hover:bg-accent-soft"
                          title="下载导出文件"
                          onclick={() => downloadExport(project)}
                        >
                          <Download class="h-3.5 w-3.5 text-accent-fg" />
                        </button>
                      {/if}
                      <button
                        class="action-button hover:bg-danger-soft"
                        title="删除"
                        onclick={() => (projectToDelete = project)}
                      >
                        <Trash2 class="h-3.5 w-3.5 text-danger-fg" />
                      </button>
                    </div>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </div>
  </div>
</div>

{#if projectToDelete}
  <div class="ui-modal-backdrop">
    <div class="ui-modal w-[400px] max-w-full">
      <div class="space-y-4 p-6">
        <div class="space-y-2 text-center">
          <h3 class="text-[14px] font-semibold text-fg">确认删除</h3>
          <p class="text-[12px] text-fg-muted">
            确定要删除切片「{projectToDelete.title || "未命名切片"}」吗？
          </p>
          <p class="text-[11px] text-fg-muted">素材不会被删除，切片的导出文件和封面会一并删除。</p>
        </div>
        <div class="flex justify-center gap-2">
          <button class="ui-btn w-24" onclick={() => (projectToDelete = null)}>取消</button>
          <button class="ui-btn ui-btn-danger w-24" onclick={() => deleteProject(projectToDelete)}>
            删除
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .action-button {
    display: inline-flex;
    width: 26px;
    height: 26px;
    align-items: center;
    justify-content: center;
    border-radius: 7px;
    transition: background-color 120ms ease;
  }

  .action-button:focus-visible {
    outline: 2px solid rgb(var(--c-accent));
    outline-offset: 1px;
  }
</style>
