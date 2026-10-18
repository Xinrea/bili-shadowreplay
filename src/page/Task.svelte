<script lang="ts">
  import { createStateRefresh, watchStateChanges } from "../lib/state-refresh";
  import { invoke } from "../lib/invoker";
  import { fade } from "svelte/transition";
  import {
    Clock,
    CheckCircle,
    XCircle,
    AlertCircle,
    Loader2,
    RefreshCw,
    ChevronDown,
    X,
  } from "lucide-svelte";
  import type { TaskRow } from "../lib/db";
  import { taskTypeName } from "../lib/task-progress";
  import PageTitlebar from "../lib/components/PageTitlebar.svelte";
  import { onMount } from "svelte";
  import { registerCommands } from "../lib/stores/commands";
  import { SvelteSet } from "svelte/reactivity";

  let tasks: TaskRow[] = $state([]);
  let loading = $state(true);
  let actionTaskId: string | null = $state(null);
  let expandedTasks = new SvelteSet<string>();

  const stateRefresh = createStateRefresh(fetch_tasks);

  async function update_tasks() {
    loading = true;
    await stateRefresh.run();
  }

  async function fetch_tasks() {
    try {
      tasks = await invoke("get_tasks");
      // 按创建时间倒序排列
      tasks.sort(
        (a, b) =>
          new Date(b.created_at).getTime() - new Date(a.created_at).getTime(),
      );
    } catch (error) {
      console.error("获取任务列表失败:", error);
    } finally {
      loading = false;
    }
  }

  async function delete_task(id: string) {
    try {
      actionTaskId = id;
      await invoke("delete_task", { id });
      await update_tasks();
    } catch (error) {
      console.error("删除任务失败:", error);
      alert("删除任务失败：" + error);
    } finally {
      actionTaskId = null;
    }
  }

  async function cancel_task(id: string) {
    try {
      actionTaskId = id;
      await invoke("cancel", { eventId: id });
      await update_tasks();
    } catch (error) {
      console.error("取消任务失败:", error);
      alert("取消任务失败：" + error);
    } finally {
      actionTaskId = null;
    }
  }

  function get_status_icon(status: string) {
    switch (status.toLowerCase()) {
      case "completed":
      case "success":
        return CheckCircle;
      case "failed":
      case "error":
        return XCircle;
      case "running":
      case "processing":
        return Loader2;
      case "pending":
      case "waiting":
        return Clock;
      default:
        return AlertCircle;
    }
  }

  function get_status_color(status: string) {
    switch (status.toLowerCase()) {
      case "completed":
      case "success":
        return "text-success-fg";
      case "failed":
      case "error":
        return "text-danger-fg";
      case "running":
      case "processing":
        return "text-accent-fg";
      case "pending":
      case "waiting":
        return "text-warning-fg";
      default:
        return "text-fg-muted";
    }
  }

  function get_status_bg_color(status: string) {
    switch (status.toLowerCase()) {
      case "completed":
      case "success":
        return "bg-success-soft";
      case "failed":
      case "error":
        return "bg-danger-soft";
      case "running":
      case "processing":
        return "bg-accent-soft";
      case "pending":
      case "waiting":
        return "bg-warning-soft";
      default:
        return "bg-control";
    }
  }

  function get_status_name(status: string) {
    switch (status.toLowerCase()) {
      case "completed":
      case "success":
        return "已完成";
      case "failed":
      case "error":
        return "失败";
      case "running":
      case "processing":
        return "进行中";
      case "pending":
      case "waiting":
        return "等待中";
      default:
        return status;
    }
  }

  function count_tasks(...statuses: string[]) {
    return tasks.filter((task) =>
      statuses.includes(task.status.toLowerCase()),
    ).length;
  }

  function is_cancelable_status(status: string) {
    const normalized = status.toLowerCase();
    return normalized === "pending" || normalized === "processing";
  }

  function format_date(date_str: string) {
    const date = new Date(date_str);
    return date.toLocaleString("zh-CN", {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  }

  type JsonTokenType =
    | "key"
    | "string"
    | "number"
    | "boolean"
    | "null"
    | "punctuation"
    | "plain";

  type JsonToken = {
    text: string;
    type: JsonTokenType;
  };

  function tokenize_json(metadata: string): JsonToken[] {
    let formatted: string;
    try {
      formatted = JSON.stringify(JSON.parse(metadata), null, 2) ?? metadata;
    } catch {
      return [{ text: metadata, type: "plain" }];
    }

    const tokens: JsonToken[] = [];
    const pattern =
      /("(?:\\.|[^"\\])*")(?=\s*:)|"(?:\\.|[^"\\])*"|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?|\b(?:true|false|null)\b|[{}[\],:]|\s+/g;
    let lastIndex = 0;
    let match: RegExpExecArray | null;

    while ((match = pattern.exec(formatted)) !== null) {
      if (match.index > lastIndex) {
        tokens.push({
          text: formatted.slice(lastIndex, match.index),
          type: "plain",
        });
      }

      const text = match[0];
      let type: JsonTokenType = "plain";
      if (match[1]) {
        type = "key";
      } else if (text.startsWith('"')) {
        type = "string";
      } else if (/^-?\d/.test(text)) {
        type = "number";
      } else if (text === "true" || text === "false") {
        type = "boolean";
      } else if (text === "null") {
        type = "null";
      } else if (!/^\s+$/.test(text)) {
        type = "punctuation";
      }

      tokens.push({ text, type });
      lastIndex = pattern.lastIndex;
    }

    if (lastIndex < formatted.length) {
      tokens.push({ text: formatted.slice(lastIndex), type: "plain" });
    }
    return tokens;
  }

  function get_json_token_color(type: JsonTokenType) {
    switch (type) {
      case "key":
        return "text-violet-500";
      case "string":
        return "text-success-fg";
      case "number":
        return "text-accent-fg";
      case "boolean":
        return "text-warning-fg";
      case "null":
        return "text-fg-faint";
      case "punctuation":
        return "text-fg-muted";
      default:
        return "text-fg-secondary";
    }
  }

  function get_task_type_color(task_type: string) {
    switch (task_type.toLowerCase()) {
      case "clip_range":
        return "bg-purple-500";
      case "upload":
      case "upload_procedure":
        return "bg-success";
      case "generate_video_subtitle":
        return "bg-accent";
      case "encode_video_subtitle":
        return "bg-warning";
      case "generate_archive_summary":
        return "bg-violet-500";
      default:
        return "bg-fg-faint";
    }
  }

  function toggleMetadata(taskId: string) {
    if (expandedTasks.has(taskId)) {
      expandedTasks.delete(taskId);
    } else {
      expandedTasks.add(taskId);
    }
  }

  onMount(() => watchStateChanges(stateRefresh, ["tasks-changed"]));

  onMount(() =>
    registerCommands([
      {
        id: "task:refresh",
        group: "action",
        title: "刷新任务列表",
        detail: "任务",
        icon: RefreshCw,
        keywords: ["refresh tasks", "reload"],
        page: "任务",
        run: update_tasks,
      },
    ]),
  );
</script>

<PageTitlebar title="任务列表">
  {#snippet subtitle()}
    <span class="truncate text-[12px] text-fg-muted">
      查看后台任务的执行状态、消息和详细信息。
    </span>
  {/snippet}
  <button
    class="titlebar-icon-button"
    title="刷新任务列表"
    aria-label="刷新任务列表"
    onclick={update_tasks}
    disabled={loading}
  >
    <RefreshCw class="h-4 w-4 {loading ? 'animate-spin' : ''}" />
  </button>
</PageTitlebar>

<div class="flex-1 overflow-auto bg-canvas p-6 custom-scrollbar-light">
  <div class="space-y-4">
    <!-- Task summary -->
    <div class="ui-card px-4 py-3">
      <div class="flex flex-wrap items-center gap-x-5 gap-y-2">
        <div class="flex items-center gap-2">
          <span class="text-[12px] text-fg-muted">全部任务</span>
          <span class="text-[12px] font-semibold text-accent-fg tabular-nums">
            {tasks.length}
          </span>
        </div>
        <div class="h-3.5 w-px bg-line-strong"></div>
        <div class="flex items-center gap-2">
          <span class="h-1.5 w-1.5 rounded-full bg-accent"></span>
          <span class="text-[12px] text-fg-secondary tabular-nums">
            进行中 {count_tasks("running", "processing")}
          </span>
        </div>
        <div class="flex items-center gap-2">
          <span class="h-1.5 w-1.5 rounded-full bg-warning"></span>
          <span class="text-[12px] text-fg-secondary tabular-nums">
            等待中 {count_tasks("pending", "waiting")}
          </span>
        </div>
        <div class="flex items-center gap-2">
          <span class="h-1.5 w-1.5 rounded-full bg-danger"></span>
          <span class="text-[12px] text-fg-secondary tabular-nums">
            失败 {count_tasks("failed", "error")}
          </span>
        </div>
      </div>
    </div>

    <!-- Task List -->
    <div class="ui-card overflow-hidden">
      {#if loading && tasks.length === 0}
        <div
          class="flex flex-col items-center justify-center space-y-3 p-12 text-[12px] text-fg-muted"
        >
          <RefreshCw class="h-6 w-6 animate-spin text-fg-faint" />
          <span>加载中...</span>
        </div>
      {:else if tasks.length === 0}
        <div
          class="flex flex-col items-center justify-center space-y-3 p-12 text-fg-muted"
        >
          <Clock class="h-10 w-10 text-fg-faint" />
          <h3 class="text-[14px] font-medium text-fg">
            暂无任务
          </h3>
          <p class="text-[12px]">当前没有任务记录</p>
        </div>
      {:else}
        <div class="overflow-x-auto custom-scrollbar-light">
          <div
            class="grid min-w-[820px] grid-cols-[minmax(150px,1.1fr)_minmax(240px,2fr)_110px_170px_68px] items-center gap-3 border-b border-line bg-chrome/60 px-4 py-2 text-[11px] font-medium text-fg-muted"
          >
            <span>任务</span>
            <span>消息 / ID</span>
            <span>状态</span>
            <span>创建时间</span>
            <span class="text-right">操作</span>
          </div>

          <div class="min-w-[820px] divide-y divide-line">
            {#each tasks as task (task.id)}
              <div
                class="group transition-colors hover:bg-control-hover/50"
                in:fade={{ duration: 120 }}
                out:fade={{ duration: 120 }}
              >
                <div
                  class="grid min-h-[48px] grid-cols-[minmax(150px,1.1fr)_minmax(240px,2fr)_110px_170px_68px] items-center gap-3 px-4"
                >
                  <div class="flex min-w-0 items-center gap-2">
                    <button
                      class="-ml-1 shrink-0 rounded-[6px] p-1 text-fg-faint transition-colors hover:bg-control-hover hover:text-fg"
                      onclick={() => toggleMetadata(task.id)}
                      title={expandedTasks.has(task.id)
                        ? "收起任务详情"
                        : "展开任务详情"}
                      aria-label={expandedTasks.has(task.id)
                        ? "收起任务详情"
                        : "展开任务详情"}
                      aria-expanded={expandedTasks.has(task.id)}
                    >
                      <ChevronDown
                        class="w-3.5 h-3.5 transition-transform {expandedTasks.has(
                          task.id,
                        )
                          ? 'rotate-180'
                          : ''}"
                      />
                    </button>
                    <span
                      class="h-2 w-2 shrink-0 {get_task_type_color(
                        task.task_type,
                      )} rounded-full"
                    ></span>
                    <span
                      class="truncate text-[13px] font-medium text-fg"
                      title={taskTypeName(task.task_type)}
                    >
                      {taskTypeName(task.task_type)}
                    </span>
                  </div>

                  <div class="flex min-w-0 items-center gap-2">
                    <span
                      class="truncate text-[12px] {task.message
                        ? 'text-fg-secondary'
                        : 'text-fg-faint'}"
                      title={task.message || "无任务消息"}
                    >
                      {task.message || "无任务消息"}
                    </span>
                    <span
                      class="max-w-[92px] shrink-0 truncate font-mono text-[10px] text-fg-faint"
                      title={task.id}
                    >{task.id}</span>
                  </div>

                  <div>
                    <span
                      class="ui-badge text-[11px] {get_status_bg_color(
                        task.status,
                      )} {get_status_color(task.status)}"
                      title={task.status}
                    >
                      {#if task.status.toLowerCase() === "pending" || task.status.toLowerCase() === "processing"}
                        <Loader2 class="w-3 h-3 animate-spin" />
                      {:else}
                        {@const SvelteComponent = get_status_icon(task.status)}
                        <SvelteComponent
                          class="w-3 h-3"
                        />
                      {/if}
                      <span>{get_status_name(task.status)}</span>
                    </span>
                  </div>

                  <time
                    class="text-[11px] tabular-nums text-fg-muted"
                    datetime={task.created_at}
                  >
                    {format_date(task.created_at)}
                  </time>

                  <div class="flex justify-end">
                    <button
                      class={`flex h-7 min-w-[28px] items-center justify-center gap-1 rounded-[7px] px-1.5 transition-colors ${
                        is_cancelable_status(task.status)
                          ? "text-warning-fg hover:bg-warning-soft"
                          : "text-fg-faint hover:bg-danger-soft hover:text-danger-fg"
                      } ${actionTaskId === task.id ? "cursor-wait" : ""}`}
                      onclick={() =>
                        is_cancelable_status(task.status)
                          ? cancel_task(task.id)
                          : delete_task(task.id)}
                      disabled={actionTaskId === task.id}
                      title={is_cancelable_status(task.status)
                        ? "取消任务"
                        : "删除任务"}
                    >
                      {#if actionTaskId === task.id}
                        <Loader2 class="w-3.5 h-3.5 animate-spin" />
                      {:else if is_cancelable_status(task.status)}
                        <XCircle class="w-3.5 h-3.5" />
                        <span class="text-[11px] font-medium">取消</span>
                      {:else}
                        <X class="w-3.5 h-3.5" />
                      {/if}
                    </button>
                  </div>
                </div>

                {#if expandedTasks.has(task.id)}
                  <div
                    class="px-4 pb-4"
                    in:fade={{ duration: 120 }}
                    out:fade={{ duration: 120 }}
                  >
                    <div class="space-y-4 border-t border-line pt-3">
                      <div
                        class="grid grid-cols-2 gap-x-6 gap-y-3 md:grid-cols-4"
                      >
                        <div class="min-w-0">
                          <div class="mb-1 text-[11px] text-fg-faint">
                            任务 ID
                          </div>
                          <div
                            class="select-text break-all font-mono text-[11px] text-fg-secondary"
                          >
                            {task.id}
                          </div>
                        </div>
                        <div class="min-w-0">
                          <div class="mb-1 text-[11px] text-fg-faint">
                            原始任务类型
                          </div>
                          <div
                            class="select-text break-all font-mono text-[11px] text-fg-secondary"
                          >
                            {task.task_type}
                          </div>
                        </div>
                        <div class="min-w-0">
                          <div class="mb-1 text-[11px] text-fg-faint">
                            原始状态
                          </div>
                          <div
                            class="select-text break-all font-mono text-[11px] text-fg-secondary"
                          >
                            {task.status}
                          </div>
                        </div>
                        <div class="min-w-0">
                          <div class="mb-1 text-[11px] text-fg-faint">
                            创建时间
                          </div>
                          <div class="text-[11px] tabular-nums text-fg-secondary">
                            {format_date(task.created_at)}
                          </div>
                        </div>
                      </div>

                      <div>
                        <div class="mb-1.5 text-[11px] text-fg-faint">
                          完整消息
                        </div>
                        <div
                          class="select-text whitespace-pre-wrap break-words text-[12px] leading-5 text-fg-secondary"
                        >
                          {task.message || "无任务消息"}
                        </div>
                      </div>

                      {#if task.metadata}
                        <div>
                          <div class="mb-1.5 text-[11px] text-fg-faint">
                            Metadata
                          </div>
                          <pre
                            class="select-text overflow-x-auto rounded-[8px] border border-line bg-sunken p-3 text-[11px] leading-5"
                          >{#each tokenize_json(task.metadata) as token}<span class={get_json_token_color(token.type)}>{token.text}</span>{/each}</pre>
                        </div>
                      {/if}
                    </div>
                  </div>
                {/if}
              </div>
            {/each}
          </div>
        </div>
      {/if}
    </div>
  </div>
</div>
