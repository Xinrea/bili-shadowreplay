<script lang="ts">
  import { onDestroy } from "svelte";
  import { Check, Copy } from "lucide-svelte";

  interface Props {
    content: string | (() => string);
  }

  let { content }: Props = $props();

  let copied = $state(false);
  let resetTimer: ReturnType<typeof setTimeout> | undefined;

  async function copyMarkdown() {
    try {
      const markdown = typeof content === "function" ? content() : content;
      await navigator.clipboard.writeText(markdown);
      copied = true;
      if (resetTimer) clearTimeout(resetTimer);
      resetTimer = setTimeout(() => {
        copied = false;
      }, 1500);
    } catch (error) {
      console.error("Failed to copy message:", error);
    }
  }

  onDestroy(() => {
    if (resetTimer) clearTimeout(resetTimer);
  });
</script>

<button
  type="button"
  onclick={copyMarkdown}
  class="inline-flex h-6 w-6 items-center justify-center rounded text-gray-400 transition-colors hover:bg-gray-100 hover:text-gray-600 focus:outline-none focus:ring-2 focus:ring-blue-500 dark:hover:bg-gray-700 dark:hover:text-gray-200"
  aria-label={copied ? "已复制 Markdown" : "复制为 Markdown"}
  title={copied ? "已复制" : "复制为 Markdown"}
>
  {#if copied}
    <Check class="h-3.5 w-3.5 text-green-500" />
  {:else}
    <Copy class="h-3.5 w-3.5" />
  {/if}
</button>
