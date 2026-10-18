<script lang="ts">
  import Room from "./page/Room.svelte";
  import BSidebar, {
    SIDEBAR_RAIL_WIDTH,
    SIDEBAR_WIDTH,
  } from "./lib/components/BSidebar.svelte";
  import SidebarToggle from "./lib/components/SidebarToggle.svelte";
  import { MAC_TRAFFIC_LIGHTS } from "./lib/window-chrome";
  import Summary from "./page/Summary.svelte";
  import Setting from "./page/Setting.svelte";
  import Account from "./page/Account.svelte";
  import About from "./page/About.svelte";
  import { log, onOpenUrl } from "./lib/invoker";
  import Clip from "./page/Clip.svelte";
  import Task from "./page/Task.svelte";
  import AI from "./page/AI.svelte";
  import Archive from "./page/Archive.svelte";
  import TaskCenter from "./lib/components/TaskCenter.svelte";
  import TaskCenterButton from "./lib/components/TaskCenterButton.svelte";
  import TitleBar from "./lib/components/TitleBar.svelte";
  import PageView from "./lib/components/PageView.svelte";
  import type { PageTitlebarContent } from "./lib/page-titlebar";
  import { onMount } from "svelte";

  let active = $state("总览");
  // Filled by each page's `PageTitlebar`, keyed by sidebar label.
  let titlebars: Record<string, PageTitlebarContent | null> = $state({});
  let titlebar = $derived(titlebars[active]);

  let sidebarCollapsed = $state(
    localStorage.getItem("sidebar-collapsed") === "true",
  );
  function toggleSidebar() {
    sidebarCollapsed = !sidebarCollapsed;
    localStorage.setItem("sidebar-collapsed", String(sidebarCollapsed));
  }
  onMount(async () => {
    await onOpenUrl((urls: string[]) => {
      console.log("Received Deep Link:", urls);
      if (urls.length > 0) {
        const url = urls[0];
        // extract platform and room_id from url
        // url example:
        // bsr://live.bilibili.com/167537?live_from=85001&spm_id_from=333.1365.live_users.item.click
        // bsr://live.douyin.com/200525029536

        let platform = "";
        let room_id = "";

        if (url.startsWith("bsr://live.bilibili.com/")) {
          // 1. remove bsr://live.bilibili.com/
          // 2. remove all query params
          room_id = url.replace("bsr://live.bilibili.com/", "").split("?")[0];
          platform = "bilibili";
        }

        if (url.startsWith("bsr://live.douyin.com/")) {
          room_id = url.replace("bsr://live.douyin.com/", "").split("?")[0];
          platform = "douyin";
        }

        if (url.startsWith("bsr://www.douyu.com/") || url.startsWith("bsr://live.douyu.com/")) {
          room_id = url.split(/[?#]/)[0].replace(/\/+$/, "").split("/").pop() || "";
          platform = "douyu";
        }

        if (url.startsWith("bsr://live.kuaishou.com/")) {
          room_id = url.replace("bsr://live.kuaishou.com/", "").split("?")[0];
          room_id = room_id.replace(/^u\//, "");
          platform = "kuaishou";
        }

        if (url.startsWith("bsr://live.tiktok.com/")) {
          room_id = url.replace("bsr://live.tiktok.com/", "").split("?")[0];
          platform = "tiktok";
        }

        if (
          url.startsWith("bsr://www.twitch.tv/") ||
          url.startsWith("bsr://m.twitch.tv/") ||
          url.startsWith("bsr://twitch.tv/")
        ) {
          room_id = url
            .replace(/^bsr:\/\/(?:(?:www|m)\.)?twitch\.tv\//, "")
            .split(/[?/]/)[0];
          platform = "twitch";
        }

        if (
          url.startsWith("bsr://www.youtube.com/") ||
          url.startsWith("bsr://youtube.com/") ||
          url.startsWith("bsr://youtu.be/")
        ) {
          room_id = url.replace("bsr://", "https://");
          platform = "youtube";
        }

        if (platform && room_id) {
          // switch to room page
          active = "直播间";
        }
      }
    });
  });

  log.info("App loaded");

  onMount(() => {
    const onNavSettings = () => (active = "设置");
    window.addEventListener("navigate-settings", onNavSettings);
    return () => window.removeEventListener("navigate-settings", onNavSettings);
  });
</script>

<TaskCenter />

<main class="wrap">
  <BSidebar
    bind:activeUrl={active}
    collapsed={sidebarCollapsed}
    onActiveChange={(label) => {
      active = label;
    }}
    onToggleCollapsed={toggleSidebar}
  />
  <div class="main-column">
    <TitleBar
      title={titlebar?.title ?? active}
      trafficLightOffset={sidebarCollapsed ? SIDEBAR_RAIL_WIDTH : SIDEBAR_WIDTH}
    >
      {#snippet leading()}
        <!-- The collapsed rail cannot fit it after the traffic lights. -->
        {#if MAC_TRAFFIC_LIGHTS && sidebarCollapsed}
          <SidebarToggle collapsed onToggle={toggleSidebar} />
        {/if}
      {/snippet}
      {@render titlebar?.subtitle?.()}
      {#snippet actions()}
        {#if titlebar?.actions}
          {@render titlebar.actions()}
          <span class="mx-1 h-4 w-px bg-line"></span>
        {/if}
        <TaskCenterButton />
      {/snippet}
    </TitleBar>
    <div class="content bg-canvas">
      <PageView visible={active == "总览"} bind:titlebar={titlebars["总览"]}>
        <Summary />
      </PageView>
      <PageView visible={active == "直播间"} bind:titlebar={titlebars["直播间"]}>
        <Room />
      </PageView>
      <PageView visible={active == "录播"} bind:titlebar={titlebars["录播"]}>
        <Archive />
      </PageView>
      <PageView visible={active == "切片"} bind:titlebar={titlebars["切片"]}>
        <Clip />
      </PageView>
      <PageView visible={active == "任务"} bind:titlebar={titlebars["任务"]}>
        <Task />
      </PageView>
      <PageView visible={active == "助手"} bind:titlebar={titlebars["助手"]}>
        <AI onNavigateSettings={() => (active = "设置")} />
      </PageView>
      <PageView visible={active == "账号"} bind:titlebar={titlebars["账号"]}>
        <Account />
      </PageView>
      <PageView visible={active == "设置"} bind:titlebar={titlebars["设置"]}>
        <Setting active={active === "设置"} />
      </PageView>
      <PageView visible={active == "关于"} bind:titlebar={titlebars["关于"]}>
        <About />
      </PageView>
    </div>
  </div>
</main>

<style>
  .wrap {
    display: flex;
    height: 100vh;
    flex-direction: row;
    overflow: hidden;
  }

  .main-column {
    display: flex;
    min-width: 0;
    flex: 1;
    flex-direction: column;
  }

  .content {
    min-height: 0;
    flex: 1;
    overflow: hidden;
  }
</style>
