<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { Dropdown, DropdownItem, Select } from "flowbite-svelte";
  import { ChevronDownOutline } from "flowbite-svelte-icons";
  import type { Children, VideoType } from "./interface";
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
  get_video_typelist();
</script>

<div class="flex">
  <button
    class="z-10 w-2/5 inline-flex justify-between items-center py-2.5 px-4 text-sm font-medium text-center rounded-s-lg focus:border-primary-500 focus:ring-primary-500 bg-gray-700 text-white placeholder-gray-400 border border-gray-600"
    type="button"
  >
    {parentSelected ? parentSelected.name : ""}
    <ChevronDownOutline class="w-6 h-6 ms-2" />
  </button>
  <Dropdown
    bind:open={parentOpen}
    containerClass="divide-y z-50 h-48 overflow-y-auto w-24"
  >
    {#each items as item}
      <DropdownItem
        on:click={() => {
          parentOpen = false;
          areaOpen = false;
          parentSelected = item;
          areaSelected = parentSelected.children[0];
          value = areaSelected.id;
        }}
        class="flex items-center">{item.name}</DropdownItem
      >
    {/each}
  </Dropdown>
  <button
    class="z-10 w-3/5 inline-flex justify-between items-center py-2.5 px-4 text-sm font-medium text-center rounded-e-lg focus:border-primary-500 focus:ring-primary-500 bg-gray-700 text-white placeholder-gray-400 border border-gray-600"
    type="button"
  >
    {areaSelected ? areaSelected.name : ""}
    <ChevronDownOutline class="w-6 h-6 ms-2" />
  </button>
  <Dropdown
    bind:open={areaOpen}
    containerClass="divide-y z-50 h-48 overflow-y-auto min-w-32 bg-gray-700 text-gray-200 rounded-lg border-gray-100 border-gray-600 divide-gray-100 divide-gray-600 shadow-md"
  >
    {#each parentSelected.children as child}
      <DropdownItem
        on:click={() => {
          areaOpen = false;
          parentOpen = false;
          areaSelected = child;
          value = child.id;
        }}
        class="flex items-center">{child.name}</DropdownItem
      >
    {/each}
  </Dropdown>
</div>

<style>
</style>
