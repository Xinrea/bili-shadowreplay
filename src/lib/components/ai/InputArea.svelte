<script lang="ts">
  import { Send, Trash2, Settings } from "lucide-svelte";

  export let inputMessage: string;
  export let isProcessing: boolean;
  export let agent: any;
  export let onSend: () => void;
  export let onClear: () => void;
  export let onSettings: () => void;
  export let onKeyPress: (e: KeyboardEvent) => void;
</script>

<div class="border-t border-gray-200 dark:border-gray-800 bg-white dark:bg-gray-900 p-4">
  <div class="max-w-4xl mx-auto space-y-3">
    <!-- Input Row -->
    <div class="flex items-end space-x-2">
      <div class="flex-1 relative">
        <textarea
          bind:value={inputMessage}
          on:keypress={onKeyPress}
          placeholder={!agent ? "请先配置 AI 模型..." : "输入您的消息..."}
          class="w-full px-4 py-3 pr-16 border border-gray-300 dark:border-gray-700 rounded-xl bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100 placeholder-gray-400 dark:placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-gray-900 dark:focus:ring-gray-100 focus:border-transparent resize-none min-h-[52px] max-h-[200px] text-[15px] leading-relaxed disabled:opacity-50 disabled:cursor-not-allowed transition-shadow"
          rows="1"
          disabled={isProcessing || !agent}
        ></textarea>

        {#if inputMessage.trim()}
          <div class="absolute right-3 bottom-3 text-xs text-gray-400 dark:text-gray-600">
            {inputMessage.length}
          </div>
        {/if}
      </div>

      <button
        class="px-4 py-3 bg-gray-900 dark:bg-gray-100 text-white dark:text-gray-900 rounded-xl hover:bg-gray-800 dark:hover:bg-gray-200 disabled:opacity-50 disabled:cursor-not-allowed transition-colors flex items-center space-x-2"
        disabled={!inputMessage.trim() || isProcessing || !agent}
        on:click={onSend}
      >
        <Send class="w-4 h-4" />
        <span class="text-sm font-medium">发送</span>
      </button>
    </div>

    <!-- Action Buttons -->
    <div class="flex items-center justify-between">
      <button
        class="px-3 py-1.5 text-sm text-gray-600 dark:text-gray-400 hover:text-gray-900 dark:hover:text-gray-200 hover:bg-gray-100 dark:hover:bg-gray-800 rounded-lg disabled:opacity-50 disabled:cursor-not-allowed transition-colors flex items-center space-x-1.5"
        on:click={onClear}
        disabled={!agent}
      >
        <Trash2 class="w-3.5 h-3.5" />
        <span>清空对话</span>
      </button>

      <button
        class="p-1.5 text-gray-600 dark:text-gray-400 hover:text-gray-900 dark:hover:text-gray-200 hover:bg-gray-100 dark:hover:bg-gray-800 rounded-lg transition-colors"
        on:click={onSettings}
        title="设置"
      >
        <Settings class="w-4 h-4" />
      </button>
    </div>
  </div>
</div>
