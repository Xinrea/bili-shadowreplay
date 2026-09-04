<script lang="ts">
  import {
    FileVideo,
    Info,
    LayoutDashboard,
    List,
    Settings,
    Users,
    Video,
    Brain,
    History,
  } from "lucide-svelte";
  import { hasNewVersion } from "../stores/version";
  import SidebarItem from "./SidebarItem.svelte";
  interface Props {
    activeUrl?: string;
    onActiveChange?: (label: string) => void;
  }

  let { activeUrl = $bindable("总览"), onActiveChange }: Props = $props();

  /**
   * @param {{ detail: String; }} route
   */
  function navigate(label: string) {
    activeUrl = label;
    onActiveChange?.(label);
  }
</script>

<div
  class="w-48 bg-[#f0f0f3]/50 dark:bg-black backdrop-blur-xl border-r border-gray-200 dark:border-gray-700"
>
  <nav class="p-3 space-y-1">
    <SidebarItem label="总览" {activeUrl} onActiveChange={navigate}>
      {#snippet icon()}
            <div >
          <LayoutDashboard class="w-5 h-5" />
        </div>
          {/snippet}
    </SidebarItem>
    <SidebarItem label="直播间" {activeUrl} onActiveChange={navigate}>
      {#snippet icon()}
            <div >
          <Video class="w-5 h-5" />
        </div>
          {/snippet}
    </SidebarItem>
    <SidebarItem label="录播" {activeUrl} onActiveChange={navigate}>
      {#snippet icon()}
            <div >
          <History class="w-5 h-5" />
        </div>
          {/snippet}
    </SidebarItem>
    <SidebarItem label="切片" {activeUrl} onActiveChange={navigate}>
      {#snippet icon()}
            <div >
          <FileVideo class="w-5 h-5" />
        </div>
          {/snippet}
    </SidebarItem>
    <SidebarItem label="任务" {activeUrl} onActiveChange={navigate}>
      {#snippet icon()}
            <div >
          <List class="w-5 h-5" />
        </div>
          {/snippet}
    </SidebarItem>
    <SidebarItem label="助手" {activeUrl} onActiveChange={navigate}>
      {#snippet icon()}
            <div >
          <Brain class="w-5 h-5" />
        </div>
          {/snippet}
    </SidebarItem>
    <SidebarItem label="账号" {activeUrl} onActiveChange={navigate}>
      {#snippet icon()}
            <div >
          <Users class="w-5 h-5" />
        </div>
          {/snippet}
    </SidebarItem>
    <SidebarItem label="设置" {activeUrl} onActiveChange={navigate}>
      {#snippet icon()}
            <div >
          <Settings class="w-5 h-5" />
        </div>
          {/snippet}
    </SidebarItem>
    <SidebarItem
      label="关于"
      {activeUrl}
      onActiveChange={navigate}
      dot={$hasNewVersion}
    >
      {#snippet icon()}
            <div >
          <Info class="w-5 h-5" />
        </div>
          {/snippet}
    </SidebarItem>
  </nav>
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
