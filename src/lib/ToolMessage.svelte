<script lang="ts">
  import {
    Wrench,
    CheckCircle,
    AlertCircle,
    ChevronDown,
    ChevronRight,
  } from "lucide-svelte";
  import { ToolMessage } from "@langchain/core/messages";

  export let message: ToolMessage;
  export let formatTime: (date: Date) => string;

  // 折叠状态
  let isExpanded = false;

  // 获取消息时间戳，如果没有则使用当前时间
  $: messageTime = message.additional_kwargs?.timestamp 
    ? new Date(message.additional_kwargs.timestamp)
    : new Date();

  // 获取状态图标和颜色
  function getStatusInfo() {
    if (message.status === "success" || !message.status) {
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
  function formatToolCallId(id: string | undefined): string {
    if (!id) return "";
    return id.length > 8 ? id.slice(-8) : id;
  }

  // 切换折叠状态
  function toggleExpanded() {
    isExpanded = !isExpanded;
  }

  $: statusInfo = getStatusInfo();
  $: StatusIcon = statusInfo.icon;
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
      </div>

      <div
        class="bg-white dark:bg-gray-800 rounded-2xl px-4 py-3 shadow-sm border border-gray-200 dark:border-gray-700"
      >
        <div class="text-gray-900 dark:text-white text-sm leading-relaxed">
          <!-- 工具信息头部 -->
          <div class="mb-3">
            <div class="flex items-center space-x-2 mb-2">
              <svelte:component
                this={StatusIcon}
                class="w-4 h-4 {statusInfo.color}"
              />
              <span
                class="text-sm font-medium text-gray-700 dark:text-gray-300"
              >
                {message.name || "未知工具"}
              </span>
              {#if message.tool_call_id}
                <span class="text-xs text-gray-500 dark:text-gray-400">
                  (ID: {formatToolCallId(message.tool_call_id)})
                </span>
              {/if}
            </div>
          </div>

          <!-- 折叠按钮和内容 -->
          <div class="space-y-2">
            <!-- 折叠按钮 -->
            <button
              on:click={toggleExpanded}
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
                  class="text-sm text-gray-700 dark:text-gray-300 leading-relaxed"
                >
                  {message.content || "无响应内容"}
                </div>
              </div>
            {/if}
          </div>

          <!-- 状态信息 -->
          {#if message.status}
            <div class="mt-2 text-xs text-gray-500 dark:text-gray-400">
              状态: {message.status}
            </div>
          {/if}
        </div>
      </div>
    </div>
  </div>
</div>
