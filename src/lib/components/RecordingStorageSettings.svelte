<script lang="ts">
  import { onMount, tick } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { invoke, TAURI_ENV } from "../invoker";
  import type { Config, StorageConfig } from "../interface";
  import { storageBackend, storageBackendEntries, storageBackendSupported, storageBackendLocation } from "../recording-storage";
  import StorageTransferSettings from "./StorageTransferSettings.svelte";
  import S3CredentialFields from "./S3CredentialFields.svelte";
  import { Check, FolderOpen, HardDrive, ListChecks, Plus, Trash2, X } from "lucide-svelte";
  import { trapFocus } from "../focus-trap";
  import { pageVisibility } from "../page-visibility";
  import { openTaskCenter } from "../stores/tasks";

  let { onclose }: { onclose: () => void } = $props();
  const pageVisible = pageVisibility();
  let storage: StorageConfig | null = $state(null);
  let defaultId = $state<string | null>("");
  let newId = $state("");
  let newRoot = $state("");
  let newType = $state("local");
  let s3Endpoint = $state("");
  let s3Bucket = $state("");
  let s3Region = $state("us-east-1");
  let s3Prefix = $state("");
  let s3PathStyle = $state(true);
  let s3AccessKeyId = $state("");
  let s3SecretAccessKey = $state("");
  let s3SessionToken = $state("");
  let editingCredentials = $state<string | null>(null);
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
  let catalogStatuses = $state<Record<string, { ready: boolean; writable: boolean; error: string | null }>>({});

  async function loadCatalogStatus(refresh = false) {
    try {
      const statuses = await invoke<Array<{ alias: string; ready: boolean; writable: boolean; error: string | null }>>("get_storage_catalog_status", { refresh });
      catalogStatuses = Array.isArray(statuses) ? Object.fromEntries(statuses.map(status => [status.alias, status])) : {};
    } catch {
      catalogStatuses = {};
    }
  }

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
    editingCredentials = null;
    clearCredentials();
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

  function clearCredentials() {
    s3AccessKeyId = "";
    s3SecretAccessKey = "";
    s3SessionToken = "";
  }

  function editCredentials(id: string) {
    if (saving || browsing || transferBusy) return;
    showAdd = false;
    removingId = null;
    clearCredentials();
    editingCredentials = id;
    error = "";
    saved = "";
  }

  function cancelCredentials() {
    editingCredentials = null;
    clearCredentials();
  }

  async function saveCredentials() {
    if (!storage || !editingCredentials || saving || browsing || transferBusy) return;
    if (!s3AccessKeyId.trim() || !s3SecretAccessKey.trim()) {
      error = "请填写 Access Key ID 和 Secret Access Key。";
      return;
    }
    saving = true;
    error = "";
    saved = "";
    try {
      storage = await invoke<StorageConfig>("set_storage_config", {
        storage, expected: storage,
        credentials: { [editingCredentials]: {
          access_key_id: s3AccessKeyId.trim(), secret_access_key: s3SecretAccessKey, session_token: s3SessionToken,
        } },
      });
      cancelCredentials();
      void loadCatalogStatus();
      saved = "S3 凭证已更新，无需重启应用。";
    } catch (e) {
      error = `凭证保存失败：${String(e)}。若配置已被修改，请重新加载后再操作。`;
    } finally {
      saving = false;
    }
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key !== "Escape" || !pageVisible() || event.defaultPrevented || event.isComposing) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (saving || browsing || transferStarting) return;
    if (removingId !== null) removingId = null;
    else if (editingCredentials !== null && view === "locations") cancelCredentials();
    else if (showAdd && view === "locations") { showAdd = false; clearCredentials(); }
    else close();
  }

  async function load() {
    if (saving || browsing || transferStarting) return;
    loading = true;
    transferBusy = true;
    error = "";
    saved = "";
    removingId = null;
    cancelCredentials();
    try {
      storage = (await invoke<Config>("get_config")).storage;
      defaultId = storage.default_backend;
      await loadCatalogStatus(true);
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
    if (add && (!id || storageBackend(storage, id) || (newType === "local" ? !root : !s3Bucket.trim() || !s3Region.trim() || !s3AccessKeyId.trim() || !s3SecretAccessKey.trim()))) {
      error = "请输入未使用的存储 ID，并填写目录路径或 S3 必填配置。";
      return;
    }
    const next: StorageConfig = {
      default_backend: add ? storage.default_backend : defaultId ?? "",
      backends: { ...storage.backends, ...(add ? { [id]: newType === "local" ? { type: "local", root } : {
        type: "s3", root: "", s3: {
          endpoint: s3Endpoint.trim(), bucket: s3Bucket.trim(), region: s3Region.trim(),
          prefix: s3Prefix.trim(), path_style: s3PathStyle,
        },
      } } : {}) },
    };
    if (!storageBackendSupported(storageBackend(next, next.default_backend))) {
      error = "请选择已配置的存储作为默认存储。";
      return;
    }
    saving = true;
    try {
      // Compare-and-swap prevents a stale full replacement deleting another client's backend.
      storage = await invoke<StorageConfig>("set_storage_config", {
        storage: next, expected: storage,
        ...(add && newType === "s3" ? { credentials: { [id]: {
          access_key_id: s3AccessKeyId.trim(), secret_access_key: s3SecretAccessKey, session_token: s3SessionToken,
        } } } : {}),
      });
      defaultId = storage.default_backend;
      if (add) {
        newId = ""; newRoot = ""; s3Endpoint = ""; s3Bucket = ""; s3Prefix = "";
        s3Region = "us-east-1"; s3PathStyle = true; clearCredentials(); showAdd = false;
      }
      void loadCatalogStatus();
      saved = add ? "存储配置已添加，目录状态见下方。" : "默认录制存储已保存。";
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
      if (editingCredentials === id) cancelCredentials();
      void loadCatalogStatus();
      saved = `存储 ${id} 已卸载，对应录播已从列表隐藏。存储上的文件和目录数据库未删除。`;
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
        <button class="ui-btn ui-btn-ghost ui-btn-sm" type="button" onclick={() => openTaskCenter()}><ListChecks class="h-3.5 w-3.5" />任务中心</button>
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
        <p class="ui-hint">同一存储请只用一个 BSR 实例写入；切换机器前先停止原实例。</p>
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
                <h4 class="text-[13px] font-medium">添加存储</h4>
              </div>
              <fieldset class="min-w-0 space-y-3" disabled={saving || browsing || transferBusy}>
                <div>
                  <label for="new-storage-type" class="ui-label mb-1 block">存储类型</label>
                  <select id="new-storage-type" class="ui-input w-full" bind:value={newType}>
                    <option value="local">本地目录</option>
                    <option value="s3">S3 兼容对象存储</option>
                  </select>
                </div>
                <div>
                  <label for="new-storage-id" class="ui-label mb-1 block">存储名称（ID）</label>
                  <input id="new-storage-id" bind:this={newIdInput} class="ui-input w-full" bind:value={newId} placeholder="例如 archive-disk" />
                </div>
                {#if newType === "local"}
                <div>
                  <label for="new-storage-root" class="ui-label mb-1 block">目录路径</label>
                  <div class="flex gap-2">
                    <input id="new-storage-root" class="ui-input min-w-0 flex-1" bind:value={newRoot} placeholder="例如 /mnt/recordings" />
                    {#if TAURI_ENV}<button type="button" class="ui-btn shrink-0" onclick={browse}><FolderOpen class="h-3.5 w-3.5" />选择</button>{/if}
                  </div>
                  <p class="ui-hint mt-1">{TAURI_ENV ? "名称和目录保存后不可修改；换位置时请添加新的存储。" : "填写服务器上的目录，而不是浏览器所在机器的路径。名称和目录保存后不可修改。"}</p>
                </div>
                {:else}
                  <div>
                    <label for="s3-endpoint" class="ui-label mb-1 block">Endpoint</label>
                    <input id="s3-endpoint" class="ui-input w-full" bind:value={s3Endpoint} placeholder="留空使用 AWS；RustFS：http://127.0.0.1:9000" />
                    <p class="ui-hint mt-1">填写服务地址，不带 bucket 路径；关闭 path-style 时会自动将 bucket 加到域名前。生产环境请使用 HTTPS。</p>
                  </div>
                  <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
                    <div>
                      <label for="s3-bucket" class="ui-label mb-1 block">Bucket</label>
                      <input id="s3-bucket" class="ui-input w-full" bind:value={s3Bucket} placeholder="已创建的 bucket" />
                    </div>
                    <div>
                      <label for="s3-region" class="ui-label mb-1 block">Region</label>
                      <input id="s3-region" class="ui-input w-full" bind:value={s3Region} />
                    </div>
                  </div>
                  <div>
                    <label for="s3-prefix" class="ui-label mb-1 block">对象前缀（可选）</label>
                    <input id="s3-prefix" class="ui-input w-full" bind:value={s3Prefix} placeholder="例如 shadowreplay，不带首尾 /" />
                  </div>
                  <S3CredentialFields idPrefix="new-s3" bind:accessKeyId={s3AccessKeyId} bind:secretAccessKey={s3SecretAccessKey} bind:sessionToken={s3SessionToken} />
                  <label class="flex items-center gap-2 text-[12px]"><input type="checkbox" bind:checked={s3PathStyle} />使用 path-style（RustFS / MinIO）</label>
                  <p class="ui-hint">存储位置保存后不可修改，凭证可单独更新；换 bucket 或前缀时请添加新存储，再通过录播转移搬迁。</p>
                {/if}
                <div class="flex justify-end gap-2">
                  <button type="button" class="ui-btn ui-btn-ghost" onclick={() => { showAdd = false; clearCredentials(); }}>取消</button>
                  <button type="submit" class="ui-btn ui-btn-primary" disabled={!newId.trim() || (newType === "local" ? !newRoot.trim() : !s3Bucket.trim() || !s3Region.trim() || !s3AccessKeyId.trim() || !s3SecretAccessKey.trim())}>{saving ? "正在添加…" : "添加存储"}</button>
                </div>
              </fieldset>
            </form>
          {/if}
          <div class="overflow-hidden rounded-[8px] border border-line">
            <table class="w-full table-fixed text-left">
              <thead class="border-b border-line text-[11px] font-medium text-fg-muted">
                <tr>
                  <th class="w-8 px-3 py-2 font-medium"><span class="sr-only">状态</span></th>
                  <th class="w-[200px] whitespace-nowrap px-2 py-2 font-medium">名称</th>
                  <th class="w-[76px] whitespace-nowrap px-2 py-2 font-medium">类型</th>
                  <th class="px-2 py-2 font-medium">位置</th>
                  <th class="w-[170px] whitespace-nowrap px-3 py-2 text-right font-medium">操作</th>
                </tr>
              </thead>
              <tbody class="divide-y divide-line text-[12px]">
                {#if storageBackendEntries(storage).length === 0}
                  <tr><td colspan="5" class="px-3 py-6 text-center text-[12px] text-fg-muted">还没有配置存储位置。</td></tr>
                {/if}
                {#each storageBackendEntries(storage) as [id, backend] (id)}
                  {@const status = catalogStatuses[id]}
                  <tr class="group hover:bg-control-hover/50">
                    <td class="px-3 py-2.5">
                      {#if status}
                        <span class="inline-block h-2 w-2 rounded-full {status.ready ? (status.writable ? "bg-success" : "bg-warning") : "bg-danger"}"
                          role="img" aria-label={status.ready ? (status.writable ? "已连接，可写入" : "只读") : "不可用"}
                          title={status.ready ? (status.writable ? "已连接，可写入" : "只读：本机存储正在使用或应用处于只读模式")
                            : (status.error || "目录暂不可用")}></span>
                      {:else}
                        <span class="inline-block h-2 w-2 rounded-full bg-line-strong" role="img" aria-label="状态未知"
                          title="目录状态暂不可用；如有旧转移未完成，请完成后重启应用进行迁移。"></span>
                      {/if}
                    </td>
                    <td class="min-w-0 px-2 py-2.5">
                      <div class="flex min-w-0 items-center gap-2">
                        <span class="min-w-0 truncate text-[13px] font-medium text-fg" title={id}>{id}</span>
                        {#if id === storage.default_backend}<span class="ui-badge ui-badge-accent inline-flex shrink-0 items-center gap-1"><Check class="h-3 w-3" />默认</span>{/if}
                        {#if id !== storage.default_backend}
                          <button class="storage-default-action opacity-0 pointer-events-none focus-visible:opacity-100 focus-visible:pointer-events-auto group-hover:opacity-100 group-hover:pointer-events-auto"
                            disabled={saving || browsing || transferBusy || !storageBackendSupported(backend)} onclick={() => setDefault(id)}>
                            <Check class="h-3 w-3" />设为默认
                          </button>
                        {/if}
                      </div>
                    </td>
                    <td class="whitespace-nowrap px-2 py-2.5 text-fg-muted">{backend.type === "local" ? "本地目录" : backend.type === "s3" ? "S3" : "暂不支持"}</td>
                    <td class="overflow-hidden px-2 py-2.5">
                      <span class="block select-text truncate text-fg-muted" title={storageBackendLocation(backend)}>{storageBackendLocation(backend)}</span>
                    </td>
                    <td class="whitespace-nowrap px-3 py-2.5">
                      <div class="storage-actions flex items-center justify-end gap-1" role="group" aria-label={`${id} 的存储操作`}>
                        {#if backend.type === "s3"}
                          <button class="ui-btn ui-btn-ghost ui-btn-sm" disabled={saving || browsing || transferBusy} onclick={() => editCredentials(id)}>
                            {storage.credential_revisions?.[id] ? "更新凭证" : "填写凭证"}
                          </button>
                        {/if}
                        <button class="ui-btn ui-btn-ghost ui-btn-sm" aria-label={`移除存储 ${id}`}
                          disabled={saving || browsing || transferBusy || id === storage.default_backend}
                          title={id === storage.default_backend ? "先将其他位置设为默认，再移除此存储" : "移除存储配置"}
                          onclick={() => { removingId = id; error = ""; saved = ""; }}>
                          <Trash2 class="h-3.5 w-3.5" />移除
                        </button>
                      </div>
                    </td>
                  </tr>
                  {#if editingCredentials === id}
                    <tr class="bg-sunken/40">
                      <td colspan="5" class="px-3 pb-3 pt-1.5">
                        <form class="space-y-3 rounded-[8px] border border-line-strong bg-surface p-3" onsubmit={(event) => { event.preventDefault(); void saveCredentials(); }}>
                          <h4 class="text-[13px] font-medium">{id} 的 S3 凭证</h4>
                          <fieldset class="space-y-3" disabled={saving || browsing || transferBusy}>
                            <S3CredentialFields idPrefix="edit-s3" bind:accessKeyId={s3AccessKeyId} bind:secretAccessKey={s3SecretAccessKey} bind:sessionToken={s3SessionToken} />
                            <p class="ui-hint">更新不改变已有录播的位置。如果此存储正在录制，请先停止录制再更新。</p>
                            <div class="flex justify-end gap-2">
                              <button type="button" class="ui-btn ui-btn-ghost" onclick={cancelCredentials}>取消</button>
                              <button type="submit" class="ui-btn ui-btn-primary" disabled={!s3AccessKeyId.trim() || !s3SecretAccessKey.trim()}>{saving ? "正在保存…" : "保存凭证"}</button>
                            </div>
                          </fieldset>
                        </form>
                      </td>
                    </tr>
                  {/if}
                  {#if removingId === id}
                    <tr class="bg-danger-soft/40">
                      <td colspan="5" class="px-3 pb-3 pt-1.5">
                        <div class="space-y-2 rounded-[8px] border border-danger/30 bg-danger-soft p-3" role="group" aria-label={`确认移除存储 ${id}`}>
                          <p class="text-[13px] font-medium text-danger-fg">移除 {id}？</p>
                          <p class="text-[12px] leading-relaxed text-fg-secondary">对应录播将从列表隐藏，存储上的文件和目录数据库不会删除。重新添加同一目录或 S3 bucket/prefix 后会恢复。</p>
                          <p class="text-[12px] leading-relaxed text-fg-secondary">存储名称可以复用。直播间仍绑定原存储，不会因同名存储被添加而自动切换。</p>
                          <div class="flex justify-end gap-2">
                            <button class="ui-btn ui-btn-ghost ui-btn-sm" disabled={saving} onclick={() => removingId = null}>取消</button>
                            <button class="ui-btn ui-btn-sm ui-btn-danger" disabled={saving || browsing || transferBusy} onclick={() => removeBackend(id)}>确认移除</button>
                          </div>
                        </div>
                      </td>
                    </tr>
                  {/if}
                {/each}
              </tbody>
            </table>
          </div>
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
  /* Same pill as the default badge, but clickable. */
  .storage-default-action {
    display: inline-flex;
    flex-shrink: 0;
    align-items: center;
    gap: 4px;
    border-radius: 5px;
    background: rgb(var(--c-control));
    padding: 1px 6px;
    color: rgb(var(--c-fg-muted));
    font-size: 10px;
    font-weight: 500;
    line-height: 16px;
    white-space: nowrap;
    transition: background-color 120ms, color 120ms, opacity 120ms;
  }
  .storage-default-action:hover:not(:disabled) { background: rgb(var(--c-control-hover)); color: rgb(var(--c-fg)); }
  .storage-default-action:disabled { cursor: not-allowed; }
  .storage-default-action:focus-visible { outline: 2px solid rgb(var(--c-accent)); outline-offset: 1px; }
</style>
