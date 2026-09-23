<script lang="ts">
  let { class: className = "" }: { class?: string } = $props();
</script>

<svg
  class={className}
  viewBox="0 0 24 24"
  fill="currentColor"
  role="img"
  aria-label="Twitch"
>
  <path d="M4 2 2 6v16h6v4h4l4-4h4l4-4V2H4Zm17 15-3 3h-5l-3 3v-3H5V4h16v13Zm-9-9H9v6h3V8Zm5 0h-3v6h3V8Z" />
</svg>
