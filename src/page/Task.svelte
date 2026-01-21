<script lang="ts">
  import { invoke } from "../lib/invoker";
  import { scale, fade } from "svelte/transition";
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
          new Date(b.created_at).getTime() - new Date(a.created_at).getTime()
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

  function parse_metadata(metadata: string) {
    try {
      return JSON.parse(metadata);
    } catch {
      return null;
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

<div class="flex-1 p-6 overflow-auto custom-scrollbar-light bg-gray-50 dark:bg-black">
  <div class="space-y-6">
    <!-- Header -->
    <div class="flex justify-between items-center">
      <div class="flex items-center space-x-4">
        <h1 class="text-2xl font-semibold text-gray-900 dark:text-white">
          任务列表
        </h1>
        <div
          class="flex items-center space-x-2 text-sm text-gray-500 dark:text-gray-400"
        >
          <span> 共 {tasks.length} 个任务</span>
        </div>
      </div>
      <button
        on:click={update_tasks}
        class="px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors flex items-center space-x-2"
        disabled={loading}
      >
        {#if loading}
          <Loader2 class="w-5 h-5 icon-white animate-spin" />
        {:else}
          <RefreshCw class="w-5 h-5 icon-white" />
        {/if}
        <span>刷新</span>
      </button>
    </div>

    <!-- Task List -->
    <div class="space-y-4">
      {#if loading && tasks.length === 0}
        <div class="flex items-center justify-center py-12">
          <Loader2 class="w-8 h-8 text-gray-400 animate-spin" />
          <span class="ml-2 text-gray-500">加载中...</span>
        </div>
      {:else if tasks.length === 0}
        <div
          class="flex flex-col items-center justify-center py-12 text-center"
        >
          <div
            class="w-16 h-16 rounded-full bg-gray-100 dark:bg-gray-800 flex items-center justify-center mb-4"
          >
            <Clock class="w-8 h-8 text-gray-400" />
          </div>
          <h3 class="text-lg font-medium text-gray-900 dark:text-white mb-2">
            暂无任务
          </h3>
          <p class="text-gray-500 dark:text-gray-400">当前没有任务记录</p>
        </div>
      {:else}
        {#each tasks as task (task.id)}
          <div
            class="p-4 rounded-xl bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 hover:border-blue-500 dark:hover:border-blue-400 transition-colors"
            in:scale={{ duration: 150, start: 0.95 }}
            out:scale={{ duration: 150, start: 0.95 }}
          >
            <div class="flex items-start justify-between">
              <div class="flex-1 min-w-0">
                <div class="flex items-center space-x-3 mb-2">
                  <div class="flex items-center space-x-2">
                    <div
                      class="w-2 h-2 {get_task_type_color(
                        task.task_type
                      )} rounded-full"
                    ></div>
                    <span
                      class="text-sm font-medium text-gray-900 dark:text-white"
                    >
                      {get_task_type_name(task.task_type)}
                    </span>
                  </div>
                  <div class="flex items-center space-x-2">
                    <div
                      class="flex items-center space-x-1 px-2 py-1 rounded-full text-xs font-medium {get_status_bg_color(
                        task.status
                      )} {get_status_color(task.status)}"
                    >
                      {#if task.status.toLowerCase() === "pending" || task.status.toLowerCase() === "processing"}
                        <Loader2 class="w-3 h-3 animate-spin" />
                      {:else}
                        <svelte:component
                          this={get_status_icon(task.status)}
                          class="w-3 h-3"
                        />
                      {/if}
                      <span>{task.status}</span>
                    </div>
                  </div>
                </div>

                {#if task.message}
                  <div class="text-xs text-gray-400 dark:text-gray-500 my-2">
                    {task.message}
                  </div>
                {/if}

                {#if task.metadata}
                  {@const metadata = parse_metadata(task.metadata)}
                  {#if metadata}
                    <div class="mb-2">
                      <button
                        class="flex items-center space-x-1 text-xs text-gray-500 dark:text-gray-500 hover:text-gray-700 dark:hover:text-gray-300 transition-colors"
                        on:click={() => toggleMetadata(task.id)}
                      >
                        <ChevronDown
                          class="w-3 h-3 transition-transform {expandedTasks.has(
                            task.id
                          )
                            ? 'rotate-180'
                            : ''}"
                        />
                        <span>详细信息</span>
                      </button>

                      {#if expandedTasks.has(task.id)}
                        <div
                          class="mt-2 p-3 bg-gray-50 dark:bg-gray-800 rounded-lg text-xs text-gray-600 dark:text-gray-400 space-y-1"
                          in:scale={{ duration: 150, start: 0.95 }}
                          out:scale={{ duration: 150, start: 0.95 }}
                        >
                          <pre>{JSON.stringify(metadata, null, 2)}</pre>
                        </div>
                      {/if}
                    </div>
                  {/if}
                {/if}

                <div class="text-xs text-gray-400 dark:text-gray-500 mt-2">
                  创建时间: {format_date(task.created_at)}
                </div>
              </div>

              <div class="flex items-center space-x-2 ml-4">
                <button
                  class={`p-2 rounded-lg transition-colors flex items-center space-x-1 ${
                    is_cancelable_status(task.status)
                      ? "text-amber-600 hover:text-amber-700 hover:bg-amber-50 dark:hover:bg-amber-900/20"
                      : "text-gray-500 hover:text-gray-700 hover:bg-gray-100 dark:hover:bg-gray-800/60"
                  } ${actionTaskId === task.id ? "cursor-wait" : ""}`}
                  on:click={() =>
                    is_cancelable_status(task.status)
                      ? cancel_task(task.id)
                      : delete_task(task.id)}
                  disabled={actionTaskId === task.id}
                >
                  {#if actionTaskId === task.id}
                    <Loader2 class="w-4 h-4 animate-spin" />
                  {:else if is_cancelable_status(task.status)}
                    <XCircle class="w-4 h-4" />
                    <span class="text-xs font-medium">取消</span>
                  {:else}
                    <X class="w-4 h-4" />
                  {/if}
                </button>
              </div>
            </div>
          </div>
        {/each}
      {/if}
    </div>
  </div>
</div>
