<script lang="ts">
  import { User } from "lucide-svelte";
  import type { HumanMessage } from "../agent/messages";
  import AgentAttachmentList from "./AgentAttachmentList.svelte";
  import CopyMarkdownButton from "./CopyMarkdownButton.svelte";

  interface Props {
    message: HumanMessage;
    formatTime: (date: Date) => string;
  }

  let { message, formatTime }: Props = $props();
  let messageTime = $derived(new Date(message.timestamp));
</script>

<div class="flex justify-end">
  <div class="flex max-w-2xl items-start gap-2.5">
    <div class="flex min-w-0 flex-col space-y-1">
      <div class="flex h-6 items-center gap-2">
        <span class="text-[12px] font-medium text-fg-secondary">
          你
        </span>
        <span class="text-[11px] tabular-nums text-fg-muted">
          {formatTime(messageTime)}
        </span>
        {#if message.content}
          <CopyMarkdownButton content={message.content} />
        {/if}
      </div>

      <div
        class="rounded-[10px] border border-accent/25 bg-accent-soft px-3.5 py-2.5"
      >
        {#if message.content}
          <div class="whitespace-pre-wrap break-words text-[13px] leading-relaxed text-fg">
            {message.content}
          </div>
        {/if}
        <AgentAttachmentList attachments={message.attachments} />
      </div>
    </div>

    <div
      class="flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-full border border-line-strong bg-control text-fg-muted"
    >
      <User class="h-3.5 w-3.5" />
    </div>
  </div>
</div>
