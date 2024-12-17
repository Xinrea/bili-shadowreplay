<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  const appWindow = getCurrentWindow();
  export let dark = false;
</script>

<div data-tauri-drag-region class="titlebar z-[500]" class:dark>
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <div
    class="titlebar-button"
    id="titlebar-minimize"
    on:click={() => appWindow.minimize()}
  >
    <img
      src="https://api.iconify.design/mdi:window-minimize.svg"
      alt="minimize"
    />
  </div>
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <div
    class="titlebar-button"
    id="titlebar-maximize"
    on:click={async () => {
      let m = await appWindow.isMaximized();
      if (m) {
        appWindow.unmaximize();
      } else {
        appWindow.maximize();
      }
    }}
  >
    <img
      src="https://api.iconify.design/mdi:window-maximize.svg"
      alt="maximize"
    />
  </div>
  <div class="titlebar-button" id="titlebar-close">
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <img
      src="https://api.iconify.design/mdi:close.svg"
      alt="close"
      on:click={() => appWindow.close()}
    />
  </div>
</div>

<style>
  .titlebar {
    height: 35px;
    user-select: none;
    display: flex;
    justify-content: flex-end;
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    background: linear-gradient(
      to bottom,
      rgba(255, 255, 255, 0.2),
      rgba(255, 255, 255, 0)
    );
  }

  .titlebar-button {
    display: inline-flex;
    justify-content: center;
    align-items: center;
    width: 35px;
    height: 35px;
    user-select: none;
    -webkit-user-select: none;
  }

  .titlebar-button:hover {
    @apply bg-gray-50 bg-opacity-50;
  }

  .dark .titlebar-button:hover {
    @apply bg-gray-300 bg-opacity-50;
  }
</style>
