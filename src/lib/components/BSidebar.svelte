<script module lang="ts">
  export const SIDEBAR_WIDTH = 192;
  /** Collapsed: an icon rail, centered on the icons' fixed position. */
  export const SIDEBAR_RAIL_WIDTH = 52;
</script>

<script lang="ts">
  import { Command } from "lucide-svelte";
  import { PALETTE_SHORTCUT } from "../command-palette";
  import { pages } from "../pages";
  import { openCommandPalette } from "../stores/commands";
  import { hasNewVersion } from "../stores/version";
  import {
    MAC_TRAFFIC_LIGHTS,
    TRAFFIC_LIGHTS_INSET,
    windowDrag,
    windowState,
  } from "../window-chrome";
  import SidebarItem from "./SidebarItem.svelte";
  import SidebarToggle from "./SidebarToggle.svelte";
  interface Props {
    activeUrl?: string;
    collapsed?: boolean;
    onActiveChange?: (label: string) => void;
    onToggleCollapsed?: () => void;
  }

  let {
    activeUrl = $bindable("总览"),
    collapsed = false,
    onActiveChange,
    onToggleCollapsed,
  }: Props = $props();

  // On macOS the toggle sits right after the traffic lights. The rail is too
  // narrow for that, so while collapsed the window title bar shows it instead.
  let lightsInset = $derived(MAC_TRAFFIC_LIGHTS && !$windowState.fullscreen);

  /**
   * @param {{ detail: String; }} route
   */
  function navigate(label: string) {
    activeUrl = label;
    onActiveChange?.(label);
  }
</script>

<div
  class="flex shrink-0 flex-col overflow-hidden bg-chrome transition-[width] duration-200"
  style:width="{collapsed ? SIDEBAR_RAIL_WIDTH : SIDEBAR_WIDTH}px"
>
  <!-- Title bar part of the sidebar; on macOS the traffic lights sit here. -->
  <div
    class="flex h-10 shrink-0 items-center gap-2 overflow-hidden pr-3"
    style:padding-left="{lightsInset ? TRAFFIC_LIGHTS_INSET : 12}px"
    use:windowDrag
  >
    {#if !(MAC_TRAFFIC_LIGHTS && collapsed)}
      <SidebarToggle {collapsed} onToggle={() => onToggleCollapsed?.()} />
    {/if}
    {#if !MAC_TRAFFIC_LIGHTS && !collapsed}
      <img src="/icon.svg" alt="" class="h-5 w-5 shrink-0" />
      <span class="truncate text-[13px] font-semibold text-fg">
        ShadowReplay
      </span>
    {/if}
  </div>
  <!-- The divider starts below the title row, so the traffic lights never sit on it. -->
  <div class="flex min-h-0 flex-1 flex-col border-r border-line px-2 pb-3 pt-1">
    <nav class="space-y-0.5">
      {#each pages as page (page.label)}
        <SidebarItem
          label={page.label}
          {activeUrl}
          {collapsed}
          onActiveChange={navigate}
          dot={page.label === "关于" && $hasNewVersion}
        >
          {#snippet icon()}
            <page.icon class="h-4 w-4 shrink-0" />
          {/snippet}
        </SidebarItem>
      {/each}
    </nav>

    <!-- Like a sidebar item, so the rail keeps just its icon. -->
    <button
      type="button"
      class="mt-auto flex h-8 w-full items-center gap-2.5 overflow-hidden whitespace-nowrap rounded-[8px] px-2.5 text-[13px] text-fg-muted transition-colors hover:bg-control-hover hover:text-fg"
      title="命令面板（{PALETTE_SHORTCUT}）"
      onclick={openCommandPalette}
    >
      <Command class="h-4 w-4 shrink-0" />
      <span>命令面板</span>
      <kbd class="ml-auto font-sans text-[10px] text-fg-faint">{PALETTE_SHORTCUT}</kbd>
    </button>
  </div>
</div>

<style>
  :global(img.text-\[\#0A84FF\]) {
    filter: invert(48%) sepia(85%) saturate(2229%) hue-rotate(198deg)
      brightness(100%) contrast(101%);
  }

  :global(img.text-gray-700) {
    filter: invert(23%) sepia(10%) saturate(532%) hue-rotate(182deg)
      brightness(94%) contrast(90%);
  }

  :global(.dark img.text-\[\#0A84FF\]) {
    filter: invert(48%) sepia(85%) saturate(2229%) hue-rotate(198deg)
      brightness(100%) contrast(101%);
  }
</style>
