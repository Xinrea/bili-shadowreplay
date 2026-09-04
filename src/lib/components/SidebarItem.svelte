<script lang="ts">
  import { Info, LayoutDashboard, Settings, Users, Video } from "lucide-svelte";
  interface Props {
    // acitveUrl is shared between project
    activeUrl?: string;
    label?: string;
    dot?: boolean;
    icon?: import('svelte').Snippet<[any]>;
    onActiveChange?: (label: string) => void;
  }

  let {
    activeUrl = "总览",
    label = "",
    dot = false,
    icon,
    onActiveChange
  }: Props = $props();
</script>

<button
  onclick={() => onActiveChange?.(label)}
  class="flex w-full items-center space-x-2 px-3 py-2 rounded-lg {activeUrl ===
  label
    ? 'bg-blue-500/10 text-[#0A84FF] dark:bg-transparent dark:text-white'
    : 'text-gray-700 dark:text-white'} hover:bg-[#e5e5e5] dark:hover:bg-transparent"
>
  {@render icon?.({ class: activeUrl === label
      ? "text-[#0A84FF] dark:text-white"
      : "text-gray-700 dark:text-white", })}
  <span>{label}</span>
  {#if dot}
    <div class="absolute right-6 w-2 h-2 bg-red-500 rounded-full"></div>
  {/if}
</button>
