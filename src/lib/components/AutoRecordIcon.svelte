<script lang="ts">
  import { Activity, RefreshCw } from "lucide-svelte";
  interface Props {
    size?: number;
    color?: string;
    class?: string;
  }

  let { size = 18, color = "currentColor", class: className = "" }: Props = $props();
</script>

<div class="relative {className}" style="width: {size}px; height: {size}px;">
  <div class="absolute inset-0 flex items-center justify-center">
    <RefreshCw {size} {color} class="opacity-80" strokeWidth={1} />
    <Activity class="absolute" size={size * 0.6} {color} />
  </div>
</div>
