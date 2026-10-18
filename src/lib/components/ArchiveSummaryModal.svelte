<script lang="ts">
  import { onDestroy } from "svelte";
  import { marked } from "marked";
  import { AlertCircle, Loader2, RefreshCw, Sparkles, X } from "lucide-svelte";
  import { invoke } from "../invoker";
  import ArchiveClipButton from "./ArchiveClipButton.svelte";
  import CopyMarkdownButton from "./CopyMarkdownButton.svelte";
  import TaskCenterButton from "./TaskCenterButton.svelte";
  import type {
    RecordItem,
    RecordSummary,
    RecordSummaryStatus,
    SummaryHighlight,
    TaskRow,
  } from "../db";
  import type { LlmConfig } from "../interface";

  interface Props {
    showModal?: boolean;
    archive?: RecordItem | null;
    onUpdated?: (status: RecordSummaryStatus) => void;
  }

  let { showModal = $bindable(false), archive = null, onUpdated }: Props = $props();

  let summary: RecordSummary | null = $state(null);
  let loading = $state(false);
  let actionLoading = $state(false);
  let error = $state("");
  let loadedKey = $state("");
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  let llmConfig: LlmConfig | null = $state(null);
  let llmLoadedKey = $state("");
  let llmConfigLoading = $state(true);
  let llmConfigError = $state(false);


  function clearPoll() {
    if (pollTimer) {
      clearTimeout(pollTimer);
      pollTimer = null;
    }
  }

  function close() {
    clearPoll();
    showModal = false;
    loadedKey = "";
    llmLoadedKey = "";
    llmConfig = null;
    llmConfigLoading = true;
    llmConfigError = false;
  }

  function goToSettings() {
    close();
    window.dispatchEvent(new CustomEvent("navigate-settings"));
  }

  async function loadSummary(showLoading = true) {
    if (!archive) return;
    if (showLoading) loading = true;
    error = "";
    try {
      summary = await invoke<RecordSummary | null>("get_archive_summary", {
        platform: archive.platform,
        roomId: archive.room_id,
        liveId: archive.live_id,
      });
      emitStatus();
      if (summary?.status === "processing") schedulePoll();
      else clearPoll();
    } catch (loadError) {
      error = String(loadError);
    } finally {
      loading = false;
    }
  }

  function schedulePoll() {
    clearPoll();
    pollTimer = setTimeout(async () => {
      await loadSummary(false);
    }, 2000);
  }

  async function loadLlmConfig(key: string) {
    llmConfigLoading = true;
    llmConfigError = false;
    try {
      llmConfig = await invoke<LlmConfig>("get_llm_config");
      llmLoadedKey = key;
    } catch {
      // 读取失败时无法校验 API Key，保守禁止生成并提示前往设置
      llmConfig = null;
      llmConfigError = true;
    } finally {
      llmConfigLoading = false;
    }
  }

  async function generate(force: boolean) {
    if (!archive || actionLoading) return;
    actionLoading = true;
    error = "";
    try {
      const task = await invoke<TaskRow>("generate_archive_summary", {
        platform: archive.platform,
        roomId: archive.room_id,
        liveId: archive.live_id,
        force,
      });
      summary = {
        ...(summary || ({} as RecordSummary)),
        platform: archive.platform,
        room_id: archive.room_id,
        live_id: archive.live_id,
        status: "processing",
        stage: summary?.subtitle_text && !force ? "summarizing" : "extracting_audio",
        task_id: task.id,
      };
      emitStatus();
      schedulePoll();
    } catch (generateError) {
      error = String(generateError);
      await loadSummary(false);
    } finally {
      actionLoading = false;
    }
  }

  function parseHighlights(value: string | null | undefined): SummaryHighlight[] {
    if (!value) return [];
    try {
      const parsed = JSON.parse(value);
      return Array.isArray(parsed) ? parsed : [];
    } catch {
      return [];
    }
  }

  function emitStatus() {
    if (!summary || !archive) return;
    onUpdated?.({
      platform: archive.platform,
      room_id: archive.room_id,
      live_id: archive.live_id,
      status: summary.status,
      stage: summary.stage,
    });
  }

  function formatTime(seconds: number): string {
    const total = Math.max(0, Math.round(seconds));
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const secs = total % 60;
    return [hours, minutes, secs]
      .map((part) => part.toString().padStart(2, "0"))
      .join(":");
  }

  function stageText(stage: string): string {
    switch (stage) {
      case "extracting_audio":
        return "正在提取完整音频";
      case "transcribing":
        return "正在生成字幕";
      case "summarizing":
        return "正在总结直播内容";
      default:
        return "正在准备 Summary";
    }
  }

  onDestroy(clearPoll);
  let currentKey = $derived(archive
    ? `${archive.platform}:${archive.room_id}:${archive.live_id}`
    : "");
  let llmKey = $derived(showModal && archive ? "load" : "");
  $effect(() => {
    if (llmKey && !llmLoadedKey) {
      void loadLlmConfig(llmKey);
    }
  });
  $effect(() => {
    if (showModal && archive && currentKey !== loadedKey) {
      loadedKey = currentKey;
      void loadSummary();
    }
  });
  let needsApiKey = $derived(
    llmConfig?.provider === "openai" && !(llmConfig.api_key || "").trim()
  );
  let llmConfigUnavailable = $derived(llmConfigLoading || llmConfigError);
  let highlights = $derived(parseHighlights(summary?.highlights_json));
  let summaryMarkdown = $derived(summary?.summary_markdown || "");
</script>

{#if showModal && archive}
  <div class="ui-modal-backdrop p-6">
    <div class="ui-modal max-h-[90vh] w-full max-w-4xl">
      <div class="flex shrink-0 items-center justify-between gap-3 border-b border-line px-[18px] py-3">
        <div class="min-w-0">
          <div class="flex items-center gap-2">
            <Sparkles class="h-4 w-4 text-accent-fg" />
            <h2 class="text-[14px] font-semibold text-fg">直播 Summary</h2>
            {#if summary?.status === "success"}
              <CopyMarkdownButton content={summaryMarkdown} />
            {/if}
          </div>
          <p class="mt-0.5 max-w-2xl truncate text-[12px] text-fg-muted">
            {archive.title}
          </p>
        </div>
        <div class="flex shrink-0 items-center gap-2">
          <TaskCenterButton />
          <button class="ui-btn ui-btn-ghost ui-btn-icon" onclick={close}>
            <X class="h-4 w-4" />
          </button>
        </div>
      </div>

      <div class="ui-modal-body custom-scrollbar-light">
        {#if loading}
          <div class="flex min-h-[260px] items-center justify-center gap-2.5 text-[13px] text-fg-muted">
            <Loader2 class="h-5 w-5 animate-spin" />
            <span>读取 Summary 中</span>
          </div>
        {:else if summary?.status === "processing"}
          <div class="flex min-h-[260px] flex-col items-center justify-center gap-4 text-center">
            <Loader2 class="h-8 w-8 animate-spin text-accent-fg" />
            <div>
              <p class="text-[13px] font-medium text-fg">{stageText(summary.stage)}</p>
              <p class="mt-1 text-[12px] text-fg-muted">任务在后台执行，关闭窗口不会中断处理。</p>
            </div>
          </div>
        {:else if summary?.status === "success"}
          <div class="space-y-5">
            <div class="summary-markdown text-[13px] text-fg">
              {@html marked(summaryMarkdown)}
            </div>

            {#if highlights.length > 0}
              <div class="space-y-2">
                <h3 class="ui-section-title">精彩时间段</h3>
                {#each highlights as highlight}
                  <div class="rounded-[10px] border border-line bg-control p-3">
                    <div class="flex items-start justify-between gap-3">
                      <span class="text-[13px] font-medium text-fg">{highlight.title}</span>
                      <div class="flex shrink-0 items-center gap-2">
                        <span class="whitespace-nowrap rounded-full bg-accent-soft px-2 py-0.5 text-[11px] font-medium tabular-nums text-accent-fg">
                          {formatTime(highlight.start_seconds)}–{formatTime(highlight.end_seconds)}
                        </span>
                        <ArchiveClipButton
                          {archive}
                          ranges={[
                            {
                              start: highlight.start_seconds,
                              end: highlight.end_seconds,
                            },
                          ]}
                          initialNote={highlight.title}
                          variant="compact"
                        />
                      </div>
                    </div>
                    <p class="mt-1.5 text-[12px] leading-relaxed text-fg-secondary">{highlight.reason}</p>
                  </div>
                {/each}
              </div>
            {/if}

            {#if summary.subtitle_srt}
              <details class="overflow-hidden rounded-[10px] border border-line">
                <summary class="cursor-pointer px-3.5 py-2.5 text-[12px] font-medium text-fg-secondary transition-colors hover:text-fg">查看完整字幕</summary>
                <pre class="max-h-80 overflow-auto whitespace-pre-wrap border-t border-line bg-sunken p-3.5 text-[11px] leading-relaxed text-fg-secondary custom-scrollbar-light">{summary.subtitle_srt}</pre>
              </details>
            {/if}

          </div>
        {:else if summary?.status === "failed"}
          <div class="flex min-h-[260px] flex-col items-center justify-center gap-4 text-center">
            <AlertCircle class="h-8 w-8 text-danger-fg" />
            <div>
              <p class="text-[13px] font-medium text-fg">Summary 生成失败</p>
              <p class="mt-2 max-w-xl text-[12px] text-danger-fg">{summary.error_message || "未知错误"}</p>
            </div>
            <button class="ui-btn ui-btn-primary" onclick={() => generate(false)} disabled={actionLoading}>
              <RefreshCw class="h-3.5 w-3.5 {actionLoading ? 'animate-spin' : ''}" />
              从失败阶段重试
            </button>
          </div>
        {:else}
          <div class="flex min-h-[260px] flex-col items-center justify-center gap-4 text-center">
            <Sparkles class="h-10 w-10 text-accent-fg" />
            <div>
              <p class="text-[13px] font-medium text-fg">尚未生成 Summary</p>
              <p class="mt-1 text-[12px] text-fg-muted">将依次提取完整音频、生成字幕并总结直播内容。</p>
            </div>
            {#if needsApiKey}
              <div class="max-w-md rounded-[10px] border border-warning/40 bg-warning-soft px-3.5 py-3 text-left text-[12px] text-warning-fg">
                <p class="font-medium">尚未配置 LLM API Key</p>
                <p class="mt-1 leading-relaxed">生成 Summary 需要调用 LLM 接口。请在「设置」页填写 OpenAI 兼容接口的 API Key（若使用本地 Ollama 则无需配置）。</p>
                <button
                  class="mt-3 inline-flex h-[26px] items-center rounded-[7px] border border-warning/50 bg-warning/10 px-2.5 text-[11px] font-medium text-warning-fg transition-colors hover:bg-warning/20"
                  onclick={goToSettings}
                >
                  前往设置
                </button>
              </div>
            {:else if llmConfigError}
              <div class="max-w-md rounded-[10px] border border-danger/40 bg-danger-soft px-3.5 py-3 text-left text-[12px] text-danger-fg">
                <p class="font-medium">无法读取 LLM 配置</p>
                <p class="mt-1 leading-relaxed">暂时无法校验 API Key，请在「设置」页确认模型配置后重试。</p>
                <button
                  class="mt-3 inline-flex h-[26px] items-center rounded-[7px] border border-danger/50 bg-danger/10 px-2.5 text-[11px] font-medium text-danger-fg transition-colors hover:bg-danger/20"
                  onclick={goToSettings}
                >
                  前往设置
                </button>
              </div>
            {/if}
            <button
              class="ui-btn ui-btn-primary h-8 px-4"
              onclick={() => generate(false)}
              disabled={actionLoading || needsApiKey || llmConfigUnavailable}
              title={needsApiKey || llmConfigError ? "请先在设置中配置 LLM API Key" : ""}
            >
              {actionLoading ? "正在创建任务…" : needsApiKey || llmConfigError ? "请先配置 LLM API Key" : llmConfigLoading ? "检测配置中…" : "生成 Summary"}
            </button>
          </div>
        {/if}

        {#if error}
          <div class="mt-4 rounded-[8px] border border-danger/30 bg-danger-soft p-3 text-[12px] text-danger-fg">{error}</div>
        {/if}
      </div>

      {#if summary?.status === "success"}
        <div class="ui-modal-footer">
          <button class="ui-btn" onclick={close}>关闭</button>
          <button class="ui-btn ui-btn-primary" onclick={() => generate(true)} disabled={actionLoading}>
            重新生成
          </button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  :global(.summary-markdown) {
    line-height: 1.75;
    overflow-wrap: anywhere;
  }

  :global(.summary-markdown h1) {
    margin: 0 0 0.85rem;
    padding-bottom: 0.55rem;
    border-bottom: 1px solid rgb(var(--c-line));
    color: rgb(var(--c-fg));
    font-size: 17px;
    line-height: 1.45;
    font-weight: 650;
  }

  :global(.summary-markdown h2) {
    margin: 1.5rem 0 0.6rem;
    color: rgb(var(--c-fg));
    font-size: 15px;
    line-height: 1.5;
    font-weight: 650;
  }

  :global(.summary-markdown h3) {
    margin: 1.1rem 0 0.45rem;
    color: rgb(var(--c-fg));
    font-size: 13px;
    line-height: 1.5;
    font-weight: 650;
  }

  :global(.summary-markdown p) {
    margin: 0.6rem 0;
  }

  :global(.summary-markdown ul),
  :global(.summary-markdown ol) {
    margin: 0.6rem 0;
    padding-left: 1.5rem;
  }

  :global(.summary-markdown ul) {
    list-style: disc;
  }

  :global(.summary-markdown ol) {
    list-style: decimal;
  }

  :global(.summary-markdown li) {
    margin: 0.25rem 0;
  }

  :global(.summary-markdown li::marker) {
    color: rgb(var(--c-fg-faint));
  }

  :global(.summary-markdown strong) {
    color: rgb(var(--c-fg));
    font-weight: 600;
  }

  :global(.summary-markdown blockquote) {
    margin: 0.8rem 0;
    padding: 0.55rem 0.9rem;
    border-left: 3px solid rgb(var(--c-accent));
    border-radius: 0 8px 8px 0;
    background: rgb(var(--c-accent-soft) / 0.6);
    color: rgb(var(--c-fg-secondary));
  }

  :global(.summary-markdown code) {
    padding: 0.1rem 0.35rem;
    border: 1px solid rgb(var(--c-line));
    border-radius: 5px;
    background: rgb(var(--c-control));
    font-size: 0.85em;
  }

  :global(.summary-markdown pre) {
    margin: 0.8rem 0;
    padding: 0.85rem 1rem;
    overflow-x: auto;
    border: 1px solid rgb(var(--c-line));
    border-radius: 8px;
    background: rgb(var(--c-sunken));
    color: rgb(var(--c-fg));
  }

  :global(.summary-markdown pre code) {
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
  }

  :global(.summary-markdown a) {
    color: rgb(var(--c-accent-fg));
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  :global(.summary-markdown hr) {
    margin: 1.25rem 0;
    border: 0;
    border-top: 1px solid rgb(var(--c-line));
  }

  :global(.summary-markdown table) {
    width: 100%;
    margin: 0.9rem 0;
    border-collapse: collapse;
    font-size: 12px;
  }

  :global(.summary-markdown th),
  :global(.summary-markdown td) {
    padding: 0.45rem 0.7rem;
    border: 1px solid rgb(var(--c-line));
    text-align: left;
  }

  :global(.summary-markdown th) {
    background: rgb(var(--c-control));
    color: rgb(var(--c-fg));
    font-weight: 600;
  }
</style>
