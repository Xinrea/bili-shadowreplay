<script lang="ts">
  import {
    Wrench,
    CheckCircle,
    AlertCircle,
    ChevronDown,
    ChevronRight,
  } from "lucide-svelte";
  import {
    messageContentToDisplayParts,
    messageContentToMarkdown,
    type ToolMessage,
  } from "../agent/messages";
  import CopyMarkdownButton from "./CopyMarkdownButton.svelte";

  interface Props {
    message: ToolMessage;
    formatTime: (date: Date) => string;
  }

  let { message, formatTime }: Props = $props();

  // 折叠状态 - 默认折叠
  let isExpanded = $state(false);

  let messageTime = $derived(new Date(message.timestamp));
  let contentParts = $derived(messageContentToDisplayParts(message.content));

  function markdownContent(): string {
    return messageContentToMarkdown(message.content);
  }

  // 获取状态图标和颜色
  function getStatusInfo() {
    if (message.status === "success") {
      return {
        icon: CheckCircle,
        color: "text-success",
        bgColor: "bg-success-soft",
        borderColor: "border-success/40",
      };
    } else {
      return {
        icon: AlertCircle,
        color: "text-danger",
        bgColor: "bg-danger-soft",
        borderColor: "border-danger/40",
      };
    }
  }

  // 格式化工具调用ID
  function formatToolCallId(id: string): string {
    return id.length > 8 ? id.slice(-8) : id;
  }

  // 切换折叠状态
  function toggleExpanded() {
    isExpanded = !isExpanded;
  }

  let statusInfo = $derived(getStatusInfo());
  let StatusIcon = $derived(statusInfo.icon);
</script>

<div class="flex justify-start">
  <div class="flex max-w-2xl items-start gap-2.5">
    <div
      class="flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-full border border-success/30 bg-success-soft text-success-fg"
    >
      <Wrench class="h-3.5 w-3.5" />
    </div>

    <div class="flex min-w-0 flex-col space-y-1">
      <div class="flex h-6 items-center gap-2">
        <span class="text-[12px] font-medium text-fg-secondary">
          工具响应
        </span>
        <span class="text-[11px] tabular-nums text-fg-muted">
          {formatTime(messageTime)}
        </span>
        <CopyMarkdownButton content={markdownContent} />
      </div>

      <div
        class="rounded-[10px] border border-line bg-surface px-3.5 py-2.5"
      >
        <div class="text-[13px] leading-relaxed text-fg">
          <!-- 工具信息头部 -->
          <div class="mb-2">
            <div class="flex items-center gap-1.5">
              <StatusIcon
                class="h-3.5 w-3.5 flex-shrink-0 {statusInfo.color}"
              />
              <span
                class="min-w-0 break-all text-[12px] font-medium text-fg"
              >
                {message.name}
              </span>
              <span class="flex-shrink-0 text-[11px] text-fg-muted">
                (ID: {formatToolCallId(message.toolCallId)})
              </span>
            </div>
          </div>

          <!-- 折叠按钮和内容 -->
          <div class="space-y-2">
            <!-- 折叠按钮 -->
            <button
              onclick={toggleExpanded}
              class="flex items-center gap-1 rounded-[6px] text-[12px] text-fg-muted transition-colors hover:text-fg"
            >
              {#if isExpanded}
                <ChevronDown class="h-3.5 w-3.5" />
              {:else}
                <ChevronRight class="h-3.5 w-3.5" />
              {/if}
              <span>{isExpanded ? "收起详情" : "展开详情"}</span>
            </button>

            <!-- 折叠内容 -->
            {#if isExpanded}
              <div
                class="rounded-[8px] border border-line bg-sunken p-3"
              >
                <div
                  class="space-y-3 text-[12px] leading-relaxed text-fg-secondary"
                >
                  {#if contentParts.length === 0}
                    无响应内容
                  {:else}
                    {#each contentParts as part}
                      {#if part.kind === "image"}
                        <img
                          src={part.src}
                          alt={part.alt}
                          loading="lazy"
                          class="max-h-[28rem] max-w-full rounded-[8px] border border-line object-contain"
                        />
                      {:else if part.format === "json"}
                        <pre class="overflow-x-auto whitespace-pre-wrap break-words text-[11px]">{part.text}</pre>
                      {:else}
                        <div class="whitespace-pre-wrap break-words">{part.text}</div>
                      {/if}
                    {/each}
                  {/if}
                </div>
              </div>
            {/if}
          </div>

          <!-- 状态信息 -->
          <div class="ui-badge mt-2 {message.status === 'success' ? 'ui-badge-success' : 'ui-badge-danger'}">
            状态: {message.status}
          </div>
        </div>
      </div>
    </div>
  </div>
</div>
