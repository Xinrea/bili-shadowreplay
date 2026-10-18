<script lang="ts">
  import MaterialPreview from "./lib/components/MaterialPreview.svelte";
  import ProjectEditor from "./lib/components/project/ProjectEditor.svelte";
  import TaskCenter from "./lib/components/TaskCenter.svelte";
  import TitleBar from "./lib/components/TitleBar.svelte";

  const params = new URLSearchParams(window.location.search);
  const projectId = Number(params.get("project")) || null;
  // `id` is the material link used before projects existed.
  const materialId = Number(params.get("material") ?? params.get("id")) || null;
</script>

<TaskCenter />

{#if projectId}
  <ProjectEditor {projectId} />
{:else if materialId}
  <MaterialPreview {materialId} />
{:else}
  <main class="flex h-screen flex-col bg-[#1c1c1e] text-white">
    <TitleBar tone="dark" title="切片" />
    <div class="flex flex-1 items-center justify-center text-sm text-gray-400">没有指定要打开的切片或素材</div>
  </main>
{/if}
