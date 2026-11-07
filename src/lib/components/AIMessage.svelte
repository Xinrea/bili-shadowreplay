<script lang="ts">
  import {
    Bot,
    Check,
    X,
    AlertTriangle,
  } from "lucide-svelte";
  import {
    messageContentToDisplayParts,
    messageContentToMarkdown,
    type AssistantMessage,
    type ToolCall,
  } from "../agent/messages";
  import { marked } from "marked";
  import CopyMarkdownButton from "./CopyMarkdownButton.svelte";

  interface Props {
    message: AssistantMessage;
    formatTime: (date: Date) => string;
    onToolCallConfirm?: (toolCall: ToolCall) => void;
    onToolCallReject?: (toolCall: ToolCall) => void;
    getToolCallState?: (
      toolCallId: string,
    ) => 'confirmed' | 'rejected' | 'none';
    confirmationDisabled?: boolean;
  }

  let {
    message,
    formatTime,
    onToolCallConfirm,
    onToolCallReject,
    getToolCallState = () => 'none',
    confirmationDisabled = false
  }: Props = $props();

  let isError = $derived(message.isError === true);
  let messageTime = $derived(new Date(message.timestamp));
  let contentParts = $derived(messageContentToDisplayParts(message.content));

  function markdownContent(): string {
    return messageContentToMarkdown(message.content);
  }

  function containsTable(content: string): boolean {
    return content.includes('|') || content.includes('---') ||
      content.includes('|--') || content.includes('| -');
  }

  let hasTable = $derived(contentParts.some(
    (part) => part.kind === "text" &&
      part.format === "markdown" && containsTable(part.text),
  ));

  function isExecutedToolCall(toolCall: ToolCall): boolean {
    return toolCall.executed === true;
  }

  function isPendingToolCall(toolCall: ToolCall): boolean {
    return toolCall.executed === false;
  }

  function toolCallFailed(toolCall: ToolCall): boolean {
    return Boolean(toolCall.error);
  }

  function toolCallError(toolCall: ToolCall): string {
    return String(toolCall.error || "未知错误");
  }
</script>

<div class="flex justify-start">
  <div class="flex items-start gap-2.5" class:max-w-2xl={!hasTable} class:max-w-4xl={hasTable}>
    <div
      class="flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-full text-white {isError ? 'bg-danger' : 'bg-accent'}"
    >
      {#if isError}
        <AlertTriangle class="h-3.5 w-3.5" />
      {:else}
        <Bot class="h-3.5 w-3.5" />
      {/if}
    </div>

    <div class="flex min-w-0 flex-col space-y-1">
      <div class="flex h-6 items-center gap-2">
        <span class="text-[12px] font-medium text-fg-secondary">
          小轴
        </span>
        <span class="text-[11px] tabular-nums text-fg-muted">
          {formatTime(messageTime)}
        </span>
        <CopyMarkdownButton content={markdownContent} />
      </div>

      <div
        class="rounded-[10px] border px-3.5 py-2.5 {isError ? 'border-danger/40 bg-danger-soft' : 'border-line bg-surface'}"
      >
        <div
          class="prose prose-sm max-w-none text-[13px] leading-relaxed text-fg [&_.prose]:bg-transparent [&_.prose_*]:bg-transparent [&_p]:bg-transparent [&_div]:bg-transparent [&_span]:bg-transparent [&_code]:rounded-[4px] [&_code]:bg-sunken [&_code]:px-1 [&_code]:py-px [&_code]:text-[12px] [&_code]:text-fg-secondary [&_pre]:overflow-x-auto [&_pre]:rounded-[8px] [&_pre]:p-3 [&_pre]:border [&_pre]:border-line [&_pre]:bg-sunken [&_pre]:text-[12px] [&_pre]:text-fg-secondary [&_pre_code]:bg-transparent [&_pre_code]:p-0 [&_a]:text-accent-fg [&_a:hover]:underline [&_blockquote]:bg-transparent [&_ul]:bg-transparent [&_ol]:bg-transparent [&_li]:bg-transparent [&_h1]:bg-transparent [&_h2]:bg-transparent [&_h3]:bg-transparent [&_h4]:bg-transparent [&_h5]:bg-transparent [&_h6]:bg-transparent [&_p]:m-0 [&_p]:p-0 [&_div]:m-0 [&_div]:p-0 [&_ul]:m-0 [&_ul]:p-0 [&_ol]:m-0 [&_ol]:p-0 [&_li]:m-0 [&_li]:p-0 [&_li]:mb-0 [&_li]:mt-0 [&_h1]:m-0 [&_h1]:p-0 [&_h2]:m-0 [&_h2]:p-0 [&_h3]:m-0 [&_h3]:p-0 [&_h4]:m-0 [&_h4]:p-0 [&_h5]:m-0 [&_h5]:p-0 [&_h6]:m-0 [&_h6]:p-0 [&_blockquote]:m-0 [&_blockquote]:p-0"
        >
          <div class="space-y-3">
            {#each contentParts as part}
              {#if part.kind === "image"}
                <img
                  src={part.src}
                  alt={part.alt}
                  loading="lazy"
                  class="max-h-[28rem] max-w-full rounded-[8px] border border-line object-contain"
                />
              {:else if part.format === "json"}
                <pre class="overflow-x-auto whitespace-pre-wrap break-words rounded-[8px] p-3 text-[12px]">{part.text}</pre>
              {:else}
                <div class:table-container={containsTable(part.text)}>
                  {@html marked(part.text)}
                </div>
              {/if}
            {/each}
          </div>
        </div>

        {#if message.toolCalls.length > 0}
          <div class="mt-3 space-y-2">
            {#each message.toolCalls as toolCall}
              <div
                class="rounded-[8px] border border-line bg-control p-3"
              >
                <div class="mb-2 flex items-center gap-2">
                  <div
                    class="flex h-5 w-5 items-center justify-center rounded-[5px] bg-accent-soft text-accent-fg"
                  >
                    <svg
                      class="h-3 w-3"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24"
                    >
                      <path
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        stroke-width="2"
                        d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
                      ></path>
                      <path
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        stroke-width="2"
                        d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
                      ></path>
                    </svg>
                  </div>
                  <span
                    class="min-w-0 break-all text-[12px] font-medium text-fg"
                  >
                    工具调用: {toolCall.name}
                  </span>
                </div>

                {#if Object.keys(toolCall.args).length > 0}
                  <div class="mb-2">
                    <div
                      class="mb-1 text-[11px] font-medium text-fg-muted"
                    >
                      参数:
                    </div>
                    <div class="rounded-[6px] border border-line bg-sunken p-2">
                      <pre
                        class="whitespace-pre-wrap break-words text-[11px] leading-relaxed text-fg-secondary">{JSON.stringify(
                          toolCall.args,
                          null,
                          2
                        )}</pre>
                    </div>
                  </div>
                {/if}

                {#if toolCall.id}
                  <div class="mb-2 break-all text-[11px] text-fg-muted">
                    ID: {toolCall.id}
                  </div>
                {/if}

                <!-- 工具调用状态或操作按钮 -->
                <div
                  class="mt-3 flex items-center justify-between border-t border-line pt-2.5"
                >
                  {#if isExecutedToolCall(toolCall)}
                    {#if toolCallFailed(toolCall)}
                      <div class="space-y-1 text-danger-fg">
                        <div class="flex items-center gap-1.5">
                          <AlertTriangle class="h-3.5 w-3.5" />
                          <span class="text-[12px] font-medium">调用失败</span>
                        </div>
                        <div class="break-words text-[11px]">{toolCallError(toolCall)}</div>
                      </div>
                    {:else}
                      <div class="flex items-center gap-1.5 text-success-fg">
                        <Check class="h-3.5 w-3.5" />
                        <span class="text-[12px] font-medium">已完成</span>
                      </div>
                    {/if}
                  {:else if getToolCallState(toolCall.id) === 'confirmed'}
                    <!-- 显示状态 -->
                      <div class="flex items-center gap-1.5 text-success-fg">
                        <Check class="h-3.5 w-3.5" />
                        <span class="text-[12px] font-medium">已确认</span>
                      </div>
                  {:else if getToolCallState(toolCall.id) === 'rejected'}
                      <div class="flex items-center gap-1.5 text-danger-fg">
                        <X class="h-3.5 w-3.5" />
                        <span class="text-[12px] font-medium">已拒绝</span>
                      </div>
                  {:else if isPendingToolCall(toolCall)}
                    <div class="flex w-full items-center gap-2">
                      <button
                        onclick={() => onToolCallReject?.(toolCall)}
                        disabled={confirmationDisabled}
                        class="ui-btn ui-btn-sm ui-btn-danger px-3"
                      >
                        <X class="h-3 w-3" />
                        <span>拒绝</span>
                      </button>
                      <button
                        onclick={() => onToolCallConfirm?.(toolCall)}
                        disabled={confirmationDisabled}
                        class="ui-btn ui-btn-sm ui-btn-primary flex-1"
                      >
                        <Check class="h-3 w-3" />
                        <span>确认执行</span>
                      </button>
                    </div>
                  {/if}
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  </div>
</div>
