<script lang="ts">
  import Room from "./lib/Room.svelte";
  import BSidebar from "./lib/BSidebar.svelte";
  import Summary from "./lib/Summary.svelte";
  import Setting from "./lib/Setting.svelte";
  import Account from "./lib/Account.svelte";
  import TitleBar from "./lib/TitleBar.svelte";
  const urlParams = new URLSearchParams(window.location.search);
  let active = "#总览";
  let room_count = 0;
</script>

<main>
  <TitleBar />
  <div class="wrap">
    <div class="sidebar">
      <BSidebar bind:activeUrl={active} {room_count} />
    </div>
    <div class="content">
      <!-- switch component by active -->
      <div class="page" class:visible={active == "#总览"}>
        <Summary />
      </div>
      <div class="h-full page" class:visible={active == "#直播间"}>
        <Room bind:room_count />
      </div>
      <div class="page" class:visible={active == "#账号"}>
        <Account />
      </div>
      <div class="page" class:visible={active == "#自动化"}>
        <div>自动化[开发中]</div>
      </div>
      <div class="page" class:visible={active == "#设置"}>
        <Setting />
      </div>
      <div class="page" class:visible={active == "#关于"}>
        <div>关于</div>
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
    width: 100vw;
  }
</style>
