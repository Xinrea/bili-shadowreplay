<script lang="ts">
  import { ListChecks, Loader2 } from "lucide-svelte";
  import { activeTasks, openTaskCenter } from "../stores/tasks";

  interface Props {
    /** "dark" for the preview windows; "adaptive" follows the theme. */
    tone?: "adaptive" | "dark";
  }

  let { tone = "adaptive" }: Props = $props();

  let count = $derived($activeTasks.length);
</script>

<button
  type="button"
  class="inline-flex h-7 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-[7px] px-2.5 text-[11px] font-medium transition-colors {tone === 'dark'
    ? count > 0
      ? 'bg-[#0A84FF]/15 text-[#4da3ff] hover:bg-[#0A84FF]/25'
      : 'bg-[#222a36] text-[#8c98aa] hover:text-[#dce5f3]'
    : count > 0
      ? 'bg-accent-soft text-accent-fg hover:bg-accent-soft/80'
      : 'bg-control text-fg-secondary hover:bg-control-hover hover:text-fg'}"
  title="查看进行中的任务"
  onclick={() => openTaskCenter()}
>
  {#if count > 0}
    <Loader2 class="h-3.5 w-3.5 animate-spin" />
    <span class="tabular-nums">{count} 个任务</span>
  {:else}
    <ListChecks class="h-3.5 w-3.5" />
    <span>任务</span>
  {/if}
</button>
