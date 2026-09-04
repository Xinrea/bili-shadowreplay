<script lang="ts">
  let { class: className = "" }: { class?: string } = $props();
</script>

<img src="/imgs/huya.png" class={className} alt="huya" />
