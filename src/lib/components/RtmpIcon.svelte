<script lang="ts">
  import { Radio } from "lucide-svelte";
  let { class: className = "" }: { class?: string } = $props();
</script>

<Radio class={className} />
