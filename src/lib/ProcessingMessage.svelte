<script lang="ts">
  import { Bot } from "lucide-svelte";
</script>

<div class="flex justify-start">
  <div class="flex items-start space-x-3 max-w-2xl">
    <div
      class="w-8 h-8 rounded-full bg-blue-500 flex items-center justify-center flex-shrink-0"
    >
      <Bot class="w-4 h-4 text-white" />
    </div>

    <div class="flex flex-col space-y-1">
      <span class="text-sm font-medium text-gray-700 dark:text-gray-300">
        小轴
      </span>

      <div
        class="bg-white dark:bg-gray-800 rounded-2xl px-4 py-3 shadow-sm border border-gray-200 dark:border-gray-700"
      >
        <div class="flex items-center space-x-2">
          <div
            class="w-4 h-4 border-2 border-blue-500 border-t-transparent rounded-full animate-spin"
          ></div>
          <span class="text-gray-600 dark:text-gray-400 text-sm"
            >正在思考...</span
          >
        </div>
      </div>
    </div>
  </div>
</div>
