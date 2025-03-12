<script lang="ts">
  import Room from "./page/Room.svelte";
  import BSidebar from "./lib/BSidebar.svelte";
  import Summary from "./page/Summary.svelte";
  import Setting from "./page/Setting.svelte";
  import Account from "./page/Account.svelte";
  import About from "./page/About.svelte";
  let active = "#总览";
</script>

<main>
  <div class="wrap">
    <div class="sidebar">
      <BSidebar bind:activeUrl={active} />
    </div>
    <div class="content bg-white dark:bg-[#2c2c2e]">
      <!-- switch component by active -->
      <div class="page" class:visible={active == "#总览"}>
        <Summary />
      </div>
      <div class="page" class:visible={active == "#直播间"}>
        <Room />
      </div>
      <div class="page" class:visible={active == "#账号"}>
        <Account />
      </div>
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
