<script lang="ts">
  import { Settings, Sparkles, Zap } from "lucide-svelte";

  export let onOpenSettings: () => void;
</script>

<div class="flex items-center justify-center min-h-[500px] px-6">
  <div class="max-w-md w-full text-center space-y-8">
    <!-- Icon -->
    <div class="inline-flex items-center justify-center w-16 h-16 rounded-2xl bg-gradient-to-br from-amber-400 to-orange-500 shadow-lg">
      <span class="text-3xl">🍊</span>
    </div>

    <!-- Title and Description -->
    <div class="space-y-3">
      <h2 class="text-2xl font-semibold text-gray-900 dark:text-gray-100">
        欢迎使用 AI 助手
      </h2>
      <p class="text-base text-gray-600 dark:text-gray-400 leading-relaxed max-w-sm mx-auto">
        配置您的 AI 模型以开始智能对话。支持 OpenAI 兼容 API 和本地 Ollama 模型。
      </p>
    </div>

    <!-- CTA Button -->
    <button
      on:click={onOpenSettings}
      class="inline-flex items-center justify-center space-x-2 px-6 py-3 bg-gray-900 dark:bg-gray-100 text-white dark:text-gray-900 rounded-xl hover:bg-gray-800 dark:hover:bg-gray-200 transition-colors font-medium"
    >
      <Settings class="w-4 h-4" />
      <span>开始配置</span>
    </button>

    <!-- Features -->
    <div class="flex items-center justify-center space-x-6 text-sm text-gray-500 dark:text-gray-500 pt-4">
      <div class="flex items-center space-x-1.5">
        <Zap class="w-4 h-4" />
        <span>快速响应</span>
      </div>
      <div class="flex items-center space-x-1.5">
        <Sparkles class="w-4 h-4" />
        <span>智能分析</span>
      </div>
    </div>
  </div>
</div>
