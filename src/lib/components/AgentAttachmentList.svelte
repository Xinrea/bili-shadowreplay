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
            class="rounded-[8px] border border-line object-cover {compact
              ? 'h-14 w-14'
              : 'max-h-48 max-w-[12rem]'}"
          />
          {#if removable}
            <button
              type="button"
              class="absolute -right-1.5 -top-1.5 flex h-5 w-5 items-center justify-center rounded-full border border-surface bg-fg text-surface opacity-90 transition-opacity hover:opacity-100"
              title="移除 {attachment.name}"
              onclick={() => onRemove?.(index)}
            >
              <X class="w-3 h-3" />
            </button>
          {/if}
        </div>
      {:else}
        <div
          class="relative flex max-w-[14rem] items-center gap-2 rounded-[8px] border border-line bg-control px-2.5 py-1.5"
          title={attachment.name}
        >
          <FileText class="h-3.5 w-3.5 flex-shrink-0 text-fg-muted" />
          <span class="truncate text-[12px] text-fg-secondary">
            {attachment.name}
          </span>
          {#if removable}
            <button
              type="button"
              class="flex-shrink-0 text-fg-faint transition-colors hover:text-fg"
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
