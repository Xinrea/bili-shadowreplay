<script lang="ts">
  import { invoke } from "../invoker";
  import { ChevronDownOutline } from "flowbite-svelte-icons";
  import { scale } from "svelte/transition";
  import type { Children, VideoType } from "../interface";
  export let value = 0;
  let parentSelected: VideoType;
  let areaSelected: Children;
  let parentOpen = false;
  let areaOpen = false;
  let items: VideoType[] = [];

  async function get_video_typelist() {
    items = (await invoke("get_video_typelist")) as VideoType[];
    // find parentSelected by value
    let valid = false;
    for (let i = 0; i < items.length; i++) {
      for (let j = 0; j < items[i].children.length; j++) {
        if (items[i].children[j].id === value) {
          parentSelected = items[i];
          areaSelected = items[i].children[j];
          valid = true;
          break;
        }
      }
    }
    if (!valid) {
      parentSelected = items[0];
      areaSelected = items[0].children[0];
      value = areaSelected.id;
    }
  }

  function handleParentClick() {
    parentOpen = !parentOpen;
    areaOpen = false;
  }

  function handleAreaClick() {
    areaOpen = !areaOpen;
    parentOpen = false;
  }

  function selectParent(item: VideoType) {
    parentSelected = item;
    areaSelected = parentSelected.children[0];
    value = areaSelected.id;
    parentOpen = false;
  }

  function selectArea(child: Children) {
    areaSelected = child;
    value = child.id;
    areaOpen = false;
  }

  // Close dropdowns when clicking outside
  function handleClickOutside(event: MouseEvent) {
    const target = event.target as HTMLElement;
    if (!target.closest(".type-select-container")) {
      parentOpen = false;
      areaOpen = false;
    }
  }

  get_video_typelist();
</script>

<svelte:window on:click={handleClickOutside} />

<div class="type-select-container flex w-full max-w-md">
  <!-- Parent Select -->
  <div class="relative flex-1">
    <button
      class="w-full inline-flex justify-between items-center px-3 py-2 text-sm font-medium text-left bg-[#1c1c1e] text-white border border-gray-600 rounded-l-lg hover:bg-[#2c2c2e] focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent transition-colors duration-200"
      type="button"
      on:click={handleParentClick}
    >
      <span class="truncate">{parentSelected ? parentSelected.name : ""}</span>
      <ChevronDownOutline
        class="w-4 h-4 ml-2 flex-shrink-0 transition-transform duration-200 {parentOpen
          ? 'rotate-180'
          : ''}"
      />
    </button>

    {#if parentOpen}
      <div
        class="absolute top-full left-0 mt-1 w-full bg-[#1c1c1e] border border-gray-600 rounded-lg shadow-lg z-50 max-h-48 overflow-y-auto"
        transition:scale={{ duration: 150, start: 0.95 }}
      >
        {#each items as item}
          <button
            class="w-full px-3 py-2 text-sm text-left text-white hover:bg-[#2c2c2e] first:rounded-t-lg last:rounded-b-lg transition-colors duration-150 {parentSelected?.id ===
            item.id
              ? 'bg-blue-900/20 text-blue-400'
              : ''}"
            on:click={() => selectParent(item)}
          >
            {item.name}
          </button>
        {/each}
      </div>
    {/if}
  </div>

  <!-- Area Select -->
  <div class="relative flex-1">
    <button
      class="w-full inline-flex justify-between items-center px-3 py-2 text-sm font-medium text-left bg-[#1c1c1e] text-white border border-l-0 border-gray-600 rounded-r-lg hover:bg-[#2c2c2e] focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent transition-colors duration-200"
      type="button"
      on:click={handleAreaClick}
    >
      <span class="truncate">{areaSelected ? areaSelected.name : ""}</span>
      <ChevronDownOutline
        class="w-4 h-4 ml-2 flex-shrink-0 transition-transform duration-200 {areaOpen
          ? 'rotate-180'
          : ''}"
      />
    </button>

    {#if areaOpen}
      <div
        class="absolute top-full right-0 mt-1 w-full bg-[#1c1c1e] border border-gray-600 rounded-lg shadow-lg z-50 max-h-48 overflow-y-auto"
        transition:scale={{ duration: 150, start: 0.95 }}
      >
        {#each parentSelected?.children || [] as child}
          <button
            class="w-full px-3 py-2 text-sm text-left text-white hover:bg-[#2c2c2e] first:rounded-t-lg last:rounded-b-lg transition-colors duration-150 {areaSelected?.id ===
            child.id
              ? 'bg-blue-900/20 text-blue-400'
              : ''}"
            on:click={() => selectArea(child)}
          >
            {child.name}
          </button>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  /* Custom scrollbar for dropdowns */
  :global(
      .type-select-container div[class*="overflow-y-auto"]::-webkit-scrollbar
    ) {
    width: 6px;
  }

  :global(
      .type-select-container
        div[class*="overflow-y-auto"]::-webkit-scrollbar-track
    ) {
    background: transparent;
  }

  :global(
      .type-select-container
        div[class*="overflow-y-auto"]::-webkit-scrollbar-thumb
    ) {
    background: rgba(75, 85, 99, 0.5);
    border-radius: 3px;
  }

  :global(
      .type-select-container
        div[class*="overflow-y-auto"]::-webkit-scrollbar-thumb:hover
    ) {
    background: rgba(75, 85, 99, 0.7);
  }
</style>
