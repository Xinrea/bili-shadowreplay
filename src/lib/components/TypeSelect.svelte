<script lang="ts">
  import { invoke } from "../invoker";
  import type { VideoType } from "../interface";

  interface Props {
    value?: number;
    disabled?: boolean;
  }

  let { value = $bindable(0), disabled = false }: Props = $props();
  let items: VideoType[] = $state([]);
  let error = $state("");

  let parent = $derived(items.find((item) => item.children.some((child) => child.id === value)));

  async function load() {
    try {
      items = await invoke<VideoType[]>("get_video_typelist");
      // Bilibili requires an area, so an unknown one falls back to the first.
      if (!parent && items[0]?.children[0]) value = items[0].children[0].id;
    } catch (e) {
      console.error("Failed to load video types:", e);
      error = "无法读取分区列表，请先在「账号」页登录 B 站账号。";
    }
  }

  function selectParent(id: number) {
    const next = items.find((item) => item.id === id);
    if (next?.children[0]) value = next.children[0].id;
  }

  load();
</script>

<div class="flex w-full gap-2">
  <select
    class="ui-input min-w-0 flex-1"
    aria-label="主分区"
    value={parent?.id}
    disabled={disabled || items.length === 0}
    onchange={(event) => selectParent(Number(event.currentTarget.value))}
  >
    {#each items as item (item.id)}
      <option value={item.id}>{item.name}</option>
    {/each}
  </select>
  <select
    class="ui-input min-w-0 flex-1"
    aria-label="子分区"
    bind:value
    disabled={disabled || !parent}
  >
    {#each parent?.children ?? [] as child (child.id)}
      <option value={child.id}>{child.name}</option>
    {/each}
  </select>
</div>
{#if error}
  <p class="ui-hint mt-1 text-danger-fg">{error}</p>
{/if}
