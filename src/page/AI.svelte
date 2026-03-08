<script lang="ts">
  import { onMount } from "svelte";
  import { Settings, Send, Sparkles, Trash2, Zap, MessageSquare, Bot } from "lucide-svelte";
  import createAgent, { type AgentConfig } from "../lib/agent/agent";
  import { tools } from "../lib/agent/tools";
  import {
    HumanMessage,
    AIMessage,
    ToolMessage,
  } from "@langchain/core/messages";
  import HumanMessageComponent from "../lib/components/HumanMessage.svelte";
  import AIMessageComponent from "../lib/components/AIMessage.svelte";
  import ProcessingMessageComponent from "../lib/components/ProcessingMessage.svelte";
  import ToolMessageComponent from "../lib/components/ToolMessage.svelte";
  import SettingsModal from "../lib/components/ai/SettingsModal.svelte";

  let messages: any[] = [];
  let inputMessage = "";
  let isProcessing = false;
  let messageContainer: HTMLElement;
  let agent = null;

  // 设置相关状态
  let showSettings = false;
  let isLoadingModels = false;
  let settings = {
    provider: "openai" as "openai" | "ollama",
    endpoint: "",
    api_key: "",
    model: ""
  };

  let availableModels = [];
  const toolCallStates = new Map<string, 'confirmed' | 'rejected' | 'none'>();

  // 预设提示词
  const presetPrompts = [
    { title: "录制直播", description: "添加新的直播间并开始录制", prompt: "我该如何添加新的直播间？", icon: "📡" },
    { title: "查看任务", description: "显示所有录制任务状态", prompt: "显示我所有的录制任务", icon: "📊" },
    { title: "管理账户", description: "查看已添加的账户信息", prompt: "显示可用的账号信息", icon: "👤" },
    { title: "生成切片", description: "分析录播并生成精彩片段", prompt: "分析最新录制的录播有哪些精彩部分，选择一段生成切片", icon: "✂️" },
    { title: "视频转码", description: "转换视频格式", prompt: "帮我将视频转码为mp4格式", icon: "🎬" },
    { title: "提取音频", description: "从视频中提取音频", prompt: "帮我提取视频中的音频", icon: "🎵" }
  ];

  function openSettings() { showSettings = true; }
  function closeSettings() { showSettings = false; }

  async function saveSettings() {
    localStorage.setItem('ai_settings', JSON.stringify(settings));
    if (settings.provider === 'ollama') {
      if (settings.endpoint || settings.model) {
        agent = createAgent({
          provider: 'ollama',
          baseURL: settings.endpoint || 'http://localhost:11434',
          model: settings.model || 'llama2',
        });
      } else {
        agent = null;
      }
    } else {
      if (settings.api_key && settings.endpoint) {
        agent = createAgent({
          provider: 'openai',
          apiKey: settings.api_key,
          baseURL: settings.endpoint,
          model: settings.model || undefined,
        });
        await loadModels();
      } else {
        agent = null;
      }
    }
    closeSettings();
  }

  async function fetchModels(endpoint: string, apiKey: string) {
    try {
      const response = await fetch(`${endpoint}/models`, {
        headers: { 'Authorization': `Bearer ${apiKey}`, 'Content-Type': 'application/json' }
      });
      if (!response.ok) throw new Error(`HTTP error! status: ${response.status}`);
      const data = await response.json();
      if (data.data && Array.isArray(data.data)) {
        return data.data.map((model: any) => ({ value: model.id, label: model.id }));
      }
      return [];
    } catch (error) {
      console.error('Failed to fetch models:', error);
      return [];
    }
  }

  async function loadModels() {
    if (settings.endpoint && settings.api_key) {
      isLoadingModels = true;
      try {
        const models = await fetchModels(settings.endpoint, settings.api_key);
        if (models.length > 0) availableModels = models;
      } catch (error) {
        console.error('Failed to load models:', error);
      } finally {
        isLoadingModels = false;
      }
    }
  }

  function loadSettings() {
    const savedSettings = localStorage.getItem('ai_settings');
    if (savedSettings) {
      settings = { ...settings, ...JSON.parse(savedSettings) };
      if (settings.provider === 'ollama') {
        if (settings.endpoint || settings.model) {
          agent = createAgent({
            provider: 'ollama',
            baseURL: settings.endpoint || 'http://localhost:11434',
            model: settings.model || 'llama2',
          });
        }
      } else {
        if (settings.api_key && settings.endpoint) {
          agent = createAgent({
            provider: 'openai',
            apiKey: settings.api_key,
            baseURL: settings.endpoint,
            model: settings.model || undefined,
          });
          loadModels();
        }
      }
    }
  }

  function getToolCallState(message: any): 'confirmed' | 'rejected' | 'none' {
    if (message.tool_calls && message.tool_calls.length > 0) {
      return toolCallStates.get(message.tool_calls[0]?.id) || 'none';
    }
    return 'none';
  }

  function scrollToBottom() {
    if (messageContainer) {
      setTimeout(() => { messageContainer.scrollTop = messageContainer.scrollHeight; }, 100);
    }
  }

  function formatTime(timestamp: string): string {
    const date = new Date(timestamp);
    return date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' });
  }

  function isSensitiveToolCall(message: any): boolean {
    if (!message.tool_calls || message.tool_calls.length === 0) return false;
    const sensitiveTools = ['delete_recorder', 'delete_archive', 'delete_video'];
    return message.tool_calls.some((toolCall: any) => sensitiveTools.includes(toolCall.name));
  }

  async function handlePresetPrompt(prompt: string) {
    inputMessage = prompt;
    await sendMessage();
  }

  async function sendMessage() {
    if (!inputMessage.trim() || isProcessing || !agent) return;
    const userMessage = new HumanMessage({ content: inputMessage });
    userMessage.additional_kwargs = { ...userMessage.additional_kwargs, timestamp: new Date().toISOString() };
    messages = [...messages, userMessage];
    inputMessage = "";
    isProcessing = true;
    scrollToBottom();
    try {
      await continueAgentFlow([userMessage]);
    } finally {
      isProcessing = false;
      scrollToBottom();
    }
  }

  // Define tool whitelist - tools that auto-execute without user confirmation
  const autoExecuteTools = new Set([
    'get_accounts',
    'get_recorder_list',
    'get_recorder_info',
    'get_archives',
    'get_archive',
    'get_background_tasks',
    'get_videos',
    'get_all_videos',
    'get_video',
    'get_video_cover',
    'get_video_typelist',
    'get_video_subtitle',
    'get_danmu_record',
    'get_recent_record',
    'get_recent_record_all',
    'get_archive_subtitle',
    'get_video_metadata',
    'get_archive_metadata',
  ]);

  async function continueAgentFlow(newMessages: any[]) {
    const config = { configurable: { thread_id: "1" } };

    try {
      const stream = await agent.stream({ messages: newMessages }, config);

      let lastAIMessage = null;
      for await (const chunk of stream) {
        if (chunk.agent) {
          const agentMessages = chunk.agent.messages;
          for (const msg of agentMessages) {
            if (msg instanceof AIMessage) {
              msg.additional_kwargs = { ...msg.additional_kwargs, timestamp: new Date().toISOString() };
              messages = [...messages, msg];
              lastAIMessage = msg;
              scrollToBottom();
            }
          }
        }
      }

    localStorage.setItem('messages', JSON.stringify(messages));
    const toolCallStatesObj = Object.fromEntries(toolCallStates);
    localStorage.setItem('toolCallStates', JSON.stringify(toolCallStatesObj));

    // After stream ends, check if we need to auto-execute whitelisted tools
    if (lastAIMessage?.tool_calls && lastAIMessage.tool_calls.length > 0) {
      const autoExecutableTools = lastAIMessage.tool_calls.filter(
        (toolCall: any) => autoExecuteTools.has(toolCall.name)
      );

      // If all tool calls are whitelisted, auto-execute them
      if (autoExecutableTools.length === lastAIMessage.tool_calls.length) {
        const toolResults: any[] = [];

        for (const toolCall of autoExecutableTools) {
          // Mark as confirmed
          toolCallStates.set(toolCall.id, 'confirmed');

          // Execute tool
          const tool = tools.find(t => t.name === toolCall.name);
          if (!tool) {
            console.error(`Tool ${toolCall.name} not found`);
            const errorDetails = {
              error: true,
              message: `Tool ${toolCall.name} not found`,
              tool: toolCall.name,
            };
            const errorMessage = new ToolMessage({
              name: toolCall.name,
              content: JSON.stringify(errorDetails),
              tool_call_id: toolCall.id || `tool_${Date.now()}`,
            });
            errorMessage.additional_kwargs = { ...errorMessage.additional_kwargs, timestamp: new Date().toISOString() };
            toolResults.push(errorMessage);
            continue;
          }

          try {
            const result = await tool.invoke(toolCall.args);
            const resultMessage = new ToolMessage({
              name: toolCall.name,
              content: typeof result === 'string' ? result : JSON.stringify(result),
              tool_call_id: toolCall.id || `tool_${Date.now()}`,
            });
            resultMessage.additional_kwargs = { ...resultMessage.additional_kwargs, timestamp: new Date().toISOString() };
            toolResults.push(resultMessage);
          } catch (error) {
            console.error(`Error executing tool ${toolCall.name}:`, error);
            const errorDetails = {
              error: true,
              message: error.message || String(error),
              tool: toolCall.name,
              args: toolCall.args,
            };
            const errorMessage = new ToolMessage({
              name: toolCall.name,
              content: JSON.stringify(errorDetails),
              tool_call_id: toolCall.id || `tool_${Date.now()}`,
            });
            errorMessage.additional_kwargs = { ...errorMessage.additional_kwargs, timestamp: new Date().toISOString() };
            toolResults.push(errorMessage);
          }
        }

        // Add all tool results to messages
        messages = [...messages, ...toolResults];
        scrollToBottom();

        // Continue agent flow with all tool results
        await continueAgentFlow(toolResults);
      }
    }
    } catch (error) {
      console.error('LLM API Error:', error);

      // Create error message to display to user
      const errorMessage = new AIMessage({
        content: `❌ **LLM API 错误**\n\n${error.message || String(error)}\n\n请检查：\n- API 端点是否正确\n- API 密钥是否有效\n- 网络连接是否正常\n- 模型名称是否正确`,
      });
      errorMessage.additional_kwargs = {
        ...errorMessage.additional_kwargs,
        timestamp: new Date().toISOString(),
        isError: true
      };
      messages = [...messages, errorMessage];
      scrollToBottom();
    }
  }

  async function handleToolCallConfirm(toolCall: any) {
    isProcessing = true;
    try {
      // 立即更新状态为 confirmed
      toolCallStates.set(toolCall.id, 'confirmed');
      messages = [...messages]; // 触发响应式更新
      scrollToBottom();

      const tool = tools.find(t => t.name === toolCall.name);
      if (!tool) {
        throw new Error(`Tool ${toolCall.name} not found`);
      }

      let resultMessage: ToolMessage;
      try {
        const result = await tool.invoke(toolCall.args);
        resultMessage = new ToolMessage({
          name: toolCall.name,
          content: typeof result === 'string' ? result : JSON.stringify(result),
          tool_call_id: toolCall.id || `tool_${Date.now()}`,
        });
      } catch (error) {
        // Wrap error as tool result instead of throwing
        console.error(`Error executing tool ${toolCall.name}:`, error);
        const errorDetails = {
          error: true,
          message: error.message || String(error),
          tool: toolCall.name,
          args: toolCall.args,
        };
        resultMessage = new ToolMessage({
          name: toolCall.name,
          content: JSON.stringify(errorDetails),
          tool_call_id: toolCall.id || `tool_${Date.now()}`,
        });
      }

      resultMessage.additional_kwargs = { ...resultMessage.additional_kwargs, timestamp: new Date().toISOString() };

      // 添加工具结果消息到对话中
      messages = [...messages, resultMessage];
      scrollToBottom();

      await continueAgentFlow([resultMessage]);
    } finally {
      isProcessing = false;
      scrollToBottom();
    }
  }

  async function handleToolCallReject(toolCall: any) {
    // 立即更新状态为 rejected
    toolCallStates.set(toolCall.id, 'rejected');
    messages = [...messages]; // 触发响应式更新
    scrollToBottom();

    const resultMessage = new ToolMessage({
      name: toolCall.name,
      content: "用户选择拒绝执行工具",
      tool_call_id: toolCall.id || `tool_${Date.now()}`,
    });
    resultMessage.additional_kwargs = { ...resultMessage.additional_kwargs, timestamp: new Date().toISOString() };

    // 添加拒绝消息到对话中
    messages = [...messages, resultMessage];
    scrollToBottom();

    await continueAgentFlow([resultMessage]);
    scrollToBottom();
  }

  async function clearConversation() {
    messages = [];
    toolCallStates.clear();
    localStorage.removeItem('messages');
    localStorage.removeItem('toolCallStates');
    if (settings.provider === 'ollama') {
      if (settings.endpoint || settings.model) {
        agent = createAgent({
          provider: 'ollama',
          baseURL: settings.endpoint || 'http://localhost:11434',
          model: settings.model || 'llama2',
        });
      }
    } else {
      if (settings.api_key && settings.endpoint) {
        agent = createAgent({
          provider: 'openai',
          apiKey: settings.api_key,
          baseURL: settings.endpoint,
          model: settings.model || undefined,
        });
      }
    }
    scrollToBottom();
  }

  function handleKeyPress(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  }

  onMount(async () => {
    loadSettings();
    const previousMessages = JSON.parse(localStorage.getItem('messages') || '[]');
    messages = previousMessages.map((message: any) => {
      if (message.id.includes('HumanMessage')) {
        const msg = new HumanMessage(message.kwargs);
        if (message.additional_kwargs?.timestamp) {
          msg.additional_kwargs = { ...msg.additional_kwargs, timestamp: message.additional_kwargs.timestamp };
        }
        return msg;
      } else if (message.id.includes('AIMessage')) {
        const msg = new AIMessage(message.kwargs);
        if (message.additional_kwargs?.timestamp) {
          msg.additional_kwargs = { ...msg.additional_kwargs, timestamp: message.additional_kwargs.timestamp };
        }
        return msg;
      } else if (message.id.includes('ToolMessage')) {
        const msg = new ToolMessage(message.kwargs);
        if (message.additional_kwargs?.timestamp) {
          msg.additional_kwargs = { ...msg.additional_kwargs, timestamp: message.additional_kwargs.timestamp };
        }
        return msg;
      }
    });
    toolCallStates.clear();
    const toolCallStatesString = localStorage.getItem('toolCallStates');
    if (toolCallStatesString) {
      const toolCallStatesObj = JSON.parse(toolCallStatesString);
      for (const [key, value] of Object.entries(toolCallStatesObj)) {
        toolCallStates.set(key, value as 'confirmed' | 'rejected' | 'none');
      }
    }
    scrollToBottom();
  });
</script>

<div class="flex h-full bg-gradient-to-br from-gray-50 to-gray-100 dark:from-gray-950 dark:to-gray-900">
  <!-- Main Content -->
  <div class="flex-1 flex flex-col relative">
    <!-- Messages Area -->
    <div class="flex-1 overflow-y-auto" bind:this={messageContainer}>
      <div class="max-w-4xl mx-auto px-6 py-8 pb-52">
        {#if !agent}
          <!-- Welcome State -->
          <div class="flex items-center justify-center min-h-[500px]">
            <div class="max-w-md text-center space-y-6">
              <div class="inline-flex items-center justify-center w-20 h-20 rounded-3xl bg-gradient-to-br from-amber-400 to-orange-500 shadow-xl">
                <span class="text-4xl">🍊</span>
              </div>
              <div class="space-y-3">
                <h2 class="text-2xl font-semibold text-gray-900 dark:text-gray-100">欢迎使用 AI 助手</h2>
                <p class="text-gray-600 dark:text-gray-400 leading-relaxed">
                  配置您的 AI 模型以开始智能对话。支持 OpenAI 兼容 API 和本地 Ollama 模型。
                </p>
              </div>
              <button
                on:click={openSettings}
                class="inline-flex items-center space-x-2 px-6 py-3 bg-gray-900 dark:bg-gray-100 text-white dark:text-gray-900 rounded-xl hover:bg-gray-800 dark:hover:bg-gray-200 transition-colors font-medium shadow-lg"
              >
                <Settings class="w-4 h-4" />
                <span>开始配置</span>
              </button>
              <div class="flex items-center justify-center space-x-6 text-sm text-gray-500 pt-4">
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
        {:else if messages.length === 0}
          <!-- Empty State with Prompts -->
          <div class="space-y-8">
            <div class="text-center space-y-3">
              <div class="inline-flex items-center justify-center w-14 h-14 rounded-2xl bg-gradient-to-br from-amber-400 to-orange-500">
                <span class="text-2xl">🍊</span>
              </div>
              <div>
                <h2 class="text-xl font-semibold text-gray-900 dark:text-gray-100">你好！我是小轴</h2>
                <p class="text-sm text-gray-600 dark:text-gray-400 mt-2">
                  我可以帮你管理直播录制、生成精彩切片、分析弹幕内容等。点击下方卡片快速开始。
                </p>
              </div>
            </div>
            <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
              {#each presetPrompts as prompt}
                <button
                  on:click={() => handlePresetPrompt(prompt.prompt)}
                  class="group p-4 text-left bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-xl hover:border-gray-300 dark:hover:border-gray-600 hover:shadow-md transition-all"
                >
                  <div class="flex items-start space-x-3">
                    <span class="text-2xl flex-shrink-0">{prompt.icon}</span>
                    <div class="flex-1 min-w-0">
                      <div class="text-sm font-medium text-gray-900 dark:text-gray-100 mb-1">{prompt.title}</div>
                      <p class="text-xs text-gray-500 dark:text-gray-500 leading-relaxed">{prompt.description}</p>
                    </div>
                  </div>
                </button>
              {/each}
            </div>
          </div>
        {:else}
          <!-- Messages -->
          <div class="space-y-6">
            {#each messages as message, index (index)}
              {#if message instanceof HumanMessage}
                <HumanMessageComponent {message} {formatTime} />
              {:else if message instanceof AIMessage}
                <AIMessageComponent
                  {message}
                  {formatTime}
                  onToolCallConfirm={handleToolCallConfirm}
                  onToolCallReject={handleToolCallReject}
                  toolCallState={getToolCallState(message)}
                  isSensitiveToolCall={isSensitiveToolCall(message)}
                />
              {:else if message instanceof ToolMessage}
                <ToolMessageComponent {message} {formatTime} />
              {/if}
            {/each}
          </div>
        {/if}

        {#if isProcessing}
          <div class="mt-6">
            <ProcessingMessageComponent />
          </div>
        {/if}
      </div>
    </div>

    <!-- Floating Input Area -->
    <div class="absolute bottom-0 left-0 right-0 px-6 pb-4 pt-8 bg-gradient-to-t from-gray-50 via-gray-50 to-transparent dark:from-gray-950 dark:via-gray-950">
      <div class="max-w-4xl mx-auto">
        <div class="bg-white dark:bg-gray-900 rounded-2xl border border-gray-200 dark:border-gray-800 shadow-lg overflow-hidden">
          <!-- Textarea -->
          <div class="relative">
            <textarea
              bind:value={inputMessage}
              on:keypress={handleKeyPress}
              placeholder={!agent ? "请先配置 AI 模型..." : "输入您的消息..."}
              class="w-full px-4 pt-3 pb-3 bg-transparent text-gray-900 dark:text-gray-100 placeholder-gray-400 dark:placeholder-gray-500 focus:outline-none resize-none min-h-[52px] max-h-[200px] text-[15px] leading-relaxed disabled:opacity-50 disabled:cursor-not-allowed"
              rows="1"
              disabled={isProcessing || !agent}
            ></textarea>
          </div>

          <!-- Bottom bar: model info + actions -->
          <div class="flex items-center justify-between px-4 py-2 border-t border-gray-100 dark:border-gray-800/50">
            <div class="flex items-center space-x-3">
              <!-- Model info -->
              <button
                on:click={openSettings}
                class="flex items-center space-x-1.5 px-2 py-1 text-xs text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-gray-200 hover:bg-gray-100 dark:hover:bg-gray-800 rounded-lg transition-colors"
                title="点击配置模型"
              >
                <Bot class="w-3.5 h-3.5" />
                {#if agent}
                  <span>{settings.provider === 'ollama' ? 'Ollama' : 'OpenAI'} · {settings.model || '未设置模型'}</span>
                {:else}
                  <span>未配置模型</span>
                {/if}
              </button>

              <button
                class="flex items-center space-x-1 px-2 py-1 text-xs text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-gray-200 hover:bg-gray-100 dark:hover:bg-gray-800 rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                on:click={clearConversation}
                disabled={!agent}
                title="清空对话"
              >
                <Trash2 class="w-3.5 h-3.5" />
                <span>清空</span>
              </button>
            </div>

            <div class="flex items-center space-x-2">
              {#if inputMessage.trim()}
                <span class="text-xs text-gray-400 dark:text-gray-600">{inputMessage.length}</span>
              {/if}
              <button
                class="px-3 py-1.5 bg-gray-900 dark:bg-gray-100 text-white dark:text-gray-900 rounded-lg hover:bg-gray-800 dark:hover:bg-gray-200 disabled:opacity-50 disabled:cursor-not-allowed transition-colors flex items-center space-x-1.5 text-sm font-medium"
                disabled={!inputMessage.trim() || isProcessing || !agent}
                on:click={sendMessage}
              >
                <Send class="w-3.5 h-3.5" />
                <span>发送</span>
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>

  <!-- Settings Modal -->
  <SettingsModal
    {showSettings}
    bind:settings
    {availableModels}
    {isLoadingModels}
    onClose={closeSettings}
    onSave={saveSettings}
    onLoadModels={loadModels}
  />
</div>

<style>
  :global(.overflow-y-auto) {
    scrollbar-width: thin;
    scrollbar-color: rgb(209 213 219) transparent;
  }
  :global(.dark .overflow-y-auto) {
    scrollbar-color: rgb(55 65 81) transparent;
  }
  :global(.overflow-y-auto::-webkit-scrollbar) {
    width: 6px;
  }
  :global(.overflow-y-auto::-webkit-scrollbar-track) {
    background: transparent;
  }
  :global(.overflow-y-auto::-webkit-scrollbar-thumb) {
    background: rgb(209 213 219);
    border-radius: 3px;
  }
  :global(.dark .overflow-y-auto::-webkit-scrollbar-thumb) {
    background: rgb(55 65 81);
  }
</style>
