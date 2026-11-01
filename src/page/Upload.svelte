<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { trapFocus } from "../lib/focus-trap";
  import {
    Calendar,
    ExternalLink,
    Eye,
    Loader2,
    Pencil,
    Plus,
    RefreshCw,
    Search,
    Trash2,
    Upload,
  } from "lucide-svelte";
  import PageTitlebar from "../lib/components/PageTitlebar.svelte";
  import BatchActionLayout from "../lib/components/BatchActionLayout.svelte";
  import DeleteConfirmActions from "../lib/components/DeleteConfirmActions.svelte";
  import UploadEditor from "../lib/components/upload/UploadEditor.svelte";
  import UploadSourceDialog from "../lib/components/upload/UploadSourceDialog.svelte";
  import { get_static_url, invoke } from "../lib/invoker";
  import type { ProjectItem, UploadItem, UploadStatus, VideoItem } from "../lib/interface";
  import { registerCommands } from "../lib/stores/commands";
  import { createStateRefresh, watchStateChanges } from "../lib/state-refresh";
  import {
    draftUpload,
    isUploadActive,
    isUploadEditable,
    sourceLabels,
    uploadAccounts,
    uploadRequest,
    uploadTitle,
    type UploadPick,
    type UploadRequest,
  } from "../lib/uploads";

  interface UploadRow extends UploadItem {
    title: string;
    coverUrl: string;
  }

  type Filter = "all" | "draft" | "active" | "posted" | "failed";

  const filters: { value: Filter; label: string; match: (upload: UploadItem) => boolean }[] = [
    { value: "all", label: "全部", match: () => true },
    { value: "draft", label: "草稿", match: (upload) => upload.status === "draft" },
    { value: "active", label: "进行中", match: isUploadActive },
    { value: "posted", label: "已投稿", match: (upload) => upload.status === "posted" },
    { value: "failed", label: "失败", match: (upload) => upload.status === "failed" },
  ];

  const statusBadges: Record<UploadStatus, { label: string; tone: string }> = {
    draft: { label: "草稿", tone: "" },
    exporting: { label: "生成视频中", tone: "ui-badge-accent" },
    posting: { label: "投稿中", tone: "ui-badge-accent" },
    posted: { label: "已投稿", tone: "ui-badge-success" },
    failed: { label: "失败", tone: "ui-badge-danger" },
  };

  let uploads: UploadRow[] = $state([]);
  let accounts: { uid: string; name: string }[] = $state([]);
  let loading = $state(true);
  let filter: Filter = $state("all");
  let search = $state("");
  let showSourceDialog = $state(false);
  let editingId: number | null = $state(null);
  const selectedUploads = new SvelteSet<number>();
  let uploadsToDelete: UploadRow[] | null = $state(null);

  let editing = $derived(uploads.find((upload) => upload.id === editingId) ?? null);
  let accountNames = $derived(new Map(accounts.map((account) => [account.uid, account.name])));
  let counts = $derived(
    Object.fromEntries(filters.map((item) => [item.value, uploads.filter(item.match).length])),
  );
  let filtered = $derived.by(() => {
    const keyword = search.trim().toLowerCase();
    const match = filters.find((item) => item.value === filter)!.match;
    return uploads.filter(
      (upload) =>
        match(upload) &&
        (!keyword ||
          upload.title.toLowerCase().includes(keyword) ||
          upload.source_title.toLowerCase().includes(keyword) ||
          upload.bvid.toLowerCase().includes(keyword)),
    );
  });
  let selectable = $derived(filtered.filter((upload) => !isUploadActive(upload)));
  let selected = $derived(selectable.filter((upload) => selectedUploads.has(upload.id)));
  let allSelected = $derived(selectable.length > 0 && selected.length === selectable.length);

  $effect(() => {
    // Drop hidden, deleted or newly active uploads from the selection.
    const selectableIds = new Set(selectable.map((upload) => upload.id));
    for (const id of selectedUploads) {
      if (!selectableIds.has(id)) selectedUploads.delete(id);
    }
  });

  async function toRow(upload: UploadItem): Promise<UploadRow> {
    const coverUrl = upload.display_cover
      ? `${await get_static_url("output", upload.display_cover)}?t=${encodeURIComponent(upload.updated_at)}`
      : "";
    return { ...upload, title: uploadTitle(upload), coverUrl };
  }

  const stateRefresh = createStateRefresh(async () => {
    try {
      const list = await invoke<UploadItem[]>("get_uploads");
      uploads = await Promise.all(list.map(toRow));
    } finally {
      loading = false;
    }
    try {
      accounts = await uploadAccounts();
    } catch (error) {
      console.error("Failed to load accounts:", error);
    }
  });

  onMount(() =>
    watchStateChanges(stateRefresh, ["uploads-changed", "projects-changed", "videos-changed"]),
  );

  onMount(() =>
    registerCommands([
      {
        id: "upload:new",
        group: "action",
        title: "新建投稿",
        detail: "投稿",
        icon: Plus,
        keywords: ["new upload", "post", "bilibili"],
        page: "投稿",
        run: () => (showSourceDialog = true),
      },
      {
        id: "upload:refresh",
        group: "action",
        title: "刷新投稿列表",
        detail: "投稿",
        icon: RefreshCw,
        keywords: ["refresh uploads", "reload"],
        page: "投稿",
        run: () => stateRefresh.run(),
      },
    ]),
  );

  // Other pages ask to post a material or project; open its draft here.
  onMount(() =>
    uploadRequest.subscribe((request) => {
      if (!request) return;
      uploadRequest.set(null);
      void openRequest(request);
    }),
  );

  async function openRequest(request: UploadRequest) {
    try {
      await stateRefresh.run();
      const pick: UploadPick =
        request.source === "material"
          ? { source: "material", item: await invoke<VideoItem>("get_video", { id: request.id }) }
          : { source: "project", item: await invoke<ProjectItem>("get_project", { id: request.id }) };
      await createDraft(pick);
    } catch (error) {
      alert("新建投稿失败：" + error);
    }
  }

  /** Open the source's unposted draft, or start one. */
  async function createDraft(pick: UploadPick) {
    try {
      const draft = await draftUpload(pick, uploads);
      if (!uploads.some((upload) => upload.id === draft.id)) {
        uploads = [await toRow(draft), ...uploads];
      }
      showSourceDialog = false;
      editingId = draft.id;
    } catch (error) {
      alert("新建投稿失败：" + error);
    }
  }

  function toggleUploadSelection(upload: UploadRow) {
    if (isUploadActive(upload)) return;
    if (selectedUploads.has(upload.id)) selectedUploads.delete(upload.id);
    else selectedUploads.add(upload.id);
  }

  function selectAllUploads() {
    if (allSelected) selectedUploads.clear();
    else selectable.forEach((upload) => selectedUploads.add(upload.id));
  }

  function confirmDelete(upload: UploadRow | null = null) {
    const targets = (upload ? [upload] : selected).filter((item) => !isUploadActive(item));
    if (targets.length > 0) uploadsToDelete = [...targets];
  }

  async function deleteUploads() {
    const targets = [...(uploadsToDelete ?? [])];
    if (targets.length === 0) return;
    try {
      for (const upload of targets) {
        const current = uploads.find((item) => item.id === upload.id);
        if (current && isUploadActive(current)) {
          throw new Error(`「${current.title}」正在投稿，无法删除`);
        }
        try {
          await invoke("delete_upload", { id: upload.id });
        } catch (error) {
          // A concurrent deletion or lost response can leave this target already gone.
          // Verify against the server; a failed read must preserve the original error.
          const remaining = await invoke<UploadItem[]>("get_uploads").catch(() => null);
          if (!remaining || remaining.some((item) => item.id === upload.id)) throw error;
        }
        if (editingId === upload.id) editingId = null;
        selectedUploads.delete(upload.id);
        uploads = uploads.filter((item) => item.id !== upload.id);
        // Preserve failed and unattempted targets for retry, even after a refresh.
        uploadsToDelete = uploadsToDelete!.filter((item) => item.id !== upload.id);
      }
    } finally {
      await stateRefresh.run();
    }
    uploadsToDelete = null;
  }
</script>

<PageTitlebar title="投稿">
  {#snippet subtitle()}
    <span class="truncate text-[12px] text-fg-muted">
      选择素材、切片或视频文件投稿到 B 站；切片会在投稿时按时间轴生成视频。
    </span>
  {/snippet}
  <button class="titlebar-icon-button" title="刷新" aria-label="刷新" onclick={() => stateRefresh.run()}>
    <RefreshCw class="h-4 w-4" />
  </button>
  <button
    class="inline-flex h-7 items-center gap-1.5 whitespace-nowrap rounded-[7px] bg-accent px-2.5 text-[12px] font-medium text-white transition-colors hover:bg-accent/90"
    onclick={() => (showSourceDialog = true)}
  >
    <Plus class="h-3.5 w-3.5" />
    新建投稿
  </button>
</PageTitlebar>

<BatchActionLayout count={selected.length} itemLabel="投稿" onclear={() => selectedUploads.clear()}>
  {#snippet actions()}
    <button class="ui-btn ui-btn-danger" onclick={() => confirmDelete()}>
      <Trash2 class="h-3.5 w-3.5" />
      <span>删除选中</span>
    </button>
  {/snippet}
  <div class="space-y-4">
    <div class="ui-card px-3 py-2.5">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div class="ui-tabs" role="tablist" aria-label="投稿状态">
          {#each filters as item (item.value)}
            <button
              type="button"
              role="tab"
              class="ui-tab"
              aria-selected={filter === item.value}
              onclick={() => (filter = item.value)}
            >
              {item.label}
              <span class="tabular-nums text-fg-faint">{counts[item.value]}</span>
            </button>
          {/each}
        </div>
        <div class="relative">
          <Search class="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-fg-faint" />
          <input class="ui-input w-56 pl-8" placeholder="搜索标题、来源或 BV 号" bind:value={search} />
        </div>
      </div>
    </div>

    <div class="ui-card overflow-hidden">
      {#if loading}
        <div class="flex flex-col items-center justify-center space-y-3 p-12 text-[12px] text-fg-muted">
          <RefreshCw class="h-6 w-6 animate-spin text-fg-faint" />
          <span>加载中...</span>
        </div>
      {:else if filtered.length === 0}
        <div class="flex flex-col items-center justify-center space-y-2 p-12 text-fg-muted">
          <Upload class="h-10 w-10 text-fg-faint" />
          <h3 class="text-[14px] font-semibold text-fg">暂无投稿</h3>
          <p class="text-[12px]">
            {uploads.length > 0
              ? "没有找到匹配的投稿"
              : "点击「新建投稿」选择素材、切片或视频文件，也可以在素材页和切片页直接投稿"}
          </p>
        </div>
      {:else}
        <div class="overflow-x-auto custom-scrollbar-light">
          <table class="w-full table-fixed whitespace-nowrap">
            <thead>
              <tr class="border-b border-line bg-control/60">
                <th class="w-12 px-4 py-2.5 text-left">
                  <input
                    type="checkbox"
                    checked={allSelected}
                    indeterminate={selected.length > 0 && !allSelected}
                    disabled={selectable.length === 0}
                    onchange={selectAllUploads}
                    class="ui-checkbox"
                    aria-label="全选当前筛选结果中可删除的投稿"
                  />
                </th>
                <th class="w-[22rem] px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted">投稿</th>
                <th class="w-44 px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted">状态</th>
                <th class="w-32 px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted">账号</th>
                <th class="w-44 px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted">更新时间</th>
                <th class="w-28 px-4 py-2.5 text-left text-[11px] font-medium text-fg-muted">操作</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-line">
              {#each filtered as upload (upload.id)}
                {@const editable = isUploadEditable(upload)}
                <tr class="group transition-colors hover:bg-control-hover/50 {selectedUploads.has(upload.id) ? 'bg-accent-soft/40' : ''}">
                  <td class="w-12 px-4 py-2.5">
                    <input
                      type="checkbox"
                      checked={selectedUploads.has(upload.id)}
                      disabled={isUploadActive(upload)}
                      onchange={() => toggleUploadSelection(upload)}
                      class="ui-checkbox"
                      aria-label={`选择投稿「${upload.title}」`}
                      title={isUploadActive(upload) ? "投稿进行中，无法删除" : "选择投稿"}
                    />
                  </td>
                  <td class="px-4 py-2.5">
                    <button
                      class="flex w-full items-center gap-3 text-left"
                      title={editable ? "编辑投稿" : "查看投稿"}
                      onclick={() => (editingId = upload.id)}
                    >
                      <div class="flex h-10 w-16 flex-shrink-0 items-center justify-center overflow-hidden rounded-[6px] border border-line bg-sunken">
                        {#if upload.coverUrl}
                          <img src={upload.coverUrl} alt="封面" class="h-full w-full object-cover" />
                        {:else}
                          <Upload class="h-4 w-4 text-fg-faint" />
                        {/if}
                      </div>
                      <div class="min-w-0 flex-1">
                        <p class="truncate text-[12px] font-medium text-fg" title={upload.title}>{upload.title}</p>
                        <p class="flex min-w-0 items-center gap-1.5 text-[11px] text-fg-muted">
                          <span class="ui-badge shrink-0">{sourceLabels[upload.source]}</span>
                          <span class="truncate" title={upload.source_title}>
                            {upload.source_title || "来源已删除"}
                          </span>
                        </p>
                      </div>
                    </button>
                  </td>
                  <td class="px-4 py-2.5">
                    <div class="flex flex-col items-start gap-1 text-[11px]">
                      <span class="ui-badge {statusBadges[upload.status].tone}">
                        {#if isUploadActive(upload)}
                          <Loader2 class="h-3 w-3 animate-spin" />
                        {/if}
                        {statusBadges[upload.status].label}
                      </span>
                      {#if upload.bvid}
                        <a
                          href={`https://www.bilibili.com/video/${upload.bvid}`}
                          target="_blank"
                          rel="noopener noreferrer"
                          class="truncate text-accent-fg hover:underline"
                        >
                          {upload.bvid}
                        </a>
                      {:else if upload.status === "failed"}
                        <span class="max-w-full truncate text-danger-fg" title={upload.message}>{upload.message}</span>
                      {:else if editable && upload.source_state === "render"}
                        <span class="text-fg-faint">投稿时生成视频</span>
                      {:else if upload.source_state === "missing"}
                        <span class="text-danger-fg">来源已不存在</span>
                      {/if}
                    </div>
                  </td>
                  <td class="px-4 py-2.5">
                    <span class="block truncate text-[12px] {upload.uid ? 'text-fg-secondary' : 'text-fg-faint'}">
                      {upload.uid ? (accountNames.get(upload.uid) ?? upload.uid) : "未选择"}
                    </span>
                  </td>
                  <td class="px-4 py-2.5">
                    <div class="flex items-center gap-2 text-[12px] text-fg-secondary">
                      <Calendar class="h-3.5 w-3.5 shrink-0 text-fg-faint" />
                      <span class="truncate">{new Date(upload.updated_at).toLocaleString()}</span>
                    </div>
                  </td>
                  <td class="px-4 py-2.5">
                    <div class="flex items-center gap-1">
                      <button
                        class="action-button hover:bg-accent-soft"
                        title={editable ? "编辑" : "查看"}
                        onclick={() => (editingId = upload.id)}
                      >
                        {#if editable}
                          <Pencil class="h-3.5 w-3.5 text-accent-fg" />
                        {:else}
                          <Eye class="h-3.5 w-3.5 text-accent-fg" />
                        {/if}
                      </button>
                      {#if upload.bvid}
                        <a
                          class="action-button hover:bg-accent-soft"
                          title="在 B 站打开"
                          href={`https://www.bilibili.com/video/${upload.bvid}`}
                          target="_blank"
                          rel="noopener noreferrer"
                        >
                          <ExternalLink class="h-3.5 w-3.5 text-accent-fg" />
                        </a>
                      {/if}
                      <button
                        class="action-button hover:bg-danger-soft disabled:pointer-events-none disabled:opacity-40"
                        title={isUploadActive(upload) ? "投稿进行中，无法删除" : "删除"}
                        disabled={isUploadActive(upload)}
                        onclick={() => confirmDelete(upload)}
                      >
                        <Trash2 class="h-3.5 w-3.5 text-danger-fg" />
                      </button>
                    </div>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </div>
  </div>
</BatchActionLayout>

{#if showSourceDialog}
  <UploadSourceDialog onPick={createDraft} onClose={() => (showSourceDialog = false)} />
{/if}

{#if editing}
  {#key editing.id}
    <UploadEditor
      upload={editing}
      {accounts}
      onChanged={() => stateRefresh.run()}
      onClose={() => (editingId = null)}
    />
  {/key}
{/if}

{#if uploadsToDelete}
  <div class="ui-modal-backdrop">
    <div
      use:trapFocus
      class="ui-modal w-[400px] max-w-full"
      role="dialog"
      aria-modal="true"
      aria-labelledby="upload-delete-title"
      tabindex="-1"
    >
      <div class="space-y-4 p-6">
        <div class="space-y-2 text-center">
          <h3 id="upload-delete-title" class="text-[14px] font-semibold text-fg">确认删除</h3>
          <p class="text-[12px] text-fg-muted">
            {#if uploadsToDelete.length === 1}
              确定要删除投稿「{uploadsToDelete[0].title}」吗？
            {:else}
              确定要删除选中的 {uploadsToDelete.length} 条投稿记录吗？
            {/if}
          </p>
          {#if uploadsToDelete.length > 1}
            <ul class="max-h-32 overflow-auto rounded-[7px] bg-sunken px-3 py-2 text-left text-[12px] text-fg-secondary custom-scrollbar-light">
              {#each uploadsToDelete as upload (upload.id)}
                <li class="truncate" title={upload.title}>{upload.title}</li>
              {/each}
            </ul>
          {/if}
          <p class="text-[11px] text-fg-muted">
            删除投稿记录和自定义封面，素材、切片、视频文件及已发布到 B 站的视频不受影响。
          </p>
        </div>
        <DeleteConfirmActions
          oncancel={() => (uploadsToDelete = null)}
          onconfirm={deleteUploads}
        />
      </div>
    </div>
  </div>
{/if}

<style>
  .action-button {
    display: inline-flex;
    width: 26px;
    height: 26px;
    align-items: center;
    justify-content: center;
    border-radius: 7px;
    transition: background-color 120ms ease;
  }

  .action-button:focus-visible {
    outline: 2px solid rgb(var(--c-accent));
    outline-offset: 1px;
  }
</style>
