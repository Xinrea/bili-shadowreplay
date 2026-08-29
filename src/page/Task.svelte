<script lang="ts">
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
  import { onMount, onDestroy } from "svelte";

  let tasks: TaskRow[] = [];
  let loading = true;
  let actionTaskId: string | null = null;
  let refreshInterval = null;
  let expandedTasks = new Set<string>();

  async function update_tasks() {
    try {
      loading = true;
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
        return "text-green-600 dark:text-green-400";
      case "failed":
      case "error":
        return "text-red-600 dark:text-red-400";
      case "running":
      case "processing":
        return "text-blue-600 dark:text-blue-400";
      case "pending":
      case "waiting":
        return "text-yellow-600 dark:text-yellow-400";
      default:
        return "text-gray-600 dark:text-gray-400";
    }
  }

  function get_status_bg_color(status: string) {
    switch (status.toLowerCase()) {
      case "completed":
      case "success":
        return "bg-green-100 dark:bg-green-900/20";
      case "failed":
      case "error":
        return "bg-red-100 dark:bg-red-900/20";
      case "running":
      case "processing":
        return "bg-blue-100 dark:bg-blue-900/20";
      case "pending":
      case "waiting":
        return "bg-yellow-100 dark:bg-yellow-900/20";
      default:
        return "bg-gray-100 dark:bg-gray-900/20";
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
        return "text-purple-600 dark:text-purple-300";
      case "string":
        return "text-green-700 dark:text-green-400";
      case "number":
        return "text-blue-600 dark:text-blue-400";
      case "boolean":
        return "text-orange-600 dark:text-orange-400";
      case "null":
        return "text-gray-400 dark:text-gray-500";
      case "punctuation":
        return "text-gray-500 dark:text-gray-400";
      default:
        return "text-gray-700 dark:text-gray-300";
    }
  }

  function get_task_type_name(task_type: string) {
    switch (task_type.toLowerCase()) {
      case "clip_range":
        return "切片生成";
      case "upload_procedure":
        return "切片投稿";
      case "generate_video_subtitle":
        return "生成字幕";
      case "encode_video_subtitle":
        return "压制字幕";
      case "generate_whole_clip":
        return "生成完整录播";
      case "generate_archive_summary":
        return "录播总结";
      default:
        return task_type;
    }
  }

  function get_task_type_color(task_type: string) {
    switch (task_type.toLowerCase()) {
      case "clip_range":
        return "bg-purple-500";
      case "upload_procedure":
        return "bg-green-500";
      case "generate_video_subtitle":
        return "bg-blue-500";
      case "encode_video_subtitle":
        return "bg-orange-500";
      case "generate_archive_summary":
        return "bg-violet-500";
      default:
        return "bg-gray-500";
    }
  }

  function toggleMetadata(taskId: string) {
    if (expandedTasks.has(taskId)) {
      expandedTasks.delete(taskId);
    } else {
      expandedTasks.add(taskId);
    }
    expandedTasks = expandedTasks; // 触发响应式更新
  }

  // 设置自动刷新
  onMount(async () => {
    // 初始化时加载任务列表
    update_tasks();

    // 设置每5秒自动刷新
    refreshInterval = setInterval(() => {
      update_tasks();
    }, 5000);
  });

  // 清理定时器
  onDestroy(() => {
    if (refreshInterval) {
      clearInterval(refreshInterval);
    }
  });
</script>

<div
  class="flex-1 p-6 overflow-auto custom-scrollbar-light bg-gray-50 dark:bg-black"
>
  <div class="space-y-6">
    <!-- Header -->
    <div class="flex justify-between items-center">
      <div class="space-y-1">
        <h1 class="text-2xl font-semibold text-gray-900 dark:text-white">
          任务列表
        </h1>
        <p class="text-sm text-gray-500 dark:text-gray-400">
          查看后台任务的执行状态、消息和详细信息。
        </p>
      </div>
      <button
        on:click={update_tasks}
        class="px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors flex items-center space-x-2 disabled:opacity-50 disabled:cursor-not-allowed"
        disabled={loading}
        title="刷新任务列表"
      >
        <RefreshCw
          class="w-4 h-4 text-white {loading ? 'animate-spin' : ''}"
        />
        <span>刷新</span>
      </button>
    </div>

    <!-- Task summary -->
    <div
      class="p-4 rounded-xl bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700"
    >
      <div class="flex items-center flex-wrap gap-x-6 gap-y-2">
        <div class="flex items-center space-x-2">
          <span class="text-sm text-gray-500 dark:text-gray-400">全部任务</span>
          <span class="text-sm font-medium text-blue-600 dark:text-blue-400">
            {tasks.length}
          </span>
        </div>
        <div class="h-4 w-px bg-gray-300 dark:bg-gray-600"></div>
        <div class="flex items-center space-x-2">
          <span class="w-2 h-2 rounded-full bg-blue-500"></span>
          <span class="text-sm text-gray-600 dark:text-gray-300">
            进行中 {count_tasks("running", "processing")}
          </span>
        </div>
        <div class="flex items-center space-x-2">
          <span class="w-2 h-2 rounded-full bg-yellow-500"></span>
          <span class="text-sm text-gray-600 dark:text-gray-300">
            等待中 {count_tasks("pending", "waiting")}
          </span>
        </div>
        <div class="flex items-center space-x-2">
          <span class="w-2 h-2 rounded-full bg-red-500"></span>
          <span class="text-sm text-gray-600 dark:text-gray-300">
            失败 {count_tasks("failed", "error")}
          </span>
        </div>
      </div>
    </div>

    <!-- Task List -->
    <div
      class="bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 rounded-xl overflow-hidden"
    >
      {#if loading && tasks.length === 0}
        <div
          class="flex flex-col items-center justify-center p-12 space-y-4 text-gray-500 dark:text-gray-400"
        >
          <RefreshCw class="w-8 h-8 animate-spin" />
          <span>加载中...</span>
        </div>
      {:else if tasks.length === 0}
        <div
          class="flex flex-col items-center justify-center p-12 space-y-4 text-gray-500 dark:text-gray-400"
        >
          <Clock class="w-12 h-12" />
          <h3 class="text-lg font-medium text-gray-900 dark:text-white">
            暂无任务
          </h3>
          <p class="text-sm">当前没有任务记录</p>
        </div>
      {:else}
        <div class="overflow-x-auto custom-scrollbar-light">
          <div
            class="min-w-[820px] grid grid-cols-[minmax(150px,1.1fr)_minmax(240px,2fr)_110px_170px_68px] gap-3 items-center px-4 py-3 border-b border-gray-200 dark:border-gray-700/50 text-sm font-medium text-gray-500 dark:text-gray-400"
          >
            <span>任务</span>
            <span>消息 / ID</span>
            <span>状态</span>
            <span>创建时间</span>
            <span class="text-right">操作</span>
          </div>

          <div class="min-w-[820px] divide-y divide-gray-100 dark:divide-gray-700">
            {#each tasks as task (task.id)}
              <div
                class="group hover:bg-[#f5f5f7] dark:hover:bg-[#3a3a3c] transition-colors"
                in:fade={{ duration: 120 }}
                out:fade={{ duration: 120 }}
              >
                <div
                  class="grid grid-cols-[minmax(150px,1.1fr)_minmax(240px,2fr)_110px_170px_68px] gap-3 items-center min-h-[52px] px-4"
                >
                  <div class="flex items-center gap-2 min-w-0">
                    <button
                      class="shrink-0 p-1 -ml-1 rounded-md text-gray-400 hover:text-blue-600 dark:hover:text-blue-400 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors"
                      on:click={() => toggleMetadata(task.id)}
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
                      class="w-2 h-2 shrink-0 {get_task_type_color(
                        task.task_type,
                      )} rounded-full"
                    ></span>
                    <span
                      class="text-sm font-medium text-gray-900 dark:text-white truncate"
                      title={get_task_type_name(task.task_type)}
                    >
                      {get_task_type_name(task.task_type)}
                    </span>
                  </div>

                  <div class="min-w-0 flex items-center gap-2">
                    <span
                      class="text-sm text-gray-700 dark:text-gray-300 truncate"
                      class:text-gray-400={!task.message}
                      title={task.message || "无任务消息"}
                    >
                      {task.message || "无任务消息"}
                    </span>
                    <span
                      class="shrink-0 max-w-[92px] truncate font-mono text-[10px] text-gray-400 dark:text-gray-500"
                      title={task.id}
                    >{task.id}</span>
                  </div>

                  <div>
                    <span
                      class="inline-flex items-center gap-1 px-2 py-1 rounded-full text-xs font-medium {get_status_bg_color(
                        task.status,
                      )} {get_status_color(task.status)}"
                      title={task.status}
                    >
                      {#if task.status.toLowerCase() === "pending" || task.status.toLowerCase() === "processing"}
                        <Loader2 class="w-3 h-3 animate-spin" />
                      {:else}
                        <svelte:component
                          this={get_status_icon(task.status)}
                          class="w-3 h-3"
                        />
                      {/if}
                      <span>{get_status_name(task.status)}</span>
                    </span>
                  </div>

                  <time
                    class="text-xs tabular-nums text-gray-500 dark:text-gray-400"
                    datetime={task.created_at}
                  >
                    {format_date(task.created_at)}
                  </time>

                  <div class="flex justify-end">
                    <button
                      class={`h-7 min-w-[28px] px-1.5 rounded-md transition-colors flex justify-center items-center gap-1 ${
                        is_cancelable_status(task.status)
                          ? "text-amber-600 hover:text-amber-700 hover:bg-amber-100 dark:hover:bg-amber-900/20"
                          : "text-gray-400 hover:text-red-600 hover:bg-red-50 dark:hover:bg-red-900/20"
                      } ${actionTaskId === task.id ? "cursor-wait" : ""}`}
                      on:click={() =>
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
                    <div
                      class="border-t border-gray-200 dark:border-gray-700/50 pt-4 space-y-4"
                    >
                      <div
                        class="grid grid-cols-2 md:grid-cols-4 gap-x-6 gap-y-3"
                      >
                        <div class="min-w-0">
                          <div
                            class="mb-1 text-xs text-gray-400 dark:text-gray-500"
                          >
                            任务 ID
                          </div>
                          <div
                            class="font-mono text-xs text-gray-700 dark:text-gray-300 break-all select-text"
                          >
                            {task.id}
                          </div>
                        </div>
                        <div class="min-w-0">
                          <div
                            class="mb-1 text-xs text-gray-400 dark:text-gray-500"
                          >
                            原始任务类型
                          </div>
                          <div
                            class="font-mono text-xs text-gray-700 dark:text-gray-300 break-all select-text"
                          >
                            {task.task_type}
                          </div>
                        </div>
                        <div class="min-w-0">
                          <div
                            class="mb-1 text-xs text-gray-400 dark:text-gray-500"
                          >
                            原始状态
                          </div>
                          <div
                            class="font-mono text-xs text-gray-700 dark:text-gray-300 break-all select-text"
                          >
                            {task.status}
                          </div>
                        </div>
                        <div class="min-w-0">
                          <div
                            class="mb-1 text-xs text-gray-400 dark:text-gray-500"
                          >
                            创建时间
                          </div>
                          <div
                            class="text-xs text-gray-700 dark:text-gray-300"
                          >
                            {format_date(task.created_at)}
                          </div>
                        </div>
                      </div>

                      <div>
                        <div
                          class="mb-1.5 text-xs text-gray-400 dark:text-gray-500"
                        >
                          完整消息
                        </div>
                        <div
                          class="text-sm leading-5 text-gray-700 dark:text-gray-300 whitespace-pre-wrap break-words select-text"
                        >
                          {task.message || "无任务消息"}
                        </div>
                      </div>

                      {#if task.metadata}
                        <div>
                          <div
                            class="mb-1.5 text-xs text-gray-400 dark:text-gray-500"
                          >
                            Metadata
                          </div>
                          <pre
                            class="p-3 bg-gray-50 dark:bg-[#2c2c2e] rounded-lg border border-gray-200 dark:border-gray-700 text-xs leading-5 overflow-x-auto select-text"
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
