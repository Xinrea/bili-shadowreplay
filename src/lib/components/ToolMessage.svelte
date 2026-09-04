<script lang="ts">
  import {
    Wrench,
    CheckCircle,
    AlertCircle,
    ChevronDown,
    ChevronRight,
  } from "lucide-svelte";
  import {
    messageContentToDisplayParts,
    messageContentToMarkdown,
    type ToolMessage,
  } from "../agent/messages";
  import CopyMarkdownButton from "./CopyMarkdownButton.svelte";

  interface Props {
    message: ToolMessage;
    formatTime: (date: Date) => string;
  }

  let { message, formatTime }: Props = $props();

  // 折叠状态 - 默认折叠
  let isExpanded = $state(false);

  let messageTime = $derived(new Date(message.timestamp));
  let contentParts = $derived(messageContentToDisplayParts(message.content));

  function markdownContent(): string {
    return messageContentToMarkdown(message.content);
  }

  // 获取状态图标和颜色
  function getStatusInfo() {
    if (message.status === "success") {
      return {
        icon: CheckCircle,
        color: "text-green-500",
        bgColor: "bg-green-50 dark:bg-green-900/20",
        borderColor: "border-green-200 dark:border-green-700",
      };
    } else {
      return {
        icon: AlertCircle,
        color: "text-red-500",
        bgColor: "bg-red-50 dark:bg-red-900/20",
        borderColor: "border-red-200 dark:border-red-700",
      };
    }
  }

  // 格式化工具调用ID
  function formatToolCallId(id: string): string {
    return id.length > 8 ? id.slice(-8) : id;
  }

  // 切换折叠状态
  function toggleExpanded() {
    isExpanded = !isExpanded;
  }

  let statusInfo = $derived(getStatusInfo());
  let StatusIcon = $derived(statusInfo.icon);
</script>

<div class="flex justify-start">
  <div class="flex items-start space-x-3 max-w-2xl">
    <div
      class="w-8 h-8 rounded-full bg-green-500 flex items-center justify-center flex-shrink-0"
    >
      <Wrench class="w-4 h-4 text-white" />
    </div>

    <div class="flex flex-col space-y-1">
      <div class="flex items-center space-x-2">
        <span class="text-sm font-medium text-gray-700 dark:text-gray-300">
          工具响应
        </span>
        <span class="text-xs text-gray-500 dark:text-gray-400">
          {formatTime(messageTime)}
        </span>
        <CopyMarkdownButton content={markdownContent} />
      </div>

      <div
        class="bg-white dark:bg-gray-800 rounded-2xl px-4 py-3 shadow-sm border border-gray-200 dark:border-gray-700"
      >
        <div class="text-gray-900 dark:text-white text-sm leading-relaxed">
          <!-- 工具信息头部 -->
          <div class="mb-3">
            <div class="flex items-center space-x-2 mb-2">
              <StatusIcon
                class="w-4 h-4 {statusInfo.color}"
              />
              <span
                class="text-sm font-medium text-gray-700 dark:text-gray-300"
              >
                {message.name}
              </span>
              <span class="text-xs text-gray-500 dark:text-gray-400">
                (ID: {formatToolCallId(message.toolCallId)})
              </span>
            </div>
          </div>

          <!-- 折叠按钮和内容 -->
          <div class="space-y-2">
            <!-- 折叠按钮 -->
            <button
              onclick={toggleExpanded}
              class="flex items-center space-x-2 text-sm text-gray-600 dark:text-gray-400 hover:text-gray-800 dark:hover:text-gray-200 transition-colors"
            >
              {#if isExpanded}
                <ChevronDown class="w-4 h-4" />
              {:else}
                <ChevronRight class="w-4 h-4" />
              {/if}
              <span>{isExpanded ? "收起详情" : "展开详情"}</span>
            </button>

            <!-- 折叠内容 -->
            {#if isExpanded}
              <div
                class="bg-gray-50 dark:bg-gray-700 rounded-lg p-3 border border-gray-200 dark:border-gray-600"
              >
                <div
                  class="space-y-3 text-sm text-gray-700 dark:text-gray-300 leading-relaxed"
                >
                  {#if contentParts.length === 0}
                    无响应内容
                  {:else}
                    {#each contentParts as part}
                      {#if part.kind === "image"}
                        <img
                          src={part.src}
                          alt={part.alt}
                          loading="lazy"
                          class="max-h-[28rem] max-w-full rounded-lg border border-gray-200 object-contain dark:border-gray-600"
                        />
                      {:else if part.format === "json"}
                        <pre class="overflow-x-auto whitespace-pre-wrap break-words text-xs">{part.text}</pre>
                      {:else}
                        <div class="whitespace-pre-wrap break-words">{part.text}</div>
                      {/if}
                    {/each}
                  {/if}
                </div>
              </div>
            {/if}
          </div>

          <!-- 状态信息 -->
          <div class="mt-2 text-xs text-gray-500 dark:text-gray-400">
            状态: {message.status}
          </div>
        </div>
      </div>
    </div>
  </div>
</div>
