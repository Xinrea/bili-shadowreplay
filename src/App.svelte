<script lang="ts">
  import Room from "./lib/Room.svelte";
  import BSidebar from "./lib/BSidebar.svelte";
  import Summary from "./lib/Summary.svelte";
  import Setting from "./lib/Setting.svelte";
  import Account from "./lib/Account.svelte";
  import TitleBar from "./lib/TitleBar.svelte";
  import Messages from "./lib/Messages.svelte";
  import About from "./lib/About.svelte";
  import { platform } from "@tauri-apps/plugin-os";
  let active = "#总览";
  let room_count = 0;
  let message_cnt = 0;
  let use_titlebar = platform() == "windows";
</script>

<main>
  {#if use_titlebar}
    <TitleBar />
  {/if}
  <div class="wrap">
    <div class="sidebar">
      <BSidebar bind:activeUrl={active} {room_count} {message_cnt} />
    </div>
    <div class="content">
      <!-- switch component by active -->
      <div class="page" class:visible={active == "#总览"}>
        <Summary />
      </div>
      <div class="h-full page" class:visible={active == "#直播间"}>
        <Room bind:room_count />
      </div>
      <div class="h-full page" class:visible={active == "#消息"}>
        <Messages bind:message_cnt />
      </div>
      <div class="h-full page" class:visible={active == "#账号"}>
        <Account />
      </div>
      <!-- <div class="page" class:visible={active == "#自动化"}>
        <div>自动化[开发中]</div>
      </div> -->
      <div class="page" class:visible={active == "#设置"}>
        <Setting />
      </div>
      <div class="page" class:visible={active == "#关于"}>
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
  }

  .visible {
    opacity: 1 !important;
    max-height: fit-content !important;
    transform: translateX(0) !important;
  }

  .page {
    opacity: 0;
    max-height: 0;
    transform: translateX(100%);
    overflow: hidden;
    transition:
      opacity 0.5s ease-in-out,
      transform 0.3s ease-in-out;
  }

  .content {
    height: 100vh;
    background-color: #e5e7eb;
  }
</style>
