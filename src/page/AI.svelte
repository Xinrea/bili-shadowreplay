<script lang="ts">
  import { Send, Sparkles, Settings } from "lucide-svelte";
  import { onMount } from "svelte";
  import createAgent from "../lib/agent/agent";
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

  let messages: any[] = [];
  let inputMessage = "";
  let isProcessing = false;
  let messageContainer: HTMLElement;
  let agent = null;
  let firstMessage = true;

  // 设置相关状态
  let showSettings = false;
  let isLoadingModels = false;
  let settings = {
    endpoint: "",
    api_key: "",
    model: ""
  };

  // 可用的模型列表
  let availableModels = [];

  const toolCallStates = new Map<string, 'confirmed' | 'rejected' | 'none'>();

  // 预设提示词
  const presetPrompts = [
    {
      title: "帮助我录制直播",
      description: "指导我如何设置和开始录制B站或抖音直播",
      prompt: "我该如何添加新的直播间？"
    },
    {
      title: "查看录制任务",
      description: "显示当前所有的录制任务和状态",
      prompt: "显示我所有的录制任务"
    },
    {
      title: "管理账户",
      description: "查看和管理已添加的B站和抖音账户",
      prompt: "显示可用的账号信息"
    },
    {
      title: "切片生成",
      description: "根据录播弹幕分析，生成切片",
      prompt: "分析最新录制的录播有哪些精彩部分，选择一段生成切片"
    },
    {
      title: "视频转码",
      description: "将视频转码为指定格式",
      prompt: "帮我将视频转码为mp4格式"
    },
    {
      title: "音频提取",
      description: "提取视频中的音频",
      prompt: "帮我提取视频中的音频"
    }
  ];

  // 设置相关函数
  function openSettings() {
    showSettings = true;
  }

  function closeSettings() {
    showSettings = false;
  }

  async function saveSettings() {
    localStorage.setItem('ai_settings', JSON.stringify(settings));
    // 只有当有必要的设置时才创建agent
    if (settings.api_key && settings.endpoint) {
      agent = createAgent({
        apiKey: settings.api_key || undefined,
        baseURL: settings.endpoint || undefined,
        model: settings.model || undefined,
      });
      // 重新加载模型列表
      await loadModels();
    } else {
      agent = null;
    }
    closeSettings();
  }

  async function fetchModels(endpoint: string, apiKey: string) {
    try {
      const response = await fetch(`${endpoint}/models`, {
        headers: {
          'Authorization': `Bearer ${apiKey}`,
          'Content-Type': 'application/json'
        }
      });
      
      if (!response.ok) {
        throw new Error(`HTTP error! status: ${response.status}`);
      }
      
      const data = await response.json();
      if (data.data && Array.isArray(data.data)) {
        return data.data.map((model: any) => ({
          value: model.id,
          label: model.id
        }));
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
        if (models.length > 0) {
          availableModels = models;
        }
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
      // 只有当有必要的设置时才创建agent
      if (settings.api_key && settings.endpoint) {
        agent = createAgent({
          apiKey: settings.api_key || undefined,
          baseURL: settings.endpoint || undefined,
          model: settings.model || undefined,
        });
        // 加载模型列表
        loadModels();
      } else {
        agent = null;
      }
    } else {
      agent = null;
    }
  }

  function getToolCallState(message: any): 'confirmed' | 'rejected' | 'none' {
    if (message.tool_calls && message.tool_calls.length > 0) {
      return toolCallStates.get(message.tool_calls[0]?.id) || 'none';
    }
    return 'none';
  }

  // 自动滚动到底部
  function scrollToBottom() {
    if (messageContainer) {
      setTimeout(() => {
        messageContainer.scrollTop = messageContainer.scrollHeight;
      }, 100);
    }
  }

  // 发送消息
  async function sendMessage() {
    if (!inputMessage.trim() || isProcessing) return;

    const userMessage = inputMessage.trim();
    inputMessage = "";

    const message = new HumanMessage(userMessage);
    // 为消息添加时间戳
    message.additional_kwargs = {
      ...message.additional_kwargs,
      timestamp: new Date().toISOString()
    };

    scrollToBottom();

    await continueAgentFlow([message]);
  }

  // 点击预设提示词
  async function handlePresetPrompt(prompt: string) {
    inputMessage = prompt;
    await sendMessage();
  }

  async function continueAgentFlow(newMessages: any[]) {
    console.log("continueAgentFlow", newMessages);
    messages = [...messages, ...newMessages];
    isProcessing = true;
    try {
      const result = await agent.invoke(
        {
          messages: firstMessage ? messages : newMessages,
        },
        {
          configurable: {
            thread_id: "chat-session",
          },
        }
      );

      console.log("result", result);

      const newLastMessage = result.messages[result.messages.length - 1];
      newLastMessage.additional_kwargs = {
        ...newLastMessage.additional_kwargs,
        timestamp: new Date().toISOString()
      };

      messages = [
        ...messages,
        newLastMessage,
      ];

      console.log("messages", messages);

      // if the last message is a tool call, and it is not sensitive
      if (isToolCall(newLastMessage) && !isSensitiveToolCall(newLastMessage)) {
        // take it as confirmed
        handleToolCallConfirm(newLastMessage.tool_calls[0]);
      }
    } catch (error) {
      console.error("AI 处理错误:", error);
      const errorMessage = new AIMessage("抱歉，处理您的消息时出现了错误。请稍后重试。");
      errorMessage.additional_kwargs = {
        ...errorMessage.additional_kwargs,
        timestamp: new Date().toISOString()
      };
      messages = [
        ...messages,
        errorMessage,
      ];
    } finally {
      isProcessing = false;
      firstMessage = false;
      scrollToBottom();

      // save messages into local storage
      localStorage.setItem('messages', JSON.stringify(messages));
      // save toolCallStates into local storage
      const toolCallStatesObj = {};
      toolCallStates.forEach((value, key) => {
        toolCallStatesObj[key] = value;
      });
      localStorage.setItem('toolCallStates', JSON.stringify(toolCallStatesObj));
    }
  }

  // 处理回车键
  function handleKeyPress(event: KeyboardEvent) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      sendMessage();
    }
  }

  function isSensitiveToolCall(message: any) {
    return message.tool_calls && message.tool_calls.length > 0
      && message.tool_calls.some((tool_call: any) => tool_call.name.includes("delete") || 
      tool_call.name.includes("remove") || 
      tool_call.name.includes("post") || 
      tool_call.name.includes("clip") || 
      tool_call.name.includes("generate") ||
      tool_call.name.includes("generic_ffmpeg_command"));
  }

  function isToolCall(message: any) {
    return message.tool_calls && message.tool_calls.length > 0;
  }

  // 清空对话
  async function clearConversation() {
    console.log("clearConversation");
    messages = [];
    toolCallStates.clear();
    localStorage.removeItem('messages');
    localStorage.removeItem('toolCallStates');
    // 只有当有必要的设置时才重新创建agent
    if (settings.api_key && settings.endpoint) {
      agent = createAgent({
        apiKey: settings.api_key || undefined,
        baseURL: settings.endpoint || undefined,
        model: settings.model || undefined,
      });
    }
    scrollToBottom();
  }

  // 格式化时间
  function formatTime(date: Date): string {
    return date.toLocaleTimeString("zh-CN", {
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  async function handleToolCallConfirm(toolCall: any) {
    console.log("handleToolCallConfirm", toolCall);
    toolCallStates.set(toolCall.id, 'confirmed');
    // update messages to trigger re-render
    messages = [...messages];
    // Execute the tool and resume the flow
    await executeToolAndResume(toolCall);
  }

  async function executeToolAndResume(toolCall: any) {
    isProcessing = true;
    try {
      // Find the tool in the tools array
      const tool = tools.find(t => t.name === toolCall.name);
      if (!tool) {
        throw new Error(`Tool ${toolCall.name} not found`);
      }

      // Execute the tool
      const toolResult = await tool.invoke(toolCall.args);
      console.log("Tool result:", toolResult);

      // Create a ToolMessage with the result
      const toolMessage = new ToolMessage({
        name: toolCall.name,
        content: JSON.stringify(toolResult),
        tool_call_id: toolCall.id || `tool_${Date.now()}`,
      });
      // 为工具消息添加时间戳
      toolMessage.additional_kwargs = {
        ...toolMessage.additional_kwargs,
        timestamp: new Date().toISOString()
      };

      // Continue the agent flow with the tool result
      await continueAgentFlow([toolMessage]);
    } catch (error) {
      console.error("Tool execution error:", error);
      const errorMessage = error instanceof Error ? error.message : String(error);
      const resultMessage = new ToolMessage({
        name: toolCall.name,
        content: errorMessage,
        tool_call_id: toolCall.id || `tool_${Date.now()}`,
      });
      // 为错误工具消息添加时间戳
      resultMessage.additional_kwargs = {
        ...resultMessage.additional_kwargs,
        timestamp: new Date().toISOString()
      };

      await continueAgentFlow([resultMessage]);
    } finally {
      isProcessing = false;
      scrollToBottom();
    }
  }

  async function handleToolCallReject(toolCall: any) {
    console.log("handleToolCallReject", toolCall);

    toolCallStates.set(toolCall.id, 'rejected');
    const resultMessage = new ToolMessage({
      name: toolCall.name,
      content: "用户选择拒绝执行工具",
      tool_call_id: toolCall.id || `tool_${Date.now()}`,
    });
    // 为拒绝的工具消息添加时间戳
    resultMessage.additional_kwargs = {
      ...resultMessage.additional_kwargs,
      timestamp: new Date().toISOString()
    };

    await continueAgentFlow([resultMessage]);

    scrollToBottom();
  }

  onMount(async () => {
    // 加载设置
    loadSettings();
    
    const previousMessages = JSON.parse(localStorage.getItem('messages') || '[]');
    // reconstruct messages
    messages = previousMessages.map((message: any) => {
      // if HumanMessage in messgae.id array
      if (message.id.includes('HumanMessage')) {
        const msg = new HumanMessage(message.kwargs);
        // 恢复时间戳
        if (message.additional_kwargs?.timestamp) {
          msg.additional_kwargs = {
            ...msg.additional_kwargs,
            timestamp: message.additional_kwargs.timestamp
          };
        }
        return msg;
      } else if (message.id.includes('AIMessage')) {
        const msg = new AIMessage(message.kwargs);
        // 恢复时间戳
        if (message.additional_kwargs?.timestamp) {
          msg.additional_kwargs = {
            ...msg.additional_kwargs,
            timestamp: message.additional_kwargs.timestamp
          };
        }
        return msg;
      } else if (message.id.includes('ToolMessage')) {
        const msg = new ToolMessage(message.kwargs);
        // 恢复时间戳
        if (message.additional_kwargs?.timestamp) {
          msg.additional_kwargs = {
            ...msg.additional_kwargs,
            timestamp: message.additional_kwargs.timestamp
          };
        }
        return msg;
      }
    });
    console.log("init messages", messages);
    // init toolCallStates
    toolCallStates.clear();
    const toolCallStatesString = localStorage.getItem('toolCallStates');
    console.log("toolCallStatesString", toolCallStatesString);
    if (toolCallStatesString) {
      const toolCallStatesObj = JSON.parse(toolCallStatesString);
      for (const [key, value] of Object.entries(toolCallStatesObj)) {
        toolCallStates.set(key, value as 'confirmed' | 'rejected' | 'none');
      }
    }

    scrollToBottom();
  });
</script>

<div class="flex-1 flex flex-col bg-gray-50 dark:bg-black h-full">
  <!-- Messages Container -->
  <div
    class="flex-1 overflow-y-auto p-6 space-y-4 custom-scrollbar-light min-h-0"
    bind:this={messageContainer}
  >
    {#if !agent}
      <div class="flex items-center justify-center min-h-[400px] px-6">
        <div class="max-w-sm w-full">
          <div class="text-center">
            <div class="w-20 h-20 bg-gradient-to-br from-blue-500 to-indigo-600 rounded-2xl flex items-center justify-center mx-auto mb-8 shadow-lg shadow-blue-500/20">
              <Settings class="w-10 h-10 text-white" />
            </div>
            <h3 class="text-2xl font-semibold text-gray-900 dark:text-white mb-4 tracking-tight">
              配置 AI 模型
            </h3>
            <p class="text-gray-500 dark:text-gray-400 mb-8 leading-relaxed text-base">
              在使用 AI 助手之前，请先配置您的 OpenAI 兼容 API 设置。
            </p>
            <button
              class="inline-flex items-center justify-center space-x-2 px-8 py-4 bg-blue-500 hover:bg-blue-600 active:bg-blue-700 text-white rounded-xl transition-all duration-200 font-medium text-base shadow-sm hover:shadow-md active:shadow-inner disabled:opacity-50 disabled:cursor-not-allowed"
              on:click={openSettings}
            >
              <Settings class="w-5 h-5" />
              <span>模型设置</span>
            </button>
          </div>
        </div>
      </div>
    {:else if messages.length === 0}
      <div class="text-center py-10">
        <p class="text-gray-500 dark:text-gray-400 text-lg mb-8">
          我是助手小轴，你可以点击下方预设提示词发送第一条消息，或是直接输入你想要执行的操作。
        </p>
        <div class="grid grid-cols-1 sm:grid-cols-2 gap-4 max-w-2xl mx-auto">
          {#each presetPrompts as prompt}
            <button
              class="group relative p-4 bg-white/80 dark:bg-gray-800/80 backdrop-blur-sm border border-gray-200/50 dark:border-gray-700/50 rounded-2xl text-left transition-all duration-200 hover:bg-white/90 dark:hover:bg-gray-800/90 hover:shadow-lg hover:shadow-gray-200/50 dark:hover:shadow-gray-900/50 hover:scale-[1.02] active:scale-[0.98]"
              on:click={() => handlePresetPrompt(prompt.prompt)}
            >
              <div class="flex items-start space-x-3">
                <div class="flex-shrink-0 w-8 h-8 bg-gradient-to-br from-blue-500 to-purple-600 rounded-xl flex items-center justify-center group-hover:from-blue-600 group-hover:to-purple-700 transition-all duration-200">
                  <Sparkles class="w-4 h-4 text-white" />
                </div>
                <div class="flex-1 min-w-0">
                  <h3 class="text-sm font-semibold text-gray-900 dark:text-white group-hover:text-blue-600 dark:group-hover:text-blue-400 transition-colors duration-200">
                    {prompt.title}
                  </h3>
                  <p class="text-xs text-gray-500 dark:text-gray-400 mt-1 leading-relaxed">
                    {prompt.description}
                  </p>
                </div>
              </div>
            </button>
          {/each}
        </div>
      </div>
    {:else}
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
    {/if}

    {#if isProcessing}
      <ProcessingMessageComponent />
    {/if}
</div>

  <!-- Input Area -->
  <div class="border-t border-gray-200 dark:border-gray-700 p-6">
    <div class="flex items-stretch space-x-3">
      <div class="flex-1 flex items-center">
        <textarea
          bind:value={inputMessage}
          on:keypress={handleKeyPress}
          placeholder={!agent ? "请先配置 AI 模型..." : "输入您的消息..."}
          class="w-full px-4 py-3 border border-gray-300 dark:border-gray-600 rounded-xl bg-white dark:bg-gray-700 text-gray-900 dark:text-white placeholder-gray-500 dark:placeholder-gray-400 focus:ring-2 focus:ring-blue-500 focus:border-transparent resize-none min-h-[44px] max-h-[120px] text-sm"
          rows="1"
          disabled={isProcessing || !agent}
        ></textarea>
      </div>

      <button
        class="px-4 py-3 bg-blue-500 hover:bg-blue-600 text-white rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center space-x-2 text-sm font-medium"
        disabled={!inputMessage.trim() || isProcessing || !agent}
        on:click={sendMessage}
      >
        <Send class="w-4 h-4" />
        <span>发送</span>
      </button>
    </div>

    <div class="flex items-center justify-between mt-4">
      <button
        class="px-3 py-1.5 text-sm text-gray-600 dark:text-gray-400 hover:text-gray-800 dark:hover:text-gray-200 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg border border-gray-300 dark:border-gray-600 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed"
        on:click={clearConversation}
        disabled={!agent}
      >
        清空对话
      </button>
      
      <button
        class="p-2 text-gray-600 dark:text-gray-400 hover:text-gray-800 dark:hover:text-gray-200 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg transition-all duration-200"
        on:click={openSettings}
        title="设置"
      >
        <Settings class="w-4 h-4" />
      </button>
    </div>
  </div>

  <!-- Settings Modal -->
  {#if showSettings}
    <div class="fixed inset-0 bg-black/30 backdrop-blur-sm flex items-center justify-center z-50 p-4">
      <div class="bg-white/95 dark:bg-gray-800/95 backdrop-blur-xl rounded-2xl shadow-2xl border border-gray-200/50 dark:border-gray-700/50 w-full max-w-md mx-auto">
        <div class="p-6">
          <div class="flex items-center justify-between mb-6">
            <h3 class="text-xl font-semibold text-gray-900 dark:text-white tracking-tight">模型设置</h3>
            <button
              class="p-2 text-gray-400 hover:text-gray-600 dark:hover:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-xl transition-all duration-200"
              on:click={closeSettings}
            >
              <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"></path>
              </svg>
            </button>
          </div>
          
          <div class="space-y-5">
            <div>
              <label for="endpoint" class="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                OpenAI Compatible API Endpoint
              </label>
              <input
                id="endpoint"
                type="text"
                bind:value={settings.endpoint}
                placeholder="https://api.openai.com/v1"
                class="w-full px-4 py-3 border border-gray-300 dark:border-gray-600 rounded-xl bg-white dark:bg-gray-700 text-gray-900 dark:text-white placeholder-gray-500 dark:placeholder-gray-400 focus:ring-2 focus:ring-blue-500 focus:border-transparent transition-all duration-200"
              />
            </div>
            
            <div>
              <label for="api_key" class="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                API Key
              </label>
              <input
                id="api_key"
                type="password"
                bind:value={settings.api_key}
                placeholder="sk-..."
                class="w-full px-4 py-3 border border-gray-300 dark:border-gray-600 rounded-xl bg-white dark:bg-gray-700 text-gray-900 dark:text-white placeholder-gray-500 dark:placeholder-gray-400 focus:ring-2 focus:ring-blue-500 focus:border-transparent transition-all duration-200"
              />
            </div>

            <div>
              <div class="flex items-center justify-between mb-2">
                <label for="model" class="block text-sm font-medium text-gray-700 dark:text-gray-300">
                  模型选择
                </label>
                <button
                  type="button"
                  class="text-xs text-blue-500 hover:text-blue-600 dark:text-blue-400 dark:hover:text-blue-300 disabled:opacity-50 disabled:cursor-not-allowed px-2 py-1 rounded-lg hover:bg-blue-50 dark:hover:bg-blue-900/20 transition-all duration-200"
                  on:click={loadModels}
                  disabled={!settings.endpoint || !settings.api_key || isLoadingModels}
                >
                  {isLoadingModels ? '加载中...' : '刷新模型列表'}
                </button>
              </div>
              <div class="relative">
                <input
                  id="model"
                  type="text"
                  bind:value={settings.model}
                  list="model-options"
                  placeholder="输入模型名称或从列表中选择"
                  class="w-full px-4 py-3 border border-gray-300 dark:border-gray-600 rounded-xl bg-white dark:bg-gray-700 text-gray-900 dark:text-white placeholder-gray-500 dark:placeholder-gray-400 focus:ring-2 focus:ring-blue-500 focus:border-transparent transition-all duration-200"
                />
                <datalist id="model-options">
                  {#each availableModels as model}
                    <option value={model.value}>{model.label}</option>
                  {/each}
                </datalist>
              </div>
            </div>
          </div>
          
          <div class="flex justify-end space-x-3 mt-8">
            <button
              class="px-6 py-3 text-sm text-gray-600 dark:text-gray-400 hover:text-gray-800 dark:hover:text-gray-200 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-xl border border-gray-300 dark:border-gray-600 transition-all duration-200 font-medium"
              on:click={closeSettings}
            >
              取消
            </button>
            <button
              class="px-6 py-3 text-sm bg-blue-500 hover:bg-blue-600 active:bg-blue-700 text-white rounded-xl transition-all duration-200 font-medium shadow-sm hover:shadow-md active:shadow-inner"
              on:click={saveSettings}
            >
              保存
            </button>
          </div>
        </div>
      </div>
    </div>
  {/if}
</div>
