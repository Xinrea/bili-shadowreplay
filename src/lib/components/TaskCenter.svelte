<script lang="ts">
  import { tick } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { CircleCheck, Loader2, X } from "lucide-svelte";
  import {
    activeTasks,
    cancelTask,
    closeTaskCenter,
    taskCenter,
  } from "../stores/tasks";
  import {
    describeTask,
    isCancellableTask,
    taskTypeName,
    type ActiveTask,
  } from "../task-progress";
  import TaskProgress from "./TaskProgress.svelte";
  import { trapFocus } from "../focus-trap";

  // Mount once per window. Open it with `openTaskCenter()` from `stores/tasks`.

  let cancelling = new SvelteSet<string>();
  let list: HTMLElement | null = $state(null);

  function placeholder(task: ActiveTask) {
    if (task.message) return task.message;
    return task.status.toLowerCase() === "pending" ? "等待开始" : "处理中";
  }

  async function cancel(id: string) {
    if (cancelling.has(id)) return;
    cancelling.add(id);
    try {
      await cancelTask(id);
    } catch (error) {
      alert("取消任务失败：" + error);
    } finally {
      cancelling.delete(id);
    }
  }

  // Handled in the capture phase: while open, the modal owns the keyboard, so
  // page shortcuts underneath (e.g. the player's Escape, which clears the
  // selected range) never see the keys. Default actions such as Tab still
  // apply, kept within the dialog by `trapFocus`.
  function handleKeydown(event: KeyboardEvent) {
    if (!$taskCenter.open) return;
    event.stopPropagation();
    if (event.key === "Escape") {
      event.preventDefault();
      closeTaskCenter();
    }
  }

  // Bring a newly started task into view; the list is newest first, but it may
  // already be scrolled.
  $effect(() => {
    const { open, highlight } = $taskCenter;
    if (!open || !highlight || !list) return;
    void tick().then(() =>
      list?.querySelector(`[data-task-id="${CSS.escape(highlight)}"]`)?.scrollIntoView({ block: "nearest" }),
    );
  });
</script>

<svelte:window onkeydowncapture={handleKeydown} />

{#if $taskCenter.open}
  <!-- Above full-window editors such as the clip preview (z-1000) and their dialogs. -->
  <div class="fixed inset-0 z-[1200] flex items-center justify-center p-4">
    <button
      type="button"
      class="absolute inset-0 cursor-default bg-black/50 backdrop-blur-sm"
      aria-label="关闭任务列表"
      onclick={closeTaskCenter}
    ></button>

    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="task-center-title"
      use:trapFocus
      class="relative flex max-h-[min(480px,80vh)] w-full max-w-md flex-col overflow-hidden rounded-[12px] border border-line bg-surface shadow-2xl"
    >
      <div class="flex items-center justify-between gap-3 border-b border-line py-2 pl-4 pr-2">
        <h3 id="task-center-title" class="flex items-center gap-2 text-[13px] font-semibold text-fg">
          进行中的任务
          {#if $activeTasks.length > 0}
            <span class="rounded-full bg-control px-1.5 text-[11px] font-medium tabular-nums text-fg-muted">
              {$activeTasks.length}
            </span>
          {/if}
        </h3>
        <button
          type="button"
          class="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-[7px] text-fg-muted transition-colors hover:bg-control-hover hover:text-fg"
          title="关闭（任务继续在后台运行）"
          aria-label="关闭"
          onclick={closeTaskCenter}
        >
          <X class="h-4 w-4" />
        </button>
      </div>

      <ul bind:this={list} class="min-h-0 flex-1 divide-y divide-line overflow-y-auto custom-scrollbar-light">
        {#each $activeTasks as task (task.id)}
          {@const highlighted = task.id === $taskCenter.highlight}
          {@const detail = describeTask(task)}
          {@const isCancelling = cancelling.has(task.id)}
          <li
            data-task-id={task.id}
            class="relative px-4 py-2 {highlighted
              ? 'highlight bg-accent/10 before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-accent'
              : ''}"
          >
            <div class="flex min-h-[1.5rem] items-center gap-2">
              <div class="flex min-w-0 flex-1 items-center gap-1.5">
                <span class="shrink-0 text-[12px] font-medium text-fg">{taskTypeName(task.task_type)}</span>
                {#if highlighted}
                  <span class="shrink-0 rounded bg-accent px-1 text-[10px] font-medium leading-4 text-white">新任务</span>
                {/if}
                {#if detail}
                  <span class="truncate text-[11px] text-fg-muted" title={detail}>{detail}</span>
                {/if}
              </div>
              {#if isCancellableTask(task.task_type)}
                <button
                  type="button"
                  class="inline-flex h-6 shrink-0 items-center gap-1 rounded-[6px] border border-line-strong px-2 text-[11px] text-fg-secondary transition-colors hover:border-danger/50 hover:bg-danger-soft hover:text-danger-fg disabled:cursor-not-allowed disabled:border-line disabled:bg-transparent disabled:text-fg-faint"
                  disabled={isCancelling}
                  onclick={() => cancel(task.id)}
                >
                  {#if isCancelling}
                    <Loader2 class="h-3 w-3 animate-spin" />
                    取消中
                  {:else}
                    取消
                  {/if}
                </button>
              {/if}
            </div>
            <TaskProgress update={task.update} placeholder={placeholder(task)} class="mt-1" />
          </li>
        {:else}
          <li class="flex items-center justify-center gap-2 py-8 text-[12px] text-fg-muted">
            <CircleCheck class="h-4 w-4 text-fg-faint" />
            当前没有进行中的任务
          </li>
        {/each}
      </ul>
    </div>
  </div>
{/if}

<style>
  .highlight {
    animation: flash 1.6s ease-out;
  }

  @keyframes flash {
    0%,
    30% {
      background-color: rgb(10 132 255 / 0.3);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .highlight {
      animation: none;
    }
  }
</style>
