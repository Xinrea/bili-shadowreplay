<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { Settings, Send, Sparkles, Trash2, Zap, Bot, Paperclip } from "lucide-svelte";
  import { registerCommands } from "../lib/stores/commands";
  import { agentChat } from "../lib/agent/agent";
  import { loadConversation, saveConversation } from "../lib/agent/conversation-store";
  import type { LlmConfig } from "../lib/interface";
  import { invoke, TAURI_ENV } from "../lib/invoker";
  import { invokeToolByName } from "../lib/agent/tools";
  import {
    FILE_ACCEPT,
    loadAttachmentsFromFiles,
    loadAttachmentsFromPaths,
    mergeAttachments,
    type AttachmentLoadError,
  } from "../lib/agent/attachments";
  import {
    isAssistantMessage,
    isToolMessage,
    type ChatMessage,
    type MessageAttachment,
    type ToolCall,
    type ToolMessage,
  } from "../lib/agent/messages";
  import AgentAttachmentList from "../lib/components/AgentAttachmentList.svelte";
  import HumanMessageComponent from "../lib/components/HumanMessage.svelte";
  import AIMessageComponent from "../lib/components/AIMessage.svelte";
  import ProcessingMessageComponent from "../lib/components/ProcessingMessage.svelte";
  import ToolMessageComponent from "../lib/components/ToolMessage.svelte";

  interface Props {
    onNavigateSettings?: () => void;
  }

  let { onNavigateSettings }: Props = $props();

  let messages: ChatMessage[] = $state([]);
  let inputMessage = $state("");
  let attachments: MessageAttachment[] = $state([]);
  let attachmentError = $state("");
  let storageError = $state("");
  let conversationLoaded = $state(false);
  let isRestoringConversation = $state(false);
  let isDraggingFiles = $state(false);
  let isProcessing = $state(false);
  let messageContainer: HTMLElement = $state();
  let inputAreaHeight = $state(0);
  let fileInput: HTMLInputElement | undefined = $state();
  let agentConfigured = $state(false);
  let stopDragDrop: (() => void) | undefined;

  let settings = $state({
    provider: "openai" as "openai" | "ollama",
    endpoint: "",
    api_key: "",
    model: ""
  });

  type ToolCallState = 'confirmed' | 'rejected' | 'none';

  // 预设提示词
  const presetPrompts = [
    { title: "录制直播", description: "添加新的直播间并开始录制", prompt: "我该如何添加新的直播间？", icon: "📡" },
    { title: "查看任务", description: "显示所有录制任务状态", prompt: "显示我所有的录制任务", icon: "📊" },
    { title: "管理账户", description: "查看已添加的账户信息", prompt: "显示可用的账号信息", icon: "👤" },
    { title: "生成切片", description: "分析录播并生成精彩片段", prompt: "分析最新录制的录播有哪些精彩部分，选择一段生成切片", icon: "✂️" },
    { title: "视频转码", description: "转换视频格式", prompt: "帮我将视频转码为mp4格式", icon: "🎬" },
    { title: "提取音频", description: "从视频中提取音频", prompt: "帮我提取视频中的音频", icon: "🎵" }
  ];

  function openSettings() {
    onNavigateSettings?.();
  }

  function updateConfiguredState() {
    agentConfigured = Boolean(
      settings.endpoint.trim() &&
      settings.model.trim() &&
      (settings.provider === 'ollama' || settings.api_key.trim()),
    );
  }

  async function loadSettings() {
    try {
      const savedSettings = await invoke<LlmConfig>('get_llm_config');
      settings = { ...settings, ...savedSettings };
      updateConfiguredState();
    } catch (error) {
      console.error('Failed to load AI settings:', error);
    }
  }

  function getToolCallState(toolCallId: string): ToolCallState {
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      const message = messages[index];
      if (isToolMessage(message) && message.toolCallId === toolCallId) {
        return message.resolution;
      }
    }
    return 'none';
  }

  function hasUnresolvedPendingToolCalls(): boolean {
    return messages.some(
      (message) =>
        isAssistantMessage(message) &&
        message.toolCalls.some(
          (toolCall) =>
            toolCall.executed === false &&
            getToolCallState(toolCall.id) === 'none',
        ),
    );
  }

  let hasPendingToolCalls = $derived(hasUnresolvedPendingToolCalls());
  let canSend = $derived(
    Boolean(inputMessage.trim() || attachments.length) &&
      !isProcessing &&
      !hasPendingToolCalls &&
      conversationLoaded &&
      agentConfigured,
  );

  async function persistConversation() {
    try {
      await saveConversation($state.snapshot(messages));
      storageError = "";
    } catch (error) {
      console.error('Failed to persist AI conversation:', error);
      storageError = "对话保存失败，请重试保存后再关闭或刷新页面。";
    }
  }

  async function restoreConversation() {
    if (isRestoringConversation) return;
    isRestoringConversation = true;
    try {
      messages = await loadConversation();
      conversationLoaded = true;
      storageError = "";
      scrollToBottom();
    } catch (error) {
      console.error('Failed to load AI conversation:', error);
      storageError = "无法加载历史对话，请重试。";
    } finally {
      isRestoringConversation = false;
    }
  }

  function toolResultContent(result: unknown): string {
    if (typeof result === 'string') return result;
    return JSON.stringify(result) ?? String(result);
  }

  function scrollToBottom() {
    if (messageContainer) {
      setTimeout(() => { messageContainer.scrollTop = messageContainer.scrollHeight; }, 100);
    }
  }

  function formatTime(timestamp: string | Date): string {
    const date = new Date(timestamp);
    return date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' });
  }

  async function handlePresetPrompt(prompt: string) {
    inputMessage = prompt;
    await sendMessage();
  }

  async function sendMessage() {
    if (!canSend) return;
    messages = [...messages, {
      kind: 'human',
      content: inputMessage.trim(),
      timestamp: new Date().toISOString(),
      attachments,
    }];
    inputMessage = "";
    attachments = [];
    attachmentError = "";
    isProcessing = true;
    void persistConversation();
    scrollToBottom();
    try {
      await continueAgentFlow();
    } finally {
      isProcessing = false;
      scrollToBottom();
    }
  }

  function formatAttachmentErrors(errors: AttachmentLoadError[], skipped: number): string {
    const parts = errors.map((error) => `${error.name}：${error.message}`);
    if (skipped > 0) {
      parts.push(`最多添加 8 个附件，已忽略 ${skipped} 个文件`);
    }
    return parts.join("；");
  }

  async function addLoadedAttachments(
    loaded: MessageAttachment[],
    errors: AttachmentLoadError[],
  ) {
    const merged = mergeAttachments(attachments, loaded);
    attachments = merged.attachments;
    attachmentError = formatAttachmentErrors(errors, merged.skipped);
  }

  async function addAttachmentsFromFileList(files: Iterable<File>) {
    const { attachments: loaded, errors } = await loadAttachmentsFromFiles(files);
    await addLoadedAttachments(loaded, errors);
  }

  async function addAttachmentsFromPaths(paths: string[]) {
    const { attachments: loaded, errors } = await loadAttachmentsFromPaths(paths);
    await addLoadedAttachments(loaded, errors);
  }

  function removeAttachment(index: number) {
    attachments = attachments.filter((_, current) => current !== index);
  }

  function openFilePicker() {
    if (isProcessing || hasPendingToolCalls) return;
    fileInput?.click();
  }

  async function handleFileInput(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const files = input.files;
    if (files && files.length > 0) {
      await addAttachmentsFromFileList(files);
    }
    input.value = "";
  }

  function handleDragOver(event: DragEvent) {
    if (![...(event.dataTransfer?.types ?? [])].includes("Files")) return;
    event.preventDefault();
    isDraggingFiles = true;
  }

  function handleDragLeave(event: DragEvent) {
    const next = event.relatedTarget as Node | null;
    if (next && (event.currentTarget as HTMLElement).contains(next)) return;
    isDraggingFiles = false;
  }

  async function handleDrop(event: DragEvent) {
    event.preventDefault();
    isDraggingFiles = false;
    if (TAURI_ENV) return;
    const files = event.dataTransfer?.files;
    if (files && files.length > 0) {
      await addAttachmentsFromFileList(files);
    }
  }

  async function handlePaste(event: ClipboardEvent) {
    const files = [...(event.clipboardData?.files ?? [])];
    if (files.length > 0) {
      event.preventDefault();
      await addAttachmentsFromFileList(files);
      return;
    }

    const imageItems = [...(event.clipboardData?.items ?? [])]
      .map((item) => item.type.startsWith("image/") ? item.getAsFile() : null)
      .filter((file): file is File => Boolean(file));
    if (imageItems.length === 0) return;
    event.preventDefault();
    await addAttachmentsFromFileList(imageItems);
  }

  async function continueAgentFlow() {
    if (!agentConfigured) return;
    try {
      const response = await agentChat(messages);
      messages = [...messages, response];
      persistConversation();
      scrollToBottom();
    } catch (error) {
      console.error('LLM API Error:', error);
      const message = error instanceof Error ? error.message : String(error);
      messages = [...messages, {
        kind: 'assistant',
        content: `❌ **LLM API 错误**\n\n${message}\n\n请检查：\n- API 端点是否正确\n- API 密钥是否有效\n- 网络连接是否正常\n- 模型名称是否正确`,
        timestamp: new Date().toISOString(),
        toolCalls: [],
        isError: true,
      }];
      persistConversation();
      scrollToBottom();
    }
  }

  async function handleToolCallConfirm(toolCall: ToolCall) {
    if (
      isProcessing ||
      !conversationLoaded ||
      toolCall.executed !== false ||
      !toolCall.id ||
      getToolCallState(toolCall.id) !== 'none'
    ) return;

    isProcessing = true;
    let resultMessage: ToolMessage;
    try {
      const result = await invokeToolByName(toolCall.name, toolCall.args);
      resultMessage = {
        kind: 'tool',
        name: toolCall.name,
        content: toolResultContent(result),
        timestamp: new Date().toISOString(),
        toolCallId: toolCall.id,
        status: 'success',
        resolution: 'confirmed',
      };
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      console.error(`Error executing tool ${toolCall.name}:`, error);
      resultMessage = {
        kind: 'tool',
        name: toolCall.name,
        content: JSON.stringify({
          error: true,
          message,
          tool: toolCall.name,
          args: toolCall.args,
        }),
        timestamp: new Date().toISOString(),
        toolCallId: toolCall.id,
        status: 'error',
        resolution: 'confirmed',
      };
    }

    messages = [...messages, resultMessage];
    persistConversation();
    scrollToBottom();

    try {
      if (!hasUnresolvedPendingToolCalls()) {
        await continueAgentFlow();
      }
    } finally {
      isProcessing = false;
      scrollToBottom();
    }
  }

  async function handleToolCallReject(toolCall: ToolCall) {
    if (
      isProcessing ||
      !conversationLoaded ||
      toolCall.executed !== false ||
      !toolCall.id ||
      getToolCallState(toolCall.id) !== 'none'
    ) return;

    isProcessing = true;
    const resultMessage: ToolMessage = {
      kind: 'tool',
      name: toolCall.name,
      content: JSON.stringify({
        rejected: true,
        message: '用户拒绝执行该工具调用',
        tool: toolCall.name,
        args: toolCall.args,
      }),
      timestamp: new Date().toISOString(),
      toolCallId: toolCall.id,
      status: 'error',
      resolution: 'rejected',
    };
    messages = [...messages, resultMessage];
    persistConversation();
    scrollToBottom();

    try {
      if (!hasUnresolvedPendingToolCalls()) {
        await continueAgentFlow();
      }
    } finally {
      isProcessing = false;
      scrollToBottom();
    }
  }

  function clearConversation() {
    if (!conversationLoaded || isProcessing) return;
    messages = [];
    attachments = [];
    attachmentError = "";
    void persistConversation();
    scrollToBottom();
  }

  onMount(() =>
    registerCommands([
      {
        id: "ai:clear",
        group: "action",
        title: "清空助手对话",
        detail: "助手",
        icon: Trash2,
        keywords: ["clear conversation", "new chat", "reset"],
        page: "助手",
        available: () => agentConfigured && conversationLoaded && !isProcessing,
        run: clearConversation,
      },
    ]),
  );

  function handleKeyPress(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  }

  onMount(() => {
    void loadSettings();
    window.addEventListener("llm-config-updated", loadSettings);
    window.addEventListener("paste", handlePaste);
    void restoreConversation();
    localStorage.removeItem('toolCallStates'); // Remove the obsolete parallel state store.
    scrollToBottom();

    if (TAURI_ENV) {
      void import("@tauri-apps/api/webview").then(async ({ getCurrentWebview }) => {
        stopDragDrop = await getCurrentWebview().onDragDropEvent((event) => {
          if (event.payload.type === "enter" || event.payload.type === "over") {
            isDraggingFiles = true;
            return;
          }
          isDraggingFiles = false;
          if (event.payload.type === "drop") {
            void addAttachmentsFromPaths(event.payload.paths);
          }
        });
      });
    }
  });

  onDestroy(() => {
    window.removeEventListener("llm-config-updated", loadSettings);
    window.removeEventListener("paste", handlePaste);
    stopDragDrop?.();
  });
</script>

<div
  class="relative flex h-full bg-canvas"
  role="region"
  aria-label="AI 助手"
  ondragover={handleDragOver}
  ondragleave={handleDragLeave}
  ondrop={handleDrop}
>
  <!-- Main Content -->
  <div class="relative flex flex-1 flex-col">
    <!-- Messages Area -->
    <div class="flex-1 overflow-y-auto" bind:this={messageContainer}>
      <div class="mx-auto max-w-4xl px-6 py-6" style="padding-bottom: {inputAreaHeight + 16}px;">
        {#if !agentConfigured}
          <!-- Welcome State -->
          <div class="flex min-h-[500px] items-center justify-center">
            <div class="max-w-md space-y-5 text-center">
              <div class="inline-flex h-16 w-16 items-center justify-center rounded-[16px] bg-gradient-to-br from-amber-400 to-orange-500">
                <span class="text-[32px] leading-none">🍊</span>
              </div>
              <div class="space-y-2">
                <h2 class="text-[18px] font-semibold text-fg">欢迎使用 AI 助手</h2>
                <p class="text-[13px] leading-relaxed text-fg-secondary">
                  配置您的 AI 模型以开始智能对话。支持 OpenAI 兼容 API 和本地 Ollama 模型。
                </p>
              </div>
              <button
                onclick={openSettings}
                class="ui-btn ui-btn-primary px-4"
              >
                <Settings class="h-3.5 w-3.5" />
                <span>开始配置</span>
              </button>
              <div class="flex items-center justify-center gap-5 pt-3 text-[12px] text-fg-muted">
                <div class="flex items-center gap-1.5">
                  <Zap class="h-3.5 w-3.5" />
                  <span>快速响应</span>
                </div>
                <div class="flex items-center gap-1.5">
                  <Sparkles class="h-3.5 w-3.5" />
                  <span>智能分析</span>
                </div>
              </div>
            </div>
          </div>
        {:else if messages.length === 0}
          <!-- Empty State with Prompts -->
          <div class="space-y-7">
            <div class="space-y-3 text-center">
              <div class="inline-flex h-12 w-12 items-center justify-center rounded-[12px] bg-gradient-to-br from-amber-400 to-orange-500">
                <span class="text-[24px] leading-none">🍊</span>
              </div>
              <div>
                <h2 class="text-[16px] font-semibold text-fg">你好！我是小轴</h2>
                <p class="mt-1.5 text-[12px] text-fg-muted">
                  我可以帮你管理直播录制、生成精彩切片、分析弹幕内容等。点击下方卡片快速开始。
                </p>
              </div>
            </div>
            <div class="grid grid-cols-1 gap-2.5 sm:grid-cols-2 lg:grid-cols-3">
              {#each presetPrompts as prompt}
                <button
                  onclick={() => handlePresetPrompt(prompt.prompt)}
                  class="ui-card group p-3 text-left transition-colors hover:border-line-strong hover:bg-control/60"
                >
                  <div class="flex items-start gap-3">
                    <span class="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-[8px] bg-control text-[17px] leading-none transition-colors group-hover:bg-control-hover">{prompt.icon}</span>
                    <div class="min-w-0 flex-1">
                      <div class="mb-0.5 text-[13px] font-medium text-fg">{prompt.title}</div>
                      <p class="text-[11px] leading-relaxed text-fg-muted">{prompt.description}</p>
                    </div>
                  </div>
                </button>
              {/each}
            </div>
          </div>
        {:else}
          <!-- Messages -->
          <div class="space-y-5">
            {#each messages as message, index (index)}
              {#if message.kind === 'human'}
                <HumanMessageComponent {message} {formatTime} />
              {:else if message.kind === 'assistant'}
                <AIMessageComponent
                  {message}
                  {formatTime}
                  {getToolCallState}
                  onToolCallConfirm={handleToolCallConfirm}
                  onToolCallReject={handleToolCallReject}
                  confirmationDisabled={isProcessing || !conversationLoaded}
                />
              {:else}
                <ToolMessageComponent {message} {formatTime} />
              {/if}
            {/each}
          </div>
        {/if}

        {#if isProcessing}
          <div class="mt-5">
            <ProcessingMessageComponent />
          </div>
        {/if}
      </div>
    </div>

    <!-- Floating Input Area -->
    <div bind:offsetHeight={inputAreaHeight} class="absolute bottom-0 left-0 right-0 bg-gradient-to-t from-canvas via-canvas to-canvas/0 px-6 pb-4 pt-8">
      <div class="mx-auto max-w-4xl">
        <div class="overflow-hidden rounded-[12px] border border-line bg-surface shadow-lg shadow-black/5 transition-colors focus-within:border-accent">
          {#if attachments.length > 0}
            <div class="px-3.5 pt-3">
              <AgentAttachmentList
                {attachments}
                removable
                compact
                onRemove={removeAttachment}
              />
            </div>
          {/if}
          <!-- Textarea -->
          <div class="relative">
            <textarea
              bind:value={inputMessage}
              onkeypress={handleKeyPress}
              placeholder={!agentConfigured ? "请先配置 AI 模型..." : hasPendingToolCalls ? "请先确认或拒绝待执行的工具调用..." : "输入消息，或拖入 / 粘贴图片和文本文件..."}
              class="block max-h-[200px] min-h-[48px] w-full resize-none border-0 bg-transparent px-3.5 py-3 text-[13px] leading-relaxed text-fg shadow-none placeholder:text-fg-faint focus:border-0 focus:outline-none focus:ring-0 disabled:cursor-not-allowed disabled:opacity-50"
              rows="1"
              disabled={isProcessing || hasPendingToolCalls || !agentConfigured || !conversationLoaded}
            ></textarea>
          </div>

          {#if attachmentError}
            <div class="px-3.5 pb-2 text-[11px] text-danger-fg">
              {attachmentError}
            </div>
          {/if}

          {#if storageError}
            <div class="px-3.5 pb-2 text-[11px] text-danger-fg" role="alert">
              {storageError}
              <button
                class="ml-2 font-medium underline underline-offset-2 hover:opacity-80 disabled:cursor-not-allowed disabled:opacity-50"
                disabled={isRestoringConversation}
                onclick={() => conversationLoaded ? persistConversation() : restoreConversation()}
              >重试</button>
            </div>
          {/if}

          <!-- Bottom bar: model info + actions -->
          <div class="flex items-center justify-between gap-3 border-t border-line px-2 py-1.5">
            <div class="flex min-w-0 items-center gap-1">
              <!-- Model info -->
              <button
                onclick={openSettings}
                class="ui-btn ui-btn-sm ui-btn-ghost min-w-0 shrink font-normal text-fg-muted"
                title="点击配置模型"
              >
                <Bot class="h-3.5 w-3.5 flex-shrink-0" />
                {#if agentConfigured}
                  <span class="truncate">{settings.provider === 'ollama' ? 'Ollama' : 'OpenAI'} · {settings.model || '未设置模型'}</span>
                {:else}
                  <span class="truncate">未配置模型</span>
                {/if}
              </button>

              <button
                class="ui-btn ui-btn-sm ui-btn-ghost font-normal text-fg-muted"
                onclick={clearConversation}
                disabled={!agentConfigured || !conversationLoaded || isProcessing}
                title="清空对话"
              >
                <Trash2 class="h-3.5 w-3.5" />
                <span>清空</span>
              </button>
            </div>

            <div class="flex flex-shrink-0 items-center gap-1.5">
              <input
                bind:this={fileInput}
                type="file"
                class="hidden"
                multiple
                accept={FILE_ACCEPT}
                onchange={handleFileInput}
              />
              <button
                type="button"
                class="ui-btn ui-btn-icon ui-btn-ghost text-fg-muted"
                onclick={openFilePicker}
                disabled={isProcessing || hasPendingToolCalls}
                title="添加图片或文本文件"
              >
                <Paperclip class="h-4 w-4" />
              </button>
              {#if inputMessage.trim()}
                <span class="text-[11px] tabular-nums text-fg-faint">{inputMessage.length}</span>
              {/if}
              <button
                class="ui-btn ui-btn-primary"
                disabled={!canSend}
                onclick={sendMessage}
              >
                <Send class="h-3.5 w-3.5" />
                <span>发送</span>
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>

  {#if isDraggingFiles}
    <div class="pointer-events-none absolute inset-0 z-20 flex items-center justify-center bg-canvas/60 backdrop-blur-[2px]">
      <div class="rounded-[12px] border border-dashed border-accent/70 bg-surface px-6 py-4 text-center shadow-xl">
        <p class="text-[13px] font-medium text-fg">松开以添加图片或文本文件</p>
        <p class="mt-1 text-[11px] text-fg-muted">最多 8 个附件，不支持视频</p>
      </div>
    </div>
  {/if}

</div>

<style>
  :global(.overflow-y-auto) {
    scrollbar-width: thin;
    scrollbar-color: rgb(var(--c-scrollbar)) transparent;
  }
  :global(.overflow-y-auto::-webkit-scrollbar) {
    width: 6px;
  }
  :global(.overflow-y-auto::-webkit-scrollbar-track) {
    background: transparent;
  }
  :global(.overflow-y-auto::-webkit-scrollbar-thumb) {
    background: rgb(var(--c-scrollbar));
    border-radius: 3px;
  }
  :global(.overflow-y-auto::-webkit-scrollbar-thumb:hover) {
    background: rgb(var(--c-scrollbar-hover));
  }
</style>
