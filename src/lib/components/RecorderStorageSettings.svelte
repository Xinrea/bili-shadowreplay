<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "../invoker";
  import type { Config, RecorderStatus, RecorderRow, StorageConfig } from "../interface";
  import { storageBackend, storageBackendSupported } from "../recording-storage";
  import StorageBackendSelect from "./StorageBackendSelect.svelte";

  let { status, onchanged, disabled = false }: {
    status: RecorderStatus; onchanged: () => Promise<void>; disabled?: boolean;
  } = $props();
  let storage: StorageConfig | null = $state(null);
  let choice = $state<string | null>(null);
  let loading = $state(true);
  let saving = $state(false);
  let error = $state("");
  let saved = $state("");
  const configured = $derived(status.storage_backend_id);
  const locked = $derived(disabled || loading || saving);
  $effect(() => { choice = configured; });

  async function load() {
    loading = true;
    error = "";
    try {
      storage = (await invoke<Config>("get_config")).storage;
    } catch (e) {
      error = `录制存储加载失败：${String(e)}`;
    } finally {
      loading = false;
    }
  }

  async function save() {
    if (locked || !storage) return;
    error = "";
    saved = "";
    const id = choice === null ? storage.default_backend : choice;
    if (!storageBackendSupported(storageBackend(storage, id))) {
      error = "请选择已配置的存储。不会自动改用其他存储。";
      return;
    }
    saving = true;
    try {
      await invoke<RecorderRow>("set_recorder_storage", {
        platform: status.recorder.room_info.platform,
        roomId: status.recorder.room_info.room_id,
        storageBackendId: choice,
      });
      saved = "后续录制的存储偏好已保存到数据库，已有录播仍绑定原存储。";
      try {
        await onchanged();
      } catch (e) {
        error = `偏好已保存，但刷新录制器状态失败：${String(e)}。请刷新查看。`;
      }
    } catch (e) {
      error = `录制存储保存失败：${String(e)}`;
    } finally {
      saving = false;
    }
  }

  onMount(() => { void load(); });
</script>

<section class="mt-4 space-y-2 border-t border-line pt-3" aria-labelledby="room-storage-label">
  <label id="room-storage-label" for="room-storage" class="text-[12px] font-medium text-fg-secondary">后续录制的存储偏好</label>
  <p class="ui-hint">可在录制期间修改偏好，无需重启；只影响下一场新录制。当前录制、断线续录和历史录播仍绑定各自原存储。</p>
  {#if status.recorder.recording}
    <p class="ui-hint">本场录播绑定的存储：{status.current_record?.storage_backend_id ?? "等待录播记录更新"}</p>
  {/if}
  {#if loading}
    <p class="ui-hint" role="status">正在加载录制存储…</p>
  {:else if storage}
    <div class="flex flex-wrap items-start gap-2">
      <div class="min-w-0 flex-1">
        <StorageBackendSelect {storage} id="room-storage" bind:value={choice} disabled={locked} />
      </div>
      <button class="ui-btn" disabled={locked || choice === configured} onclick={save}>{saving ? "保存中…" : "保存"}</button>
    </div>
  {/if}
  {#if status.recorder.recording}<p class="ui-hint">当前正在录制，保存偏好不会切换本场录播的存储。</p>{/if}
  {#if error}
    <p class="text-[12px] text-danger-fg whitespace-pre-wrap" role="alert">{error}</p>
    {#if !storage}<button class="ui-btn ui-btn-sm" disabled={loading} onclick={load}>重试</button>{/if}
  {/if}
  {#if saved}<p class="text-[12px] text-success-fg" role="status">{saved}</p>{/if}
</section>
