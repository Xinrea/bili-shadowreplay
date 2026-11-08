<script lang="ts">
  import { onMount, tick } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { invoke, TAURI_ENV } from "../invoker";
  import type { Config, StorageConfig } from "../interface";
  import { storageBackend, storageBackendEntries } from "../recording-storage";
  import StorageTransferSettings from "./StorageTransferSettings.svelte";
  import { Check, FolderOpen, HardDrive, Plus, Trash2, X } from "lucide-svelte";
  import { trapFocus } from "../focus-trap";
  import { pageVisibility } from "../page-visibility";
  import { openTaskCenter } from "../stores/tasks";

  let { onclose }: { onclose: () => void } = $props();
  const pageVisible = pageVisibility();
  let storage: StorageConfig | null = $state(null);
  let defaultId = $state<string | null>("");
  let newId = $state("");
  let newRoot = $state("");
  let loading = $state(true);
  let saving = $state(false);
  let error = $state("");
  let saved = $state("");
  let removingId = $state<string | null>(null);
  let browsing = $state(false);
  let transferBusy = $state(true);
  let transferStarting = $state(false);
  let view = $state<"locations" | "transfer">("locations");
  let showAdd = $state(false);
  let newIdInput = $state<HTMLInputElement>();

  function mountToViewport(node: HTMLElement) {
    // PageView's containment otherwise limits fixed dialogs to the page column.
    document.body.appendChild(node);
    return { destroy() { node.remove(); } };
  }

  function selectView(next: "locations" | "transfer") {
    if (saving || browsing || transferStarting) return;
    view = next;
    removingId = null;
    error = "";
    saved = "";
  }

  async function openAdd() {
    if (saving || browsing || transferBusy) return;
    showAdd = true;
    removingId = null;
    error = "";
    saved = "";
    await tick();
    newIdInput?.focus();
  }

  async function setDefault(id: string) {
    if (saving || browsing || transferBusy) return;
    defaultId = id;
    await save(false);
  }

  function close() {
    if (!saving && !browsing && !transferStarting) onclose();
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key !== "Escape" || !pageVisible() || event.defaultPrevented || event.isComposing) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (saving || browsing || transferStarting) return;
    if (removingId !== null) removingId = null;
    else if (showAdd && view === "locations") showAdd = false;
    else close();
  }

  async function load() {
    if (saving || browsing || transferStarting) return;
    loading = true;
    transferBusy = true;
    error = "";
    saved = "";
    removingId = null;
    try {
      storage = (await invoke<Config>("get_config")).storage;
      defaultId = storage.default_backend;
    } catch (e) {
      error = `录制存储加载失败：${String(e)}`;
    } finally {
      loading = false;
    }
  }

  async function save(add: boolean) {
    if (!storage || loading || saving || browsing || transferBusy) return;
    error = "";
    saved = "";
    const id = newId.trim();
    const root = newRoot.trim();
    if (add && (!id || !root || storageBackend(storage, id))) {
      error = "请输入未使用的存储 ID 和服务器上的目录路径。";
      return;
    }
    const next: StorageConfig = {
      default_backend: add ? storage.default_backend : defaultId ?? "",
      backends: { ...storage.backends, ...(add ? { [id]: { type: "local", root } } : {}) },
    };
    if (storageBackend(next, next.default_backend)?.type !== "local") {
      error = "请选择已配置的本地存储作为默认存储。";
      return;
    }
    saving = true;
    try {
      // Compare-and-swap prevents a stale full replacement deleting another client's backend.
      storage = await invoke<StorageConfig>("set_storage_config", { storage: next, expected: storage });
      defaultId = storage.default_backend;
      if (add) { newId = ""; newRoot = ""; showAdd = false; }
      saved = add ? "本地存储已添加。" : "默认录制存储已保存。";
    } catch (e) {
      error = `录制存储保存失败：${String(e)}。若配置已被修改，请重新加载后再操作。`;
    } finally {
      saving = false;
    }
  }

  async function removeBackend(id: string) {
    if (!storage || loading || saving || browsing || transferBusy || removingId !== id) return;
    error = "";
    saved = "";
    if (id === storage.default_backend) {
      error = "请先保存其他默认存储，再移除此存储。";
      return;
    }
    if (!storageBackend(storage, id)) return;
    const backends = { ...storage.backends };
    delete backends[id];
    saving = true;
    try {
      storage = await invoke<StorageConfig>("set_storage_config", {
        storage: { default_backend: storage.default_backend, backends },
        expected: storage,
      });
      defaultId = storage.default_backend;
      removingId = null;
      saved = `存储 ${id} 的配置已从数据库移除。文件和录播记录未删除。`;
    } catch (e) {
      error = `移除存储配置失败：${String(e)}。未确认移除成功，请刷新核实后重试。`;
    } finally {
      saving = false;
    }
  }

  async function browse() {
    if (browsing || saving || transferBusy) return;
    browsing = true;
    try {
      const selected = await open({ directory: true });
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (path) newRoot = path;
    } catch (e) {
      error = `无法选择文件夹：${String(e)}`;
    } finally {
      browsing = false;
    }
  }

  onMount(() => { void load(); });
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="ui-modal-backdrop" use:mountToViewport style:display={pageVisible() ? undefined : "none"} inert={!pageVisible()}>
  <button class="absolute inset-0" type="button" aria-label="关闭录制存储设置" disabled={saving || browsing || transferStarting} onclick={close}></button>
  <div use:trapFocus={{ active: pageVisible(), initial: () => document.getElementById("storage-settings-title") }} role="dialog" aria-modal="true" aria-labelledby="storage-settings-title" tabindex="-1" class="ui-modal relative w-[800px] max-w-full">
    <div class="ui-modal-header !py-4">
      <div class="flex items-center gap-2">
        <HardDrive class="h-4 w-4 text-fg-muted" />
        <h2 id="storage-settings-title" tabindex="-1" class="!text-[16px] outline-none">录制存储</h2>
      </div>
      <div class="flex items-center gap-2">
        <button class="ui-btn ui-btn-ghost ui-btn-sm" type="button" onclick={() => openTaskCenter()}>任务中心</button>
        <button class="ui-btn ui-btn-ghost ui-btn-sm ui-btn-icon" type="button" aria-label="关闭录制存储设置窗口" disabled={saving || browsing || transferStarting} onclick={close}><X class="h-4 w-4" /></button>
      </div>
    </div>
    <nav class="flex shrink-0 gap-5 border-b border-line px-[18px]" aria-label="存储设置视图">
      <button class="storage-view" class:selected={view === "locations"} aria-pressed={view === "locations"}
        disabled={saving || browsing || transferStarting} onclick={() => selectView("locations")}>
        存储位置
        {#if storage}<span class="ui-badge tabular-nums" aria-label={`${Object.keys(storage.backends).length} 个存储位置`}>{Object.keys(storage.backends).length}</span>{/if}
      </button>
      <button class="storage-view" class:selected={view === "transfer"} aria-pressed={view === "transfer"}
        disabled={saving || browsing || transferStarting} onclick={() => selectView("transfer")}>录播转移</button>
    </nav>
    <div class="ui-modal-body space-y-3">
      {#if loading}
        <p class="py-10 text-center text-[12px] text-fg-muted" role="status">正在加载存储位置…</p>
      {:else if storage}
        <div hidden={view !== "locations"} inert={view !== "locations"}>
          <div class="mb-4 flex items-start justify-between gap-4">
            <div class="min-w-0">
              <h3 class="text-[13px] font-medium text-fg">存储位置</h3>
              <p class="mt-1 text-[12px] leading-relaxed text-fg-muted">默认位置用于后续新录播，已有录播不会随之移动。</p>
            </div>
            <button class="ui-btn ui-btn-primary" disabled={saving || browsing || transferBusy || showAdd} onclick={openAdd}>
              <Plus class="h-3.5 w-3.5" />添加存储
            </button>
          </div>
          {#if showAdd}
            <form class="mb-4 space-y-3 rounded-[8px] border border-line-strong bg-sunken/50 p-4" onsubmit={(event) => { event.preventDefault(); void save(true); }}>
              <div class="flex items-center justify-between gap-3">
                <h4 class="text-[13px] font-medium">添加本地存储</h4>
                <span class="text-[11px] text-fg-muted">本地目录</span>
              </div>
              <fieldset class="min-w-0 space-y-3" disabled={saving || browsing || transferBusy}>
                <div>
                  <label for="new-storage-id" class="ui-label mb-1 block">存储名称（ID）</label>
                  <input id="new-storage-id" bind:this={newIdInput} class="ui-input w-full" bind:value={newId} placeholder="例如 archive-disk" />
                </div>
                <div>
                  <label for="new-storage-root" class="ui-label mb-1 block">目录路径</label>
                  <div class="flex gap-2">
                    <input id="new-storage-root" class="ui-input min-w-0 flex-1" bind:value={newRoot} placeholder="例如 /mnt/recordings" />
                    {#if TAURI_ENV}<button type="button" class="ui-btn shrink-0" onclick={browse}><FolderOpen class="h-3.5 w-3.5" />选择</button>{/if}
                  </div>
                  <p class="ui-hint mt-1">{TAURI_ENV ? "名称和目录保存后不可修改；换位置时请添加新的存储。" : "填写服务器上的目录，而不是浏览器所在机器的路径。名称和目录保存后不可修改。"}</p>
                </div>
                <div class="flex justify-end gap-2">
                  <button type="button" class="ui-btn ui-btn-ghost" onclick={() => showAdd = false}>取消</button>
                  <button type="submit" class="ui-btn ui-btn-primary" disabled={!newId.trim() || !newRoot.trim()}>{saving ? "正在添加…" : "添加存储"}</button>
                </div>
              </fieldset>
            </form>
          {/if}
          <ul class="divide-y divide-line border-y border-line">
            {#each storageBackendEntries(storage) as [id, backend] (id)}
              <li class="py-4">
                <div class="flex items-start gap-3">
                  <div class="flex h-9 w-9 shrink-0 items-center justify-center rounded-[8px] bg-control text-fg-muted"><HardDrive class="h-4 w-4" /></div>
                  <div class="flex min-w-0 flex-1 flex-col gap-3 sm:flex-row sm:items-start">
                    <div class="min-w-0 flex-1">
                      <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
                        <span class="break-all text-[13px] font-medium text-fg">{id}</span>
                        {#if id === storage.default_backend}<span class="inline-flex items-center gap-1 text-[11px] text-accent-fg"><Check class="h-3 w-3" />默认</span>{/if}
                        <span class="text-[11px] text-fg-muted">{backend.type === "local" ? "本地目录" : "暂不支持"}</span>
                      </div>
                      <p class="mt-1 select-text break-all text-[12px] leading-relaxed text-fg-muted">{backend.root}</p>
                    </div>
                    <div class="storage-actions flex shrink-0 items-center gap-1" role="group" aria-label={`${id} 的存储操作`}>
                      {#if id !== storage.default_backend}
                        <button class="ui-btn ui-btn-ghost ui-btn-sm" disabled={saving || browsing || transferBusy || backend.type !== "local"} onclick={() => setDefault(id)}>
                          <Check class="h-3.5 w-3.5" />设为默认
                        </button>
                      {/if}
                      <button class="ui-btn ui-btn-ghost ui-btn-sm" aria-label={`移除存储 ${id}`}
                        disabled={saving || browsing || transferBusy || id === storage.default_backend}
                        title={id === storage.default_backend ? "先将其他位置设为默认，再移除此存储" : "移除存储配置"}
                        onclick={() => { removingId = id; error = ""; saved = ""; }}>
                        <Trash2 class="h-3.5 w-3.5" />移除
                      </button>
                    </div>
                  </div>
                </div>
                {#if removingId === id}
                  <div class="mt-3 space-y-2 rounded-[8px] border border-danger/30 bg-danger-soft p-3" role="group" aria-label={`确认移除存储 ${id}`}>
                    <p class="text-[13px] font-medium text-danger-fg">移除 {id}？</p>
                    <p class="text-[12px] leading-relaxed text-fg-secondary">文件和录播记录会保留，但这些录播将无法预览或剪辑。需要继续使用的录播，请先转移。</p>
                    <p class="text-[12px] leading-relaxed text-danger-fg">此 ID 无法重新添加，移除后不能恢复配置。直播间的存储偏好不会自动切换。</p>
                    <div class="flex justify-end gap-2">
                      <button class="ui-btn ui-btn-ghost ui-btn-sm" disabled={saving} onclick={() => removingId = null}>取消</button>
                      <button class="ui-btn ui-btn-sm ui-btn-danger" disabled={saving || browsing || transferBusy} onclick={() => removeBackend(id)}>确认移除</button>
                    </div>
                  </div>
                {/if}
              </li>
            {/each}
          </ul>
          {#if transferBusy}<p class="mt-3 text-[12px] text-fg-muted" role="status">正在检查或执行转移任务，存储配置暂不可修改。可在“录播转移”中查看状态。</p>{/if}
          <p class="mt-4 text-[11px] leading-relaxed text-fg-faint">上传、预览及处理产生的临时文件由系统管理，不占用这里的录播目录。</p>
        </div>
        <!-- Keep recovery mounted when switching views; a hidden form must not
             release the configuration lock or forget a running transfer. -->
        <div hidden={view !== "transfer"} inert={view !== "transfer"}>
          <StorageTransferSettings {storage} disabled={saving || browsing}
            bind:busy={transferBusy} bind:starting={transferStarting} />
        </div>
      {/if}
      {#if error}
        <div class="rounded-[8px] bg-danger-soft p-3">
          <p class="text-[12px] text-danger-fg whitespace-pre-wrap" role="alert">{error}</p>
          <button class="ui-btn ui-btn-sm mt-2" disabled={loading || saving || browsing || (storage !== null && transferBusy)} onclick={load}>重新加载配置</button>
        </div>
      {/if}
      {#if saving}<p class="ui-hint" role="status">正在保存…</p>
      {:else if saved}<p class="text-[12px] text-success-fg" role="status">{saved}</p>{/if}
    </div>
    <div class="ui-modal-footer">
      <button class="ui-btn" type="button" disabled={saving || browsing || transferStarting} onclick={close}>关闭</button>
    </div>
  </div>
</div>

<style>
  .storage-view {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    border-bottom: 2px solid transparent;
    padding: 11px 0;
    color: rgb(var(--c-fg-muted));
    font-size: 12px;
    font-weight: 500;
  }
  .storage-view.selected { border-color: rgb(var(--c-accent)); color: rgb(var(--c-fg)); }
  .storage-view:hover:not(:disabled) { color: rgb(var(--c-fg)); }
  .storage-view:disabled { opacity: 0.5; cursor: not-allowed; }
  .storage-view:focus-visible { outline: 2px solid rgb(var(--c-accent)); outline-offset: -3px; }
</style>
