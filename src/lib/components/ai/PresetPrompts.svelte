<script lang="ts">
  export let presetPrompts: Array<{
    title: string;
    description: string;
    prompt: string;
    icon: string;
  }>;
  export let onSelectPrompt: (prompt: string) => void;
</script>

<div class="py-12 space-y-8">
  <!-- Welcome Header -->
  <div class="text-center space-y-3">
    <div class="inline-flex items-center justify-center w-14 h-14 rounded-2xl bg-gradient-to-br from-amber-400 to-orange-500 shadow-sm">
      <span class="text-2xl">🍊</span>
    </div>
    <div class="space-y-2">
      <h2 class="text-xl font-semibold text-gray-900 dark:text-gray-100">
        你好！我是小轴
      </h2>
      <p class="text-sm text-gray-600 dark:text-gray-400 max-w-xl mx-auto leading-relaxed">
        我可以帮你管理直播录制、生成精彩切片、分析弹幕内容等。点击下方卡片快速开始，或直接输入你的需求。
      </p>
    </div>
  </div>

  <!-- Preset Prompts Grid -->
  <div class="max-w-4xl mx-auto grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
    {#each presetPrompts as prompt}
      <button
        on:click={() => onSelectPrompt(prompt.prompt)}
        class="group p-4 text-left bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-xl hover:border-gray-300 dark:hover:border-gray-600 hover:shadow-sm transition-all"
      >
        <div class="flex items-start space-x-3">
          <div class="flex-shrink-0 text-xl mt-0.5">
            {prompt.icon}
          </div>
          <div class="flex-1 min-w-0 space-y-1">
            <div class="text-sm font-medium text-gray-900 dark:text-gray-100 group-hover:text-gray-700 dark:group-hover:text-gray-300">
              {prompt.title}
            </div>
            <p class="text-xs text-gray-500 dark:text-gray-500 leading-relaxed">
              {prompt.description}
            </p>
          </div>
        </div>
      </button>
    {/each}
  </div>
</div>
