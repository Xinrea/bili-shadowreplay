<script lang="ts">
  let { class: className = "" }: { class?: string } = $props();
</script>

<img src="/imgs/douyu.svg" class={"object-contain " + className} alt="douyu" />
