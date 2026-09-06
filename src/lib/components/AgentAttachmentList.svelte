<script lang="ts">
  import { FileText, X } from "lucide-svelte";
  import {
    imageAttachmentSrc,
    type MessageAttachment,
  } from "../agent/messages";

  interface Props {
    attachments: MessageAttachment[];
    removable?: boolean;
    compact?: boolean;
    onRemove?: (index: number) => void;
  }

  let {
    attachments,
    removable = false,
    compact = false,
    onRemove,
  }: Props = $props();
</script>

{#if attachments.length > 0}
  <div class="flex flex-wrap gap-2 {compact ? '' : 'mt-2'}">
    {#each attachments as attachment, index (index)}
      {#if attachment.kind === "image"}
        <div class="relative group">
          <img
            src={imageAttachmentSrc(attachment)}
            alt={attachment.name}
            title={attachment.name}
            class="rounded-lg border border-gray-200 dark:border-gray-700 object-cover {compact
              ? 'h-14 w-14'
              : 'max-h-48 max-w-[12rem]'}"
          />
          {#if removable}
            <button
              type="button"
              class="absolute -top-1.5 -right-1.5 w-5 h-5 rounded-full bg-gray-900 text-white dark:bg-gray-100 dark:text-gray-900 flex items-center justify-center opacity-90 hover:opacity-100"
              title="移除 {attachment.name}"
              onclick={() => onRemove?.(index)}
            >
              <X class="w-3 h-3" />
            </button>
          {/if}
        </div>
      {:else}
        <div
          class="relative flex items-center gap-2 max-w-[14rem] px-2.5 py-1.5 rounded-lg border border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/80"
          title={attachment.name}
        >
          <FileText class="w-3.5 h-3.5 text-gray-500 dark:text-gray-400 flex-shrink-0" />
          <span class="text-xs text-gray-700 dark:text-gray-200 truncate">
            {attachment.name}
          </span>
          {#if removable}
            <button
              type="button"
              class="flex-shrink-0 text-gray-400 hover:text-gray-700 dark:hover:text-gray-200"
              title="移除 {attachment.name}"
              onclick={() => onRemove?.(index)}
            >
              <X class="w-3.5 h-3.5" />
            </button>
          {/if}
        </div>
      {/if}
    {/each}
  </div>
{/if}
