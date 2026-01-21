<script lang="ts">
  import Room from "./page/Room.svelte";
  import BSidebar from "./lib/components/BSidebar.svelte";
  import Summary from "./page/Summary.svelte";
  import Setting from "./page/Setting.svelte";
  import Account from "./page/Account.svelte";
  import About from "./page/About.svelte";
  import { log, onOpenUrl } from "./lib/invoker";
  import Clip from "./page/Clip.svelte";
  import Task from "./page/Task.svelte";
  import AI from "./page/AI.svelte";
  import Archive from "./page/Archive.svelte";
  import { onMount } from "svelte";

  let active = "总览";
  let darkMode = false;

  function applyTheme(isDark: boolean) {
    darkMode = isDark;
    document.documentElement.classList.toggle("dark", isDark);
  }

  onMount(async () => {
    const theme = localStorage.getItem("theme");
    applyTheme(theme === "dark");

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

        if (url.startsWith("bsr://live.kuaishou.com/")) {
          room_id = url.replace("bsr://live.kuaishou.com/", "").split("?")[0];
          room_id = room_id.replace(/^u\//, "");
          platform = "kuaishou";
        }

        if (url.startsWith("bsr://live.tiktok.com/")) {
          room_id = url.replace("bsr://live.tiktok.com/", "").split("?")[0];
          platform = "tiktok";
        }

        if (platform && room_id) {
          // switch to room page
          active = "直播间";
        }
      }
    });
  });

  log.info("App loaded");
</script>

<main>
  <div class="wrap">
    <div class="sidebar">
      <BSidebar
        bind:activeUrl={active}
        on:activeChange={(e) => {
          active = e.detail;
        }}
      />
    </div>
    <div class="content bg-white dark:bg-black">
      <div class="page" class:visible={active == "总览"}>
        <Summary />
      </div>
      <div class="page" class:visible={active == "直播间"}>
        <Room />
      </div>
      <div class="page" class:visible={active == "录播"}>
        <Archive />
      </div>
      <div class="page" class:visible={active == "切片"}>
        <Clip />
      </div>
      <div class="page" class:visible={active == "任务"}>
        <Task />
      </div>
      <div class="page" class:visible={active == "助手"}>
        <AI />
      </div>
      <div class="page" class:visible={active == "账号"}>
        <Account />
      </div>
      <div class="page" class:visible={active == "设置"}>
        <Setting />
      </div>
      <div class="page" class:visible={active == "关于"}>
        <About />
      </div>
    </div>
  </div>
</main>

<style>
  .sidebar {
    display: flex;
    height: 100vh;
  }

  .wrap {
    display: flex;
    flex-direction: row;
    height: 100vh;
    overflow: hidden;
    background: #fff;
  }

  :global(.dark) .wrap {
    background: #000;
  }

  .visible {
    opacity: 1 !important;
    height: 100% !important;
    transform: translateX(0) !important;
  }

  .page {
    opacity: 0;
    height: 0;
    transform: translateX(100%);
    overflow: hidden;
    transition:
      opacity 0.5s ease-in-out,
      transform 0.3s ease-in-out;
    display: flex;
    flex-direction: column;
  }

  .content {
    width: 100%;
    height: 100vh;
    overflow: hidden;
  }
</style>
