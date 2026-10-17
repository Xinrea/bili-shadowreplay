<script lang="ts">
  import { ListChecks, Loader2 } from "lucide-svelte";
  import { activeTasks, openTaskCenter } from "../stores/tasks";

  interface Props {
    /** "dark" for always-dark surfaces such as the clip inspector. */
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
      ? 'bg-blue-50 text-blue-600 hover:bg-blue-100 dark:bg-blue-500/15 dark:text-blue-300 dark:hover:bg-blue-500/25'
      : 'bg-gray-100 text-gray-600 hover:bg-gray-200 dark:bg-gray-800 dark:text-gray-300 dark:hover:bg-gray-700'}"
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
