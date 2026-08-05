<script lang="ts">
  import { createEventDispatcher, onDestroy } from "svelte";
  import { marked } from "marked";
  import { AlertCircle, Loader2, RefreshCw, Sparkles, X } from "lucide-svelte";
  import { invoke } from "../invoker";
  import CopyMarkdownButton from "./CopyMarkdownButton.svelte";
  import type {
    RecordItem,
    RecordSummary,
    RecordSummaryStatus,
    SummaryHighlight,
    TaskRow,
  } from "../db";

  export let showModal = false;
  export let archive: RecordItem | null = null;

  const dispatch = createEventDispatcher<{ updated: RecordSummaryStatus }>();

  let summary: RecordSummary | null = null;
  let loading = false;
  let actionLoading = false;
  let error = "";
  let loadedKey = "";
  let pollTimer: ReturnType<typeof setTimeout> | null = null;

  $: currentKey = archive
    ? `${archive.platform}:${archive.room_id}:${archive.live_id}`
    : "";
  $: if (showModal && archive && currentKey !== loadedKey) {
    loadedKey = currentKey;
    void loadSummary();
  }
  $: highlights = parseHighlights(summary?.highlights_json);
  $: summaryMarkdown = summary?.summary_markdown || "";

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
    dispatch("updated", {
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
</script>

{#if showModal && archive}
  <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6 backdrop-blur-sm">
    <div class="flex max-h-[90vh] w-full max-w-4xl flex-col overflow-hidden rounded-2xl bg-white shadow-2xl dark:bg-gray-900">
      <div class="flex items-center justify-between border-b border-gray-200 px-6 py-4 dark:border-gray-700">
        <div>
          <div class="flex items-center gap-2">
            <Sparkles class="h-5 w-5 text-violet-500" />
            <h2 class="text-lg font-semibold text-gray-900 dark:text-white">直播 Summary</h2>
            {#if summary?.status === "success"}
              <CopyMarkdownButton content={summaryMarkdown} />
            {/if}
          </div>
          <p class="mt-1 max-w-2xl truncate text-sm text-gray-500 dark:text-gray-400">
            {archive.title}
          </p>
        </div>
        <button class="rounded-lg p-2 hover:bg-gray-100 dark:hover:bg-gray-800" on:click={close}>
          <X class="h-5 w-5" />
        </button>
      </div>

      <div class="flex-1 overflow-y-auto p-6">
        {#if loading}
          <div class="flex min-h-[260px] items-center justify-center gap-3 text-gray-500">
            <Loader2 class="h-6 w-6 animate-spin" />
            <span>读取 Summary 中</span>
          </div>
        {:else if summary?.status === "processing"}
          <div class="flex min-h-[260px] flex-col items-center justify-center gap-4 text-center">
            <Loader2 class="h-10 w-10 animate-spin text-violet-500" />
            <div>
              <p class="font-medium text-gray-900 dark:text-white">{stageText(summary.stage)}</p>
              <p class="mt-1 text-sm text-gray-500">任务在后台执行，关闭窗口不会中断处理。</p>
            </div>
          </div>
        {:else if summary?.status === "success"}
          <div class="space-y-6">
            <div class="summary-markdown text-sm text-gray-800 dark:text-gray-200">
              {@html marked(summaryMarkdown)}
            </div>

            {#if highlights.length > 0}
              <div class="space-y-3">
                <h3 class="font-semibold text-gray-900 dark:text-white">精彩时间段</h3>
                {#each highlights as highlight}
                  <div class="rounded-xl border border-violet-200 bg-violet-50 p-4 dark:border-violet-800 dark:bg-violet-950/30">
                    <div class="flex items-center justify-between gap-3">
                      <span class="font-medium text-gray-900 dark:text-white">{highlight.title}</span>
                      <span class="whitespace-nowrap rounded-full bg-white px-2.5 py-1 text-xs text-violet-700 dark:bg-gray-900 dark:text-violet-300">
                        {formatTime(highlight.start_seconds)}–{formatTime(highlight.end_seconds)}
                      </span>
                    </div>
                    <p class="mt-2 text-sm text-gray-600 dark:text-gray-300">{highlight.reason}</p>
                  </div>
                {/each}
              </div>
            {/if}

            {#if summary.subtitle_srt}
              <details class="rounded-xl border border-gray-200 dark:border-gray-700">
                <summary class="cursor-pointer px-4 py-3 text-sm font-medium">查看完整字幕</summary>
                <pre class="max-h-80 overflow-auto whitespace-pre-wrap border-t border-gray-200 p-4 text-xs dark:border-gray-700">{summary.subtitle_srt}</pre>
              </details>
            {/if}

          </div>
        {:else if summary?.status === "failed"}
          <div class="flex min-h-[260px] flex-col items-center justify-center gap-4 text-center">
            <AlertCircle class="h-10 w-10 text-red-500" />
            <div>
              <p class="font-medium text-gray-900 dark:text-white">Summary 生成失败</p>
              <p class="mt-2 max-w-xl text-sm text-red-600 dark:text-red-400">{summary.error_message || "未知错误"}</p>
            </div>
            <button class="flex items-center gap-2 rounded-lg bg-violet-600 px-4 py-2 text-sm text-white hover:bg-violet-700" on:click={() => generate(false)} disabled={actionLoading}>
              <RefreshCw class="h-4 w-4 {actionLoading ? 'animate-spin' : ''}" />
              从失败阶段重试
            </button>
          </div>
        {:else}
          <div class="flex min-h-[260px] flex-col items-center justify-center gap-4 text-center">
            <Sparkles class="h-12 w-12 text-violet-500" />
            <div>
              <p class="font-medium text-gray-900 dark:text-white">尚未生成 Summary</p>
              <p class="mt-1 text-sm text-gray-500">将依次提取完整音频、生成字幕并总结直播内容。</p>
            </div>
            <button class="rounded-lg bg-violet-600 px-5 py-2.5 text-sm font-medium text-white hover:bg-violet-700 disabled:opacity-50" on:click={() => generate(false)} disabled={actionLoading}>
              {actionLoading ? "正在创建任务…" : "生成 Summary"}
            </button>
          </div>
        {/if}

        {#if error}
          <div class="mt-4 rounded-lg bg-red-50 p-3 text-sm text-red-700 dark:bg-red-950/30 dark:text-red-300">{error}</div>
        {/if}
      </div>

      {#if summary?.status === "success"}
        <div class="flex justify-end gap-3 border-t border-gray-200 px-6 py-4 dark:border-gray-700">
          <button class="rounded-lg px-4 py-2 text-sm hover:bg-gray-100 dark:hover:bg-gray-800" on:click={close}>关闭</button>
          <button class="rounded-lg bg-violet-600 px-4 py-2 text-sm text-white hover:bg-violet-700" on:click={() => generate(true)} disabled={actionLoading}>
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
    margin: 0 0 1rem;
    padding-bottom: 0.65rem;
    border-bottom: 1px solid #e5e7eb;
    font-size: 1.5rem;
    line-height: 2rem;
    font-weight: 700;
  }

  :global(.summary-markdown h2) {
    margin: 1.75rem 0 0.75rem;
    font-size: 1.2rem;
    line-height: 1.75rem;
    font-weight: 650;
  }

  :global(.summary-markdown h3) {
    margin: 1.25rem 0 0.5rem;
    font-size: 1rem;
    line-height: 1.5rem;
    font-weight: 650;
  }

  :global(.summary-markdown p) {
    margin: 0.65rem 0;
  }

  :global(.summary-markdown ul),
  :global(.summary-markdown ol) {
    margin: 0.65rem 0;
    padding-left: 1.5rem;
  }

  :global(.summary-markdown ul) {
    list-style: disc;
  }

  :global(.summary-markdown ol) {
    list-style: decimal;
  }

  :global(.summary-markdown li) {
    margin: 0.3rem 0;
  }

  :global(.summary-markdown blockquote) {
    margin: 0.85rem 0;
    padding: 0.65rem 1rem;
    border-left: 4px solid #8b5cf6;
    border-radius: 0 0.5rem 0.5rem 0;
    background: #f5f3ff;
    color: #5b21b6;
  }

  :global(.summary-markdown code) {
    padding: 0.12rem 0.35rem;
    border-radius: 0.3rem;
    background: #f3f4f6;
    font-size: 0.85em;
  }

  :global(.summary-markdown pre) {
    margin: 0.85rem 0;
    padding: 1rem;
    overflow-x: auto;
    border-radius: 0.65rem;
    background: #111827;
    color: #f9fafb;
  }

  :global(.summary-markdown pre code) {
    padding: 0;
    background: transparent;
    color: inherit;
  }

  :global(.summary-markdown a) {
    color: #7c3aed;
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  :global(.summary-markdown hr) {
    margin: 1.5rem 0;
    border: 0;
    border-top: 1px solid #e5e7eb;
  }

  :global(.summary-markdown table) {
    width: 100%;
    margin: 1rem 0;
    border-collapse: collapse;
  }

  :global(.summary-markdown th),
  :global(.summary-markdown td) {
    padding: 0.55rem 0.75rem;
    border: 1px solid #e5e7eb;
    text-align: left;
  }

  :global(.summary-markdown th) {
    background: #f9fafb;
    font-weight: 600;
  }

  :global(.dark .summary-markdown h1),
  :global(.dark .summary-markdown hr),
  :global(.dark .summary-markdown th),
  :global(.dark .summary-markdown td) {
    border-color: #374151;
  }

  :global(.dark .summary-markdown blockquote) {
    background: rgb(76 29 149 / 0.2);
    color: #ddd6fe;
  }

  :global(.dark .summary-markdown code) {
    background: #1f2937;
  }

  :global(.dark .summary-markdown pre) {
    background: #030712;
  }

  :global(.dark .summary-markdown th) {
    background: #1f2937;
  }
</style>
