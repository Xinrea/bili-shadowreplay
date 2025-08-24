<script>
  import { Info, LayoutDashboard, Settings, Users, Video } from "lucide-svelte";
  import { createEventDispatcher } from "svelte";

  const dispatch = createEventDispatcher();

  // acitveUrl is shared between project
  export let activeUrl = "总览";
  export let label = "";
  export let dot = false;
</script>

<button
  on:click={() => dispatch("activeChange", label)}
  class="flex w-full items-center space-x-2 px-3 py-2 rounded-lg {activeUrl ===
  label
    ? 'bg-blue-500/10 text-[#0A84FF]'
    : 'text-gray-700'} dark:text-[#0A84FF] hover:bg-[#e5e5e5] dark:hover:bg-[#3a3a3c]"
>
  <slot
    name="icon"
    class={activeUrl === label
      ? "text-[#0A84FF]"
      : "text-gray-700 dark:text-[#0A84FF]"}
  ></slot>
  <span>{label}</span>
  {#if dot}
    <div class="absolute right-6 w-2 h-2 bg-red-500 rounded-full"></div>
  {/if}
</button>
