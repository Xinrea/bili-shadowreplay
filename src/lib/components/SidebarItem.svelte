<script lang="ts">
  import { Info, LayoutDashboard, Settings, Users, Video } from "lucide-svelte";
  interface Props {
    // acitveUrl is shared between project
    activeUrl?: string;
    label?: string;
    /** Icon only; the label is clipped and shown as a tooltip. */
    collapsed?: boolean;
    dot?: boolean;
    icon?: import('svelte').Snippet<[any]>;
    onActiveChange?: (label: string) => void;
  }

  let {
    activeUrl = "总览",
    label = "",
    collapsed = false,
    dot = false,
    icon,
    onActiveChange
  }: Props = $props();
</script>

<!-- The icon keeps its position in both widths; collapsing only clips the label. -->
<button
  onclick={() => onActiveChange?.(label)}
  title={collapsed ? label : undefined}
  class="relative flex h-8 w-full items-center gap-2.5 overflow-hidden whitespace-nowrap rounded-[8px] px-2.5 text-[13px] transition-colors {activeUrl ===
  label
    ? 'bg-accent-soft font-medium text-accent-fg'
    : 'text-fg-secondary hover:bg-control-hover hover:text-fg'}"
>
  {@render icon?.({})}
  <span>{label}</span>
  {#if dot}
    <span
      class="h-1.5 w-1.5 shrink-0 rounded-full bg-danger {collapsed
        ? 'absolute left-[23px] top-[7px]'
        : 'ml-auto'}"
    ></span>
  {/if}
</button>
