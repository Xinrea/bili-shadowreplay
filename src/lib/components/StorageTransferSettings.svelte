<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import { ArrowRight, Check, CircleAlert, Loader2, X } from "lucide-svelte";
  import type { StorageConfig } from "../interface";
  import type { TaskRow } from "../db";
  import { invoke } from "../invoker";
  import { createStateRefresh, watchStateChanges } from "../state-refresh";
  import { describeTask, isActiveTaskStatus } from "../task-progress";
  import { openTaskCenter } from "../stores/tasks";
  import { trapFocus } from "../focus-trap";
  import { storageBackendEntries, storageBackendSupported, storageBackendLocation } from "../recording-storage";
  import {
    transferBytes, transferModeLabel, transferResult, transferSelectionError, transferStatus,
    type StorageTransferPlan,
  } from "../storage-transfer";

  const DISMISSED_KEY = "storage-transfer-dismissed";

  function readDismissed(): ReadonlySet<string> {
    try {
      if (typeof localStorage === "undefined") return new Set();
      const stored = JSON.parse(localStorage.getItem(DISMISSED_KEY) || "[]");
      return new Set(Array.isArray(stored) ? stored.filter((id): id is string => typeof id === "string") : []);
    } catch {
      return new Set();
    }
  }

  function prettyMetadata(raw: string): string {
    try {
      return JSON.stringify(JSON.parse(raw), null, 2);
    } catch {
      return raw;
    }
  }

  let {
    storage, disabled = false, busy = $bindable(true), starting = $bindable(false),
    sourceId = $bindable(""),
  }: {
    storage: StorageConfig;
    disabled?: boolean;
    busy?: boolean;
    starting?: boolean;
    sourceId?: string;
  } = $props();

  let targetId = $state("");
  let plan = $state<StorageTransferPlan | null>(null);
  let planKey = $state("");
  let confirmed = $state(false);
  let scanning = $state(false);
  let error = $state("");
  let tasksError = $state("");
  let tasksKnown = $state(false);
  let tasks = $state<TaskRow[]>([]);
  let disposed = false;
  let scanRequest = 0;
  let taskRevision = 0;
  let detailTask = $state<TaskRow | null>(null);
  let dismissed = $state<ReadonlySet<string>>(readDismissed());

  const selectionKey = $derived(JSON.stringify([sourceId, targetId, storage, disabled]));
  const selectionError = $derived(transferSelectionError(storage, sourceId, targetId));
  const currentPlan = $derived(planKey === selectionKey ? plan : null);
  const canStart = $derived(!disabled && !busy && !scanning && !!currentPlan?.can_transfer &&
    currentPlan.blockers.length === 0 && currentPlan.recording_count + currentPlan.cleanup_count > 0 && confirmed);
  const activeTransfer = $derived(tasks.some(task => isActiveTaskStatus(task.status)));
  const step = $derived(currentPlan?.can_transfer && currentPlan.blockers.length === 0 ? 3 : scanning || currentPlan ? 2 : 1);
  // Completed transfers live in the task center; failures stay inline until dismissed.
  const shownTasks = $derived(tasks.some(task => isActiveTaskStatus(task.status))
    ? tasks.filter(task => isActiveTaskStatus(task.status))
    : tasks.filter(task => (task.status === "failed" || task.status === "error") && !dismissed.has(task.id)));
  const taskRefresh = createStateRefresh(loadTasks);

  function invalidatePlan() {
    scanRequest++;
    plan = null;
    planKey = "";
    confirmed = false;
    scanning = false;
    error = "";
  }

  $effect(() => {
    // Invalidate even if the user changes A → B and then back before a request finishes.
    selectionKey;
    // The request counter is bookkeeping, not an input to this effect.
    untrack(invalidatePlan);
  });

  function updateBusy() {
    busy = starting || !tasksKnown || tasks.some(task => isActiveTaskStatus(task.status));
  }

  function dismissTask(id: string) {
    const ids = [...dismissed, id].slice(-50);
    dismissed = new Set(ids);
    try { localStorage.setItem(DISMISSED_KEY, JSON.stringify(ids)); } catch { /* still hidden this session */ }
    if (detailTask?.id === id) detailTask = null;
  }

  function handleDetailKeydown(event: KeyboardEvent) {
    if (!detailTask || event.key !== "Escape") return;
    event.preventDefault();
    event.stopImmediatePropagation();
    detailTask = null;
  }

  async function loadTasks() {
    const revision = taskRevision;
    try {
      const rows = await invoke<TaskRow[]>("get_tasks");
      if (disposed || revision !== taskRevision) return;
      const transfers = rows.filter(task => task.task_type === "storage_transfer");
      // A just-queued task must not disappear behind an older server snapshot.
      const ids = new Set(transfers.map(task => task.id));
      tasks = [...transfers, ...tasks.filter(task => isActiveTaskStatus(task.status) && !ids.has(task.id))]
        .sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at));
      tasksKnown = true;
      tasksError = "";
      if (tasks.some(task => isActiveTaskStatus(task.status))) invalidatePlan();
    } catch (e) {
      if (disposed || revision !== taskRevision) return;
      tasksKnown = false;
      invalidatePlan();
      tasksError = `无法确认转移任务状态：${String(e)}。请刷新任务后再操作。`;
    } finally {
      if (!disposed && revision === taskRevision) updateBusy();
    }
  }

  async function scan() {
    if (disabled || busy || scanning || starting || disposed) return;
    invalidatePlan();
    if (selectionError) { error = selectionError; return; }
    const key = selectionKey;
    const request = ++scanRequest;
    const source = sourceId;
    const target = targetId;
    scanning = true;
    try {
      const result = await invoke<StorageTransferPlan>("preview_storage_transfer", {
        sourceBackendId: source, targetBackendId: target,
      });
      if (disposed || request !== scanRequest || key !== selectionKey) return;
      if (result.source_backend_id !== source || result.target_backend_id !== target) {
        error = "检查结果与所选存储不一致，请重新检查。";
        return;
      }
      plan = result;
      planKey = key;
    } catch (e) {
      if (!disposed && request === scanRequest && key === selectionKey) {
        error = `转移检查失败：${String(e)}`;
      }
    } finally {
      if (!disposed && request === scanRequest) scanning = false;
    }
  }

  async function start() {
    if (starting || disposed || !canStart || selectionError || !currentPlan ||
        currentPlan.source_backend_id !== sourceId || currentPlan.target_backend_id !== targetId) return;
    const source = sourceId;
    const target = targetId;
    starting = true;
    taskRevision++;
    updateBusy();
    invalidatePlan();
    try {
      const task = await invoke<TaskRow>("start_storage_transfer", {
        sourceBackendId: source, targetBackendId: target,
      });
      if (disposed) return;
      tasks = [task, ...tasks.filter(row => row.id !== task.id)];
      tasksKnown = true;
      tasksError = "";
    } catch (e) {
      if (disposed) return;
      // A lost response may still have queued work. Recover tasks before allowing a retry.
      tasksKnown = false;
      error = `转移提交未确认成功：${String(e)}。请核实任务状态，再重新检查。`;
    } finally {
      if (!disposed) {
        taskRevision++;
        starting = false;
        updateBusy();
        taskRefresh.schedule();
      }
    }
  }

  async function retryTask(task: TaskRow) {
    if (disabled || busy || starting) return;
    try {
      const metadata = JSON.parse(task.metadata);
      if (typeof metadata.source_backend_id !== "string" || typeof metadata.target_backend_id !== "string") {
        throw new Error("任务缺少来源或目标存储");
      }
      sourceId = metadata.source_backend_id;
      targetId = metadata.target_backend_id;
      invalidatePlan();
      await tick();
      await scan();
    } catch (e) {
      error = `无法重试转移：${String(e)}`;
    }
  }

  onMount(() => {
    const stop = watchStateChanges(taskRefresh, ["tasks-changed"], 2000);
    return () => {
      disposed = true;
      scanRequest++;
      taskRevision++;
      stop();
    };
  });
</script>

<svelte:window onkeydowncapture={handleDetailKeydown} />

<section class="space-y-4" aria-label="录播转移">
  {#if !tasksKnown && !tasksError}
    <p class="flex items-center gap-2 py-6 text-[12px] text-fg-muted" role="status"><Loader2 class="h-4 w-4 animate-spin motion-reduce:animate-none" />正在检查转移任务…</p>
  {/if}
  {#if tasksError}
    <div class="space-y-2 rounded-[8px] bg-danger-soft p-3">
      <p class="whitespace-pre-wrap text-[12px] text-danger-fg" role="alert">{tasksError}</p>
      <button class="ui-btn ui-btn-sm" type="button" disabled={starting} onclick={() => taskRefresh.run()}>重新检查任务</button>
    </div>
  {/if}
  {#each shownTasks as task (task.id)}
    {@const result = transferResult(task)}
    {@const failed = task.status === "failed" || task.status === "error"}
    <div class="relative rounded-[8px] border border-line bg-sunken/40 p-4" aria-live="polite">
      {#if !isActiveTaskStatus(task.status)}
        <button type="button" class="absolute right-2 top-2 inline-flex h-6 w-6 items-center justify-center rounded-[6px] text-fg-muted transition-colors hover:bg-control-hover hover:text-fg"
          title="忽略这次转移结果" aria-label={`忽略转移结果：${describeTask(task) || task.id}`} onclick={() => dismissTask(task.id)}>
          <X class="h-3.5 w-3.5" />
        </button>
      {/if}
      <div class="flex items-start gap-3" class:pr-7={!isActiveTaskStatus(task.status)}>
        {#if isActiveTaskStatus(task.status)}<Loader2 class="mt-0.5 h-4 w-4 shrink-0 animate-spin text-accent-fg motion-reduce:animate-none" />
        {:else if failed}<CircleAlert class="mt-0.5 h-4 w-4 shrink-0 text-warning-fg" />
        {:else}<Check class="mt-0.5 h-4 w-4 shrink-0 text-success-fg" />{/if}
        <div class="min-w-0 flex-1 space-y-2">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <p class="break-all text-[13px] font-medium text-fg">{describeTask(task)}</p>
            <span class="text-[11px] text-fg-muted">{transferStatus(task.status)}</span>
          </div>
          {#if task.message}<p class="text-[12px] leading-relaxed text-fg-secondary whitespace-pre-wrap break-words">{task.message}</p>{/if}
          {#if result}
            <p class="text-[12px] text-fg-muted">已转移 {result.transferred_recordings} 条，已清理来源 {result.cleaned_recordings} 条。</p>
            {#if result.failed_recording || result.error}
              <details class="text-[12px] text-fg-muted">
                <summary class="cursor-pointer">查看失败详情</summary>
                {#if result.failed_recording}<p class="mt-2 break-all">停止于：{result.failed_recording}</p>{/if}
                {#if result.error}<p class="mt-1 whitespace-pre-wrap break-words text-danger-fg">{result.error}</p>{/if}
              </details>
            {/if}
          {/if}
          {#if isActiveTaskStatus(task.status)}
            <p class="ui-hint">可以关闭窗口，转移会在后台继续。期间暂不能修改存储配置。</p>
          {:else if failed}
            <p class="ui-hint">已完成的录播不会回滚。解决问题后可重新检查，继续转移或清理来源。</p>
          {/if}
          <div class="flex flex-wrap gap-2">
            {#if failed}<button class="ui-btn ui-btn-sm" type="button" disabled={disabled || busy || scanning} onclick={() => retryTask(task)}>重新检查这次转移</button>{/if}
            {#if isActiveTaskStatus(task.status)}
              <button class="ui-btn ui-btn-ghost ui-btn-sm" type="button" onclick={() => openTaskCenter(task.id)}>在任务中心查看</button>
            {:else}
              <button class="ui-btn ui-btn-ghost ui-btn-sm" type="button" onclick={() => detailTask = task}>任务详情</button>
            {/if}
          </div>
        </div>
      </div>
    </div>
  {/each}
  {#if tasksKnown && !activeTransfer}
    <ol class="flex flex-wrap items-center gap-x-5 gap-y-2 border-b border-line pb-3 text-[12px]" aria-label="转移步骤">
      {#each ["选择位置", "检查空间", "确认转移"] as label, index}
        <li class="flex items-center gap-2" class:text-fg={step === index + 1} class:text-fg-muted={step !== index + 1} aria-current={step === index + 1 ? "step" : undefined}>
          <span class="flex h-5 w-5 items-center justify-center rounded-full bg-control text-[11px] tabular-nums" class:!bg-accent-soft={step === index + 1} class:!text-accent-fg={step === index + 1}>
            {#if step > index + 1}<Check class="h-3 w-3" />{:else}{index + 1}{/if}
          </span>
          {label}
        </li>
      {/each}
    </ol>
    <fieldset disabled={disabled || busy || starting} class="min-w-0 space-y-4">
      <div class="transfer-route">
        <div class="min-w-0">
          <label for="transfer-source" class="ui-label mb-1.5 block">从哪里转出</label>
          <select id="transfer-source" class="ui-input w-full" bind:value={sourceId}>
            <option value="">选择来源存储</option>
            {#each storageBackendEntries(storage) as [id, backend] (id)}<option value={id} disabled={!storageBackendSupported(backend)}>{id}</option>{/each}
          </select>
          {#if storage.backends[sourceId]}<p class="mt-1.5 select-text break-all text-[11px] leading-relaxed text-fg-muted">{storageBackendLocation(storage.backends[sourceId])}</p>{/if}
        </div>
        <ArrowRight class="route-arrow mt-7 h-4 w-4 text-fg-faint" />
        <div class="min-w-0">
          <label for="transfer-target" class="ui-label mb-1.5 block">转移到哪里</label>
          <select id="transfer-target" class="ui-input w-full" bind:value={targetId}>
            <option value="">选择目标存储</option>
            {#each storageBackendEntries(storage) as [id, backend] (id)}<option value={id} disabled={!storageBackendSupported(backend) || id === sourceId}>{id}</option>{/each}
          </select>
          {#if storage.backends[targetId]}<p class="mt-1.5 select-text break-all text-[11px] leading-relaxed text-fg-muted">{storageBackendLocation(storage.backends[targetId])}</p>{/if}
        </div>
      </div>
      {#if sourceId && targetId && selectionError}<p class="text-[12px] text-danger-fg" role="alert">{selectionError}</p>{/if}
      <div class="flex flex-wrap items-center justify-between gap-3">
        <p class="text-[11px] text-fg-muted">按数据库记录估算大小，不扫描录播文件。</p>
        <button class="ui-btn" class:ui-btn-primary={!currentPlan} type="button" disabled={scanning || !!selectionError} onclick={scan}>
          {#if scanning}<Loader2 class="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" />正在检查…{:else}{currentPlan ? "重新检查" : "检查空间"}{/if}
        </button>
      </div>
      {#if currentPlan}
        <div class="space-y-3 border-t border-line pt-4" aria-live="polite">
          <p class="text-[12px] text-fg-secondary">转移方式：{transferModeLabel(currentPlan.transfer_mode)}</p>
          <dl class="grid grid-cols-2 gap-x-4 gap-y-3 rounded-[8px] bg-sunken/50 p-3 text-[12px]">
            <div><dt class="text-fg-muted">待转移录播（估算）</dt><dd class="mt-1 font-medium tabular-nums">{currentPlan.recording_count} 条 <span class="font-normal text-fg-muted">/ {transferBytes(currentPlan.total_bytes)}</span></dd></div>
            <div><dt class="text-fg-muted">待补清理</dt><dd class="mt-1 font-medium tabular-nums">{currentPlan.cleanup_count} 条</dd></div>
            <div><dt class="text-fg-muted">预计所需空间</dt><dd class="mt-1 font-medium tabular-nums">{transferBytes(currentPlan.required_bytes)}</dd></div>
            <div><dt class="text-fg-muted">目标可用空间</dt><dd class="mt-1 font-medium tabular-nums">{currentPlan.available_bytes === null ? "未知（S3 不提供容量查询）" : transferBytes(currentPlan.available_bytes)}</dd></div>
          </dl>
          {#if !currentPlan.can_transfer || currentPlan.blockers.length}
            <div class="rounded-[8px] bg-warning-soft p-3 text-[12px] text-warning-fg">
              <p class="font-medium">暂不能转移，请处理后重新检查</p>
              <ul class="mt-1.5 list-inside list-disc space-y-1">
                {#each currentPlan.blockers as blocker}<li class="whitespace-pre-wrap break-words">{blocker}</li>{/each}
              </ul>
            </div>
          {:else if currentPlan.recording_count + currentPlan.cleanup_count === 0}
            <p class="text-[12px] text-fg-muted">没有需要转移或清理的录播。</p>
          {:else}
            {#if currentPlan.available_bytes === null}
              <p class="text-[12px] text-fg-muted">无法预估远端剩余容量；复制失败时不切换归属或清理来源，可使用相同来源和目标重试。</p>
            {:else if currentPlan.required_bytes > currentPlan.available_bytes}
              <p class="flex items-start gap-1.5 text-[12px] text-warning-fg"><CircleAlert class="mt-0.5 h-3.5 w-3.5 shrink-0" />预计空间不足；重试会复用已完成的复制，实际写入空间不足则停止。</p>
            {:else if currentPlan.transfer_mode === "rename"}
              <p class="flex items-center gap-1.5 text-[12px] text-success-fg"><Check class="h-3.5 w-3.5" />无需预留完整副本空间</p>
            {:else}
              <p class="flex items-center gap-1.5 text-[12px] text-success-fg"><Check class="h-3.5 w-3.5" />预计空间足够</p>
            {/if}
            {#if currentPlan.transfer_mode === "object"}
              <p class="text-[11px] leading-relaxed text-fg-muted">通过应用所在机器逐对象中转，系统临时目录需容纳单个最大对象。上传至 S3 的单个对象上限为 5 GiB。重试会复用已校验对象，不覆盖已有的陌生文件。</p>
            {:else if currentPlan.transfer_mode === "rename"}
              <p class="text-[11px] leading-relaxed text-fg-muted">直接移动录播目录，不复制媒体数据。若遇到跨文件系统错误，会按 DB 大小估算复制空间；其他错误会停止转移。</p>
            {:else}
              <p class="text-[11px] leading-relaxed text-fg-muted">空间只按 DB 大小估算，不预先详细统计文件。估算可能不包含全部附属文件；实际写入空间不足时停止当前录播，已完成的转移保留。</p>
            {/if}
            <label class="flex items-start gap-2 text-[12px] leading-relaxed text-fg-secondary">
              <input class="ui-checkbox mt-0.5 shrink-0" type="checkbox" bind:checked={confirmed} />
              <span>确认转移 {sourceId} 中的录播到 {targetId}，并更新录播的存储位置。完成后来源不再保留这些文件。开始后不可取消。</span>
            </label>
            <div class="flex justify-end">
              <button class="ui-btn ui-btn-primary" type="button" disabled={!canStart} onclick={start}>{starting ? "正在提交…" : "开始转移"}</button>
            </div>
          {/if}
        </div>
      {/if}
    </fieldset>
    <details class="border-t border-line pt-3 text-[11px] text-fg-muted">
      <summary class="cursor-pointer hover:text-fg">转移范围与限制</summary>
      <ul class="mt-2 list-inside list-disc space-y-1.5 leading-relaxed">
        <li>支持本地与 S3 双向转移、S3 与 S3 转移。本地根目录须已存在；S3 bucket 须预先创建并配置读写及删除权限。</li>
        <li>来源和目标不能相同或重叠；本地目录不能经过符号链接，同一 S3 bucket 的前缀不能互相包含。转移期间不要通过其他应用修改这些录播对象。</li>
        <li>同一文件系统内优先直接移动目录；跨文件系统则逐条完整复制、校验并切换录播绑定后，才清理来源文件。</li>
        <li>转移前会检查占用状态；只有明确的跨文件系统错误才回退到复制，空间判断仍按 DB 估算。权限或目标冲突等错误不会触发复制回退。</li>
        <li>复制逐场校验，成功后才切换绑定并清理来源。空间不足等写入失败会停止当前录播，可使用相同来源和目标重试。</li>
        <li>默认存储和直播间偏好不变，录播范围外的文件及 output 素材不会移动。</li>
      </ul>
    </details>
  {/if}
  {#if starting}<p class="ui-hint" role="status">正在提交转移，请稍候…</p>{/if}
  {#if error}<p class="whitespace-pre-wrap text-[12px] text-danger-fg" role="alert">{error}</p>{/if}
</section>

{#if detailTask}
  {@const detailResult = transferResult(detailTask)}
  <div class="fixed inset-0 z-[1300] flex items-center justify-center p-4">
    <button type="button" class="absolute inset-0 cursor-default bg-black/50 backdrop-blur-sm" aria-label="关闭任务详情" onclick={() => detailTask = null}></button>
    <div role="dialog" aria-modal="true" aria-labelledby="transfer-task-detail-title" tabindex="-1" use:trapFocus
      class="relative flex max-h-[min(560px,85vh)] w-[520px] max-w-full flex-col overflow-hidden rounded-[12px] border border-line bg-surface text-fg shadow-2xl">
      <div class="flex h-[46px] shrink-0 items-center justify-between gap-3 border-b border-line px-[18px]">
        <h3 id="transfer-task-detail-title" class="text-[14px] font-semibold">转移任务详情</h3>
        <button class="ui-btn ui-btn-ghost ui-btn-sm ui-btn-icon" type="button" aria-label="关闭任务详情" onclick={() => detailTask = null}><X class="h-4 w-4" /></button>
      </div>
      <div class="ui-modal-body custom-scrollbar-light space-y-4">
        <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-[12px]">
          <dt class="text-fg-muted">路线</dt><dd class="select-text break-all text-fg">{describeTask(detailTask) || "—"}</dd>
          <dt class="text-fg-muted">状态</dt><dd class="text-fg">{transferStatus(detailTask.status)}</dd>
          <dt class="text-fg-muted">创建时间</dt><dd class="tabular-nums text-fg">{new Date(detailTask.created_at).toLocaleString()}</dd>
          <dt class="text-fg-muted">任务 ID</dt><dd class="select-text break-all font-mono text-[11px] text-fg-secondary">{detailTask.id}</dd>
        </dl>
        {#if detailTask.message}
          <div>
            <p class="text-[11px] text-fg-muted">消息</p>
            <p class="mt-1 select-text whitespace-pre-wrap break-words text-[12px] leading-relaxed text-fg-secondary">{detailTask.message}</p>
          </div>
        {/if}
        {#if detailResult}
          <div>
            <p class="text-[11px] text-fg-muted">结果</p>
            <dl class="mt-1 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-[12px]">
              <dt class="text-fg-muted">已转移录播</dt><dd class="tabular-nums text-fg-secondary">{detailResult.transferred_recordings} 条</dd>
              <dt class="text-fg-muted">已清理来源</dt><dd class="tabular-nums text-fg-secondary">{detailResult.cleaned_recordings} 条</dd>
              {#if detailResult.failed_recording}<dt class="text-fg-muted">停止于</dt><dd class="select-text break-all text-fg-secondary">{detailResult.failed_recording}</dd>{/if}
            </dl>
            {#if detailResult.error}<p class="mt-1 select-text whitespace-pre-wrap break-words text-[12px] text-danger-fg">{detailResult.error}</p>{/if}
          </div>
        {/if}
        <details>
          <summary class="cursor-pointer text-[11px] text-fg-muted hover:text-fg">原始元数据</summary>
          <pre class="custom-scrollbar-light mt-2 max-h-48 overflow-auto rounded-[6px] bg-sunken p-2 font-mono text-[11px] leading-relaxed text-fg-secondary">{prettyMetadata(detailTask.metadata)}</pre>
        </details>
      </div>
      <div class="ui-modal-footer"><button class="ui-btn" type="button" onclick={() => detailTask = null}>关闭</button></div>
    </div>
  </div>
{/if}

<style>
  .transfer-route { display: grid; grid-template-columns: minmax(0, 1fr) 16px minmax(0, 1fr); gap: 16px; }
  @media (max-width: 640px) {
    .transfer-route { grid-template-columns: minmax(0, 1fr); gap: 12px; }
    :global(.route-arrow) { display: none; }
  }
</style>
