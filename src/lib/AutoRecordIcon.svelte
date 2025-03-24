<script lang="ts">
  import { Activity, RefreshCw } from "lucide-svelte";
  export let size: number = 18;
  export let color: string = "currentColor";
</script>

<div class="relative" class:$$props style="width: {size}px; height: {size}px;">
  <div class="absolute inset-0 flex items-center justify-center">
    <RefreshCw {size} {color} class="opacity-80" strokeWidth={1} />
    <Activity class="absolute" size={size * 0.6} {color} />
  </div>
</div>
