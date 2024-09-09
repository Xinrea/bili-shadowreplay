<script lang="ts">
  import Room from "./lib/Room.svelte";
  import BSidebar from "./lib/BSidebar.svelte";
  import Summary from "./lib/Summary.svelte";
  let active = "#总览";
  let room_count = 0;
</script>

<main class="bg-primary">
  <div class="wrap">
    <div class="sidebar">
      <BSidebar bind:activeUrl={active} {room_count} />
    </div>
    <div class="content ml-72 p-8">
      <!-- switch component by active -->
      <div class:hide={active !== "#总览"}>
        <Summary />
      </div>
      <div class:hide={active !== "#直播间"}>
        <Room bind:room_count={room_count} />
      </div>
      <div class:hide={active !== "#用户"}>
        <div>用户</div>
      </div>
      <div class:hide={active !== "#设置"}>
        <div>设置</div>
      </div>
      <div class:hide={active !== "#关于"}>
        <div>关于</div>
      </div>
    </div>
  </div>
</main>

<style>
  .sidebar {
    display: flex;
    position: fixed;
    top: 0;
    left: 0;
    height: 100vh;
  }

  .wrap {
    display: flex;
    flex-direction: row;
  }

  .hide {
    display: none;
  }
</style>
