<script>
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
  import { createEventDispatcher } from "svelte";

  const dispatch = createEventDispatcher();

  export let activeUrl = "总览";

  /**
   * @param {{ detail: String; }} route
   */
  function navigate(route) {
    dispatch("activeChange", route.detail);
  }
</script>

<div
  class="w-48 bg-[#f0f0f3]/50 dark:bg-black backdrop-blur-xl border-r border-gray-200 dark:border-gray-700"
>
  <nav class="p-3 space-y-1">
    <SidebarItem label="总览" {activeUrl} on:activeChange={navigate}>
      <div slot="icon">
        <LayoutDashboard class="w-5 h-5" />
      </div>
    </SidebarItem>
    <SidebarItem label="直播间" {activeUrl} on:activeChange={navigate}>
      <div slot="icon">
        <Video class="w-5 h-5" />
      </div>
    </SidebarItem>
    <SidebarItem label="录播" {activeUrl} on:activeChange={navigate}>
      <div slot="icon">
        <History class="w-5 h-5" />
      </div>
    </SidebarItem>
    <SidebarItem label="切片" {activeUrl} on:activeChange={navigate}>
      <div slot="icon">
        <FileVideo class="w-5 h-5" />
      </div>
    </SidebarItem>
    <SidebarItem label="任务" {activeUrl} on:activeChange={navigate}>
      <div slot="icon">
        <List class="w-5 h-5" />
      </div>
    </SidebarItem>
    <SidebarItem label="助手" {activeUrl} on:activeChange={navigate}>
      <div slot="icon">
        <Brain class="w-5 h-5" />
      </div>
    </SidebarItem>
    <SidebarItem label="账号" {activeUrl} on:activeChange={navigate}>
      <div slot="icon">
        <Users class="w-5 h-5" />
      </div>
    </SidebarItem>
    <SidebarItem label="设置" {activeUrl} on:activeChange={navigate}>
      <div slot="icon">
        <Settings class="w-5 h-5" />
      </div>
    </SidebarItem>
    <SidebarItem
      label="关于"
      {activeUrl}
      on:activeChange={navigate}
      dot={$hasNewVersion}
    >
      <div slot="icon">
        <Info class="w-5 h-5" />
      </div>
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
