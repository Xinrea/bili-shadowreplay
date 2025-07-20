<script lang="ts">
  import {
    Bot,
    Check,
    X,
    AlertTriangle,
  } from "lucide-svelte";
  import { AIMessage } from "@langchain/core/messages";
  import { marked } from "marked";

  export let message: AIMessage;
  export let formatTime: (date: Date) => string;
  export let onToolCallConfirm: ((toolCall: any) => void) | undefined =
    undefined;
  export let onToolCallReject: ((toolCall: any) => void) | undefined =
    undefined;
  export let toolCallState: 'confirmed' | 'rejected' | 'none' = 'none';
  export let isSensitiveToolCall: boolean = false;

  // 检查是否被内容过滤
  $: isContentFiltered = message.response_metadata?.finish_reason === "content_filter";

  // 获取消息时间戳，如果没有则使用当前时间
  $: messageTime = message.additional_kwargs?.timestamp 
    ? new Date(message.additional_kwargs.timestamp as string)
    : new Date();

  // 将 Markdown 转换为 HTML
  $: htmlContent = marked(
    typeof message.content === "string"
      ? message.content
      : Array.isArray(message.content)
        ? message.content
            .map((c) => (typeof c === "string" ? c : JSON.stringify(c)))
            .join("\n")
        : JSON.stringify(message.content)
  );

  // 检查消息是否包含表格
  $: hasTable = message.content && typeof message.content === 'string' && 
    (message.content.includes('|') || message.content.includes('---') || 
     message.content.includes('|--') || message.content.includes('| -'));

  // 处理工具调用确认
  function handleToolCallConfirm(toolCall: any) {
    if (onToolCallConfirm) {
      onToolCallConfirm(toolCall);
    }
  }

  // 处理工具调用拒绝
  function handleToolCallReject(toolCall: any) {
    if (onToolCallReject) {
      onToolCallReject(toolCall);
    }
  }
</script>

<div class="flex justify-start">
  <div class="flex items-start space-x-3" class:max-w-2xl={!hasTable} class:max-w-4xl={hasTable}>
    <div
      class="w-8 h-8 rounded-full bg-blue-500 flex items-center justify-center flex-shrink-0"
    >
      <Bot class="w-4 h-4 text-white" />
    </div>

    <div class="flex flex-col space-y-1">
      <div class="flex items-center space-x-2">
        <span class="text-sm font-medium text-gray-700 dark:text-gray-300">
          小轴
        </span>
        <span class="text-xs text-gray-500 dark:text-gray-400">
          {formatTime(messageTime)}
        </span>
      </div>

      <div
        class="bg-white dark:bg-gray-800 rounded-2xl px-4 py-3 shadow-sm border border-gray-200 dark:border-gray-700"
      >
        <!-- 内容过滤警告 -->
        {#if isContentFiltered}
          <div class="mb-3 p-3 bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-700 rounded-lg">
            <div class="flex items-center space-x-2">
              <AlertTriangle class="w-4 h-4 text-yellow-600 dark:text-yellow-400" />
              <span class="text-sm font-medium text-yellow-800 dark:text-yellow-200">
                内容被过滤
              </span>
            </div>
            <p class="text-xs text-yellow-700 dark:text-yellow-300 mt-1">
              由于内容安全策略，部分回复内容可能已被过滤。请尝试重新表述您的问题。
            </p>
          </div>
        {/if}

        <div
          class="text-gray-900 dark:text-white text-sm leading-relaxed prose prose-sm max-w-none [&_.prose]:bg-transparent [&_.prose_*]:bg-transparent [&_p]:bg-transparent [&_div]:bg-transparent [&_span]:bg-transparent [&_code]:bg-gray-100 dark:bg-gray-700 [&_pre]:bg-gray-100 dark:bg-gray-700 [&_blockquote]:bg-transparent [&_ul]:bg-transparent [&_ol]:bg-transparent [&_li]:bg-transparent [&_h1]:bg-transparent [&_h2]:bg-transparent [&_h3]:bg-transparent [&_h4]:bg-transparent [&_h5]:bg-transparent [&_h6]:bg-transparent [&_p]:m-0 [&_p]:p-0 [&_div]:m-0 [&_div]:p-0 [&_ul]:m-0 [&_ul]:p-0 [&_ol]:m-0 [&_ol]:p-0 [&_li]:m-0 [&_li]:p-0 [&_li]:mb-0 [&_li]:mt-0 [&_h1]:m-0 [&_h1]:p-0 [&_h2]:m-0 [&_h2]:p-0 [&_h3]:m-0 [&_h3]:p-0 [&_h4]:m-0 [&_h4]:p-0 [&_h5]:m-0 [&_h5]:p-0 [&_h6]:m-0 [&_h6]:p-0 [&_blockquote]:m-0 [&_blockquote]:p-0"
        >
          {#if hasTable}
            <div class="table-container">
              {@html htmlContent}
            </div>
          {:else}
            {@html htmlContent}
          {/if}
        </div>
        
        {#if message.tool_calls && message.tool_calls.length > 0}
          <div class="space-y-2 mt-3">
            {#each message.tool_calls as tool_call}
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
                    工具调用: {tool_call.name}
                  </span>
                </div>

                {#if tool_call.args && Object.keys(tool_call.args).length > 0}
                  <div class="mb-2">
                    <div
                      class="text-xs font-medium text-gray-600 dark:text-gray-400 mb-1"
                    >
                      参数:
                    </div>
                    <div class="bg-gray-50 dark:bg-gray-800 rounded p-2">
                      <pre
                        class="text-xs text-gray-700 dark:text-gray-300 whitespace-pre-wrap break-words">{JSON.stringify(
                          tool_call.args,
                          null,
                          2
                        )}</pre>
                    </div>
                  </div>
                {/if}

                {#if tool_call.id}
                  <div class="text-xs text-gray-500 dark:text-gray-400 mb-2">
                    ID: {tool_call.id}
                  </div>
                {/if}

                <!-- 工具调用状态或操作按钮 -->
                {#if isSensitiveToolCall}
                  <div
                    class="flex items-center justify-between mt-3 pt-2 border-t border-blue-200 dark:border-blue-700"
                  >
                    {#if tool_call.id && toolCallState === 'confirmed'}
                      <!-- 显示状态 -->
                        <div class="flex items-center space-x-2 text-green-600 dark:text-green-400">
                          <Check class="w-4 h-4" />
                          <span class="text-sm font-medium">已确认</span>
                        </div>
                    {:else if toolCallState === 'rejected'}
                        <div class="flex items-center space-x-2 text-red-600 dark:text-red-400">
                          <X class="w-4 h-4" />
                          <span class="text-sm font-medium">已拒绝</span>
                        </div>
                    {:else if onToolCallConfirm || onToolCallReject}
                      <!-- 显示操作按钮 -->
                      {#if onToolCallReject}
                        <button
                          on:click={() => handleToolCallReject(tool_call)}
                          class="flex items-center space-x-1 px-4 py-2 bg-red-500 hover:bg-red-600 active:bg-red-700 text-white text-xs font-medium rounded-lg shadow-sm transition-all duration-200 focus:outline-none focus:ring-2 focus:ring-red-500 focus:ring-offset-2 dark:focus:ring-offset-gray-800"
                        >
                          <X class="w-3 h-3" />
                          <span>拒绝</span>
                        </button>
                      {/if}
                      {#if onToolCallConfirm}
                        <button
                          on:click={() => handleToolCallConfirm(tool_call)}
                          class="flex items-center space-x-1 px-4 py-2 bg-blue-500 hover:bg-blue-600 active:bg-blue-700 text-white text-xs font-medium rounded-lg shadow-sm transition-all duration-200 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 dark:focus:ring-offset-gray-800"
                        >
                          <Check class="w-3 h-3" />
                          <span>确认</span>
                        </button>
                      {/if}
                    {/if}
                  </div>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  </div>
</div>
