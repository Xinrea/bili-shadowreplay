<script lang="ts">
  import { PanelLeftClose, PanelLeftOpen } from "lucide-svelte";

  interface Props {
    collapsed: boolean;
    onToggle: () => void;
  }

  let { collapsed, onToggle }: Props = $props();
</script>

<button
  type="button"
  class="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-[7px] text-fg-muted transition-colors hover:bg-control-hover hover:text-fg focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent"
  title={collapsed ? "展开侧边栏" : "收起侧边栏"}
  aria-label={collapsed ? "展开侧边栏" : "收起侧边栏"}
  aria-expanded={!collapsed}
  onclick={onToggle}
>
  {#if collapsed}
    <PanelLeftOpen class="h-4 w-4" />
  {:else}
    <PanelLeftClose class="h-4 w-4" />
  {/if}
</button>
