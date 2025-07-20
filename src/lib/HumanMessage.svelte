<script lang="ts">
  import { User } from "lucide-svelte";
  import { HumanMessage } from "@langchain/core/messages";

  export let message: HumanMessage;
  export let formatTime: (date: Date) => string;

  // 获取消息时间戳，如果没有则使用当前时间
  $: messageTime = message.additional_kwargs?.timestamp 
    ? new Date(message.additional_kwargs.timestamp)
    : new Date();
</script>

<div class="flex justify-end">
  <div class="flex items-start space-x-3 max-w-2xl">
    <div class="flex flex-col space-y-1">
      <div class="flex items-center space-x-2">
        <span class="text-sm font-medium text-gray-700 dark:text-gray-300">
          你
        </span>
        <span class="text-xs text-gray-500 dark:text-gray-400">
          {formatTime(messageTime)}
        </span>
      </div>

      <div
        class="bg-white dark:bg-gray-800 rounded-2xl px-4 py-3 shadow-sm border border-gray-200 dark:border-gray-700"
      >
        <div class="text-gray-900 dark:text-white text-sm leading-relaxed">
          {message.content}
        </div>
      </div>
    </div>

    <div
      class="w-8 h-8 rounded-full bg-gray-500 flex items-center justify-center flex-shrink-0"
    >
      <User class="w-4 h-4 text-white" />
    </div>
  </div>
</div>
