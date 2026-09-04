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
  <div class="flex items-start space-x-3" class:max-w-2xl={!hasTable} class:max-w-4xl={hasTable}>
    <div
      class="w-8 h-8 rounded-full flex items-center justify-center flex-shrink-0"
      class:bg-blue-500={!isError}
      class:bg-red-500={isError}
    >
      {#if isError}
        <AlertTriangle class="w-4 h-4 text-white" />
      {:else}
        <Bot class="w-4 h-4 text-white" />
      {/if}
    </div>

    <div class="flex flex-col space-y-1">
      <div class="flex items-center space-x-2">
        <span class="text-sm font-medium text-gray-700 dark:text-gray-300">
          小轴
        </span>
        <span class="text-xs text-gray-500 dark:text-gray-400">
          {formatTime(messageTime)}
        </span>
        <CopyMarkdownButton content={markdownContent} />
      </div>

      <div
        class="rounded-2xl px-4 py-3 shadow-sm border"
        class:bg-white={!isError}
        class:dark:bg-gray-800={!isError}
        class:border-gray-200={!isError}
        class:dark:border-gray-700={!isError}
        class:bg-red-50={isError}
        class:border-red-300={isError}
        class:dark:border-red-700={isError}
        class:dark:bg-red-900={isError}
      >
        <div
          class="text-gray-900 dark:text-white text-sm leading-relaxed prose prose-sm max-w-none [&_.prose]:bg-transparent [&_.prose_*]:bg-transparent [&_p]:bg-transparent [&_div]:bg-transparent [&_span]:bg-transparent [&_code]:bg-gray-100 dark:bg-gray-700 [&_pre]:bg-gray-100 dark:bg-gray-700 [&_blockquote]:bg-transparent [&_ul]:bg-transparent [&_ol]:bg-transparent [&_li]:bg-transparent [&_h1]:bg-transparent [&_h2]:bg-transparent [&_h3]:bg-transparent [&_h4]:bg-transparent [&_h5]:bg-transparent [&_h6]:bg-transparent [&_p]:m-0 [&_p]:p-0 [&_div]:m-0 [&_div]:p-0 [&_ul]:m-0 [&_ul]:p-0 [&_ol]:m-0 [&_ol]:p-0 [&_li]:m-0 [&_li]:p-0 [&_li]:mb-0 [&_li]:mt-0 [&_h1]:m-0 [&_h1]:p-0 [&_h2]:m-0 [&_h2]:p-0 [&_h3]:m-0 [&_h3]:p-0 [&_h4]:m-0 [&_h4]:p-0 [&_h5]:m-0 [&_h5]:p-0 [&_h6]:m-0 [&_h6]:p-0 [&_blockquote]:m-0 [&_blockquote]:p-0"
        >
          <div class="space-y-3">
            {#each contentParts as part}
              {#if part.kind === "image"}
                <img
                  src={part.src}
                  alt={part.alt}
                  loading="lazy"
                  class="max-h-[28rem] max-w-full rounded-lg border border-gray-200 object-contain dark:border-gray-600"
                />
              {:else if part.format === "json"}
                <pre class="overflow-x-auto whitespace-pre-wrap break-words rounded-lg p-3 text-xs">{part.text}</pre>
              {:else}
                <div class:table-container={containsTable(part.text)}>
                  {@html marked(part.text)}
                </div>
              {/if}
            {/each}
          </div>
        </div>

        {#if message.toolCalls.length > 0}
          <div class="space-y-2 mt-3">
            {#each message.toolCalls as toolCall}
              <div
                class="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-700 rounded-lg p-3"
              >
                <div class="flex items-center space-x-2 mb-2">
                  <div
                    class="w-5 h-5 rounded bg-blue-500 flex items-center justify-center"
                  >
                    <svg
                      class="w-3 h-3 text-white"
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
                    class="text-sm font-medium text-blue-700 dark:text-blue-300"
                  >
                    工具调用: {toolCall.name}
                  </span>
                </div>

                {#if Object.keys(toolCall.args).length > 0}
                  <div class="mb-2">
                    <div
                      class="text-xs font-medium text-gray-600 dark:text-gray-400 mb-1"
                    >
                      参数:
                    </div>
                    <div class="bg-gray-50 dark:bg-gray-800 rounded p-2">
                      <pre
                        class="text-xs text-gray-700 dark:text-gray-300 whitespace-pre-wrap break-words">{JSON.stringify(
                          toolCall.args,
                          null,
                          2
                        )}</pre>
                    </div>
                  </div>
                {/if}

                {#if toolCall.id}
                  <div class="text-xs text-gray-500 dark:text-gray-400 mb-2">
                    ID: {toolCall.id}
                  </div>
                {/if}

                <!-- 工具调用状态或操作按钮 -->
                <div
                  class="flex items-center justify-between mt-3 pt-2 border-t border-blue-200 dark:border-blue-700"
                >
                  {#if isExecutedToolCall(toolCall)}
                    {#if toolCallFailed(toolCall)}
                      <div class="space-y-1 text-red-600 dark:text-red-400">
                        <div class="flex items-center space-x-2">
                          <AlertTriangle class="w-4 h-4" />
                          <span class="text-sm font-medium">调用失败</span>
                        </div>
                        <div class="text-xs break-words">{toolCallError(toolCall)}</div>
                      </div>
                    {:else}
                      <div class="flex items-center space-x-2 text-green-600 dark:text-green-400">
                        <Check class="w-4 h-4" />
                        <span class="text-sm font-medium">已完成</span>
                      </div>
                    {/if}
                  {:else if getToolCallState(toolCall.id) === 'confirmed'}
                    <!-- 显示状态 -->
                      <div class="flex items-center space-x-2 text-green-600 dark:text-green-400">
                        <Check class="w-4 h-4" />
                        <span class="text-sm font-medium">已确认</span>
                      </div>
                  {:else if getToolCallState(toolCall.id) === 'rejected'}
                      <div class="flex items-center space-x-2 text-red-600 dark:text-red-400">
                        <X class="w-4 h-4" />
                        <span class="text-sm font-medium">已拒绝</span>
                      </div>
                  {:else if isPendingToolCall(toolCall)}
                    <div class="flex items-center space-x-2 w-full">
                      <button
                        onclick={() => onToolCallReject?.(toolCall)}
                        disabled={confirmationDisabled}
                        class="flex items-center space-x-1 px-4 py-2 bg-red-500 hover:bg-red-600 active:bg-red-700 text-white text-xs font-medium rounded-lg shadow-sm transition-all duration-200 focus:outline-none focus:ring-2 focus:ring-red-500 focus:ring-offset-2 dark:focus:ring-offset-gray-800 disabled:opacity-50 disabled:cursor-not-allowed"
                      >
                        <X class="w-3 h-3" />
                        <span>拒绝</span>
                      </button>
                      <button
                        onclick={() => onToolCallConfirm?.(toolCall)}
                        disabled={confirmationDisabled}
                        class="flex items-center justify-center space-x-1 px-4 py-2 bg-blue-500 hover:bg-blue-600 active:bg-blue-700 text-white text-xs font-medium rounded-lg shadow-sm transition-all duration-200 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 dark:focus:ring-offset-gray-800 disabled:opacity-50 disabled:cursor-not-allowed flex-1"
                      >
                        <Check class="w-3 h-3" />
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
