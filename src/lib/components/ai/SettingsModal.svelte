<script lang="ts">
  import { X } from "lucide-svelte";

  export let showSettings: boolean;
  export let settings: {
    provider: "openai" | "ollama";
    endpoint: string;
    api_key: string;
    model: string;
  };
  export let availableModels: Array<{ value: string; label: string }>;
  export let isLoadingModels: boolean;
  export let onClose: () => void;
  export let onSave: () => void;
  export let onLoadModels: () => void;
</script>

{#if showSettings}
  <div class="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
    <div class="bg-white dark:bg-gray-900 rounded-2xl shadow-2xl w-full max-w-lg border border-gray-200 dark:border-gray-800">
      <!-- Header -->
      <div class="px-6 py-4 border-b border-gray-200 dark:border-gray-800">
        <div class="flex items-center justify-between">
          <h3 class="text-lg font-semibold text-gray-900 dark:text-gray-100">AI 模型设置</h3>
          <button
            class="p-1.5 text-gray-400 hover:text-gray-600 dark:hover:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-800 rounded-lg transition-colors"
            on:click={onClose}
          >
            <X class="w-4 h-4" />
          </button>
        </div>
      </div>

      <!-- Body -->
      <div class="p-6 space-y-5 max-h-[calc(100vh-200px)] overflow-y-auto">
        <!-- Provider Selection -->
        <div>
          <label class="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-3">
            选择 AI 提供商
          </label>
          <div class="grid grid-cols-2 gap-3">
            <button
              type="button"
              class="relative p-4 rounded-xl border-2 transition-all {settings.provider === 'openai'
                ? 'border-gray-900 dark:border-gray-100 bg-gray-50 dark:bg-gray-800'
                : 'border-gray-200 dark:border-gray-700 hover:border-gray-300 dark:hover:border-gray-600'}"
              on:click={() => settings.provider = 'openai'}
            >
              {#if settings.provider === 'openai'}
                <div class="absolute top-3 right-3 w-2 h-2 bg-gray-900 dark:bg-gray-100 rounded-full"></div>
              {/if}
              <div class="text-center space-y-2">
                <div class="text-2xl">🌐</div>
                <div class="text-sm font-medium text-gray-900 dark:text-gray-100">OpenAI API</div>
                <div class="text-xs text-gray-500 dark:text-gray-500">兼容 OpenAI 的 API</div>
              </div>
            </button>

            <button
              type="button"
              class="relative p-4 rounded-xl border-2 transition-all {settings.provider === 'ollama'
                ? 'border-gray-900 dark:border-gray-100 bg-gray-50 dark:bg-gray-800'
                : 'border-gray-200 dark:border-gray-700 hover:border-gray-300 dark:hover:border-gray-600'}"
              on:click={() => settings.provider = 'ollama'}
            >
              {#if settings.provider === 'ollama'}
                <div class="absolute top-3 right-3 w-2 h-2 bg-gray-900 dark:bg-gray-100 rounded-full"></div>
              {/if}
              <div class="text-center space-y-2">
                <div class="text-2xl">🦙</div>
                <div class="text-sm font-medium text-gray-900 dark:text-gray-100">Ollama</div>
                <div class="text-xs text-gray-500 dark:text-gray-500">本地运行的模型</div>
              </div>
            </button>
          </div>
        </div>

        <!-- Endpoint -->
        <div>
          <label for="endpoint" class="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
            {settings.provider === 'ollama' ? 'Ollama 服务地址' : 'API Endpoint'}
          </label>
          <input
            id="endpoint"
            type="text"
            bind:value={settings.endpoint}
            placeholder={settings.provider === 'ollama' ? 'http://localhost:11434' : 'https://api.openai.com/v1'}
            class="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100 placeholder-gray-400 dark:placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-gray-900 dark:focus:ring-gray-100 focus:border-transparent text-sm"
          />
          <p class="mt-1.5 text-xs text-gray-500 dark:text-gray-500">
            {settings.provider === 'ollama'
              ? '默认为 http://localhost:11434'
              : '输入兼容 OpenAI 的 API 端点地址'}
          </p>
        </div>

        <!-- API Key (only for OpenAI) -->
        {#if settings.provider === 'openai'}
          <div>
            <label for="api_key" class="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
              API Key
            </label>
            <input
              id="api_key"
              type="password"
              bind:value={settings.api_key}
              placeholder="sk-..."
              class="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100 placeholder-gray-400 dark:placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-gray-900 dark:focus:ring-gray-100 focus:border-transparent text-sm"
            />
            <p class="mt-1.5 text-xs text-gray-500 dark:text-gray-500">
              您的 API 密钥将安全地存储在本地
            </p>
          </div>
        {/if}

        <!-- Model Selection -->
        <div>
          <div class="flex items-center justify-between mb-2">
            <label for="model" class="block text-sm font-medium text-gray-700 dark:text-gray-300">
              模型名称
            </label>
            {#if settings.provider === 'openai'}
              <button
                type="button"
                class="text-xs text-gray-600 dark:text-gray-400 hover:text-gray-900 dark:hover:text-gray-200 disabled:opacity-50 disabled:cursor-not-allowed"
                on:click={onLoadModels}
                disabled={!settings.endpoint || !settings.api_key || isLoadingModels}
              >
                {isLoadingModels ? '加载中...' : '刷新列表'}
              </button>
            {/if}
          </div>
          <input
            id="model"
            type="text"
            bind:value={settings.model}
            list="model-options"
            placeholder={settings.provider === 'ollama' ? 'llama2, mistral, qwen...' : 'gpt-4, gpt-3.5-turbo...'}
            class="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100 placeholder-gray-400 dark:placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-gray-900 dark:focus:ring-gray-100 focus:border-transparent text-sm"
          />
          <datalist id="model-options">
            {#each availableModels as model}
              <option value={model.value}>{model.label}</option>
            {/each}
          </datalist>
          <p class="mt-1.5 text-xs text-gray-500 dark:text-gray-500">
            {settings.provider === 'ollama'
              ? '输入已安装的 Ollama 模型名称'
              : '输入模型名称或从列表中选择'}
          </p>
        </div>
      </div>

      <!-- Footer -->
      <div class="px-6 py-4 border-t border-gray-200 dark:border-gray-800 flex justify-end space-x-3">
        <button
          class="px-4 py-2 text-sm text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-800 rounded-lg transition-colors"
          on:click={onClose}
        >
          取消
        </button>
        <button
          class="px-4 py-2 text-sm bg-gray-900 dark:bg-gray-100 text-white dark:text-gray-900 rounded-lg hover:bg-gray-800 dark:hover:bg-gray-200 transition-colors font-medium"
          on:click={onSave}
        >
          保存设置
        </button>
      </div>
    </div>
  </div>
{/if}
