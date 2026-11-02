<script lang="ts">
  import { Image as ImageIcon } from "lucide-svelte";

  let { src, alt = "", class: className = "", loading = "lazy" }: {
    src?: string | null; alt?: string; class?: string; loading?: "lazy" | "eager";
  } = $props();
  let failedSource = $state<string | null>(null);
</script>

<span class={`archive-cover ${className}`}>
  {#if src && failedSource !== src}
    <img {src} {alt} {loading} decoding="async" draggable="false" onerror={(event) => failedSource = event.currentTarget.getAttribute("src")} />
  {:else}
    <span role="img" aria-label={alt || "暂无封面"} class="flex h-full w-full items-center justify-center text-fg-faint"><ImageIcon class="h-5 w-5 max-h-[50%] max-w-[50%]" aria-hidden="true" /></span>
  {/if}
</span>

<style>
  .archive-cover { position: relative; display: block; overflow: hidden; background: rgb(var(--c-control)); }
  img { position: absolute; inset: 0; height: 100%; width: 100%; object-fit: cover; }
</style>
