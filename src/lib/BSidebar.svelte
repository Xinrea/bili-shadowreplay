<script>
  import { Info, LayoutDashboard, Settings, Users, Video } from "lucide-svelte";
  import { hasNewVersion } from "./stores/version";

  // acitveUrl is shared between project
  export let activeUrl = "#总览";

  function navigate(route) {
    activeUrl = route;
  }
</script>

<div
  class="w-48 bg-[#f0f0f3]/50 dark:bg-[#2c2c2e]/50 backdrop-blur-xl border-r border-gray-200 dark:border-gray-700"
>
  <nav class="p-3 space-y-1">
    <button
      on:click={() => navigate("#总览")}
      class="flex w-full items-center space-x-2 px-3 py-2 rounded-lg {activeUrl ===
      '#总览'
        ? 'bg-blue-500/10 text-[#0A84FF]'
        : 'text-gray-700'} dark:text-[#0A84FF] hover:bg-[#e5e5e5] dark:hover:bg-[#3a3a3c]"
    >
      <LayoutDashboard
        class="w-5 h-5 {activeUrl === '#总览'
          ? 'text-[#0A84FF]'
          : 'text-gray-700 dark:text-[#0A84FF]'}"
      />
      <span>总览</span>
    </button>
    <button
      on:click={() => navigate("#直播间")}
      class="flex w-full items-center space-x-2 px-3 py-2 rounded-lg {activeUrl ===
      '#直播间'
        ? 'bg-blue-500/10 text-[#0A84FF]'
        : 'text-gray-700'} dark:text-[#0A84FF] hover:bg-[#e5e5e5] dark:hover:bg-[#3a3a3c]"
    >
      <Video
        class="w-5 h-5 {activeUrl === '#直播间'
          ? 'text-[#0A84FF]'
          : 'text-gray-700 dark:text-[#0A84FF]'}"
      />
      <span>直播间</span>
    </button>
    <button
      on:click={() => navigate("#账号")}
      class="flex w-full items-center space-x-2 px-3 py-2 rounded-lg {activeUrl ===
      '#账号'
        ? 'bg-blue-500/10 text-[#0A84FF]'
        : 'text-gray-700'} dark:text-[#0A84FF] hover:bg-[#e5e5e5] dark:hover:bg-[#3a3a3c]"
    >
      <Users
        class="w-5 h-5 {activeUrl === '#账号'
          ? 'text-[#0A84FF]'
          : 'text-gray-700 dark:text-[#0A84FF]'}"
      />
      <span>账号</span>
    </button>
    <button
      on:click={() => navigate("#设置")}
      class="flex w-full items-center space-x-2 px-3 py-2 rounded-lg {activeUrl ===
      '#设置'
        ? 'bg-blue-500/10 text-[#0A84FF]'
        : 'text-gray-700'} dark:text-[#0A84FF] hover:bg-[#e5e5e5] dark:hover:bg-[#3a3a3c]"
    >
      <Settings
        class="w-5 h-5 {activeUrl === '#设置'
          ? 'text-[#0A84FF]'
          : 'text-gray-700 dark:text-[#0A84FF]'}"
      />
      <span>设置</span>
    </button>
    <button
      on:click={() => navigate("#关于")}
      class="flex w-full items-center space-x-2 px-3 py-2 rounded-lg {activeUrl ===
      '#关于'
        ? 'bg-blue-500/10 text-[#0A84FF]'
        : 'text-gray-700'} dark:text-[#0A84FF] hover:bg-[#e5e5e5] dark:hover:bg-[#3a3a3c] relative"
    >
      <Info
        class="w-5 h-5 {activeUrl === '#关于'
          ? 'text-[#0A84FF]'
          : 'text-gray-700 dark:text-[#0A84FF]'}"
      />
      <span>关于</span>
      {#if $hasNewVersion}
        <div
          class="absolute right-3 top-1/2 -translate-y-1/2 w-2 h-2 bg-red-500 rounded-full"
        ></div>
      {/if}
    </button>
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
