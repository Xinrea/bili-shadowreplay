<script lang="ts">
  import {
    AlertCircle,
    CheckCircle2,
    Clapperboard,
    ExternalLink,
    FileVideo,
    Film,
    ListChecks,
    Loader2,
    Pen,
    Upload,
    X,
  } from "lucide-svelte";
  import { onDestroy } from "svelte";
  import { get_static_url, invoke } from "../../invoker";
  import { generateEventId, type OverlayImage, type ProjectItem, type UploadItem } from "../../interface";
  import { frameRateQuery, parseTimeline } from "../../project-timeline";
  import { announceTask, openTaskCenter, settleTask } from "../../stores/tasks";
  import {
    parseProfile,
    projectSubtitleAss,
    saveRoomTemplate,
    sourceDescription,
    sourceLabels,
  } from "../../uploads";
  import CoverEditor from "../cover/CoverEditor.svelte";
  import { stickerRefUrl } from "../cover/custom-stickers";
  import { renderOverlayImages, StickerLoadError } from "../project/overlay-render";
  import TypeSelect from "../TypeSelect.svelte";

  interface Props {
    upload: UploadItem;
    accounts: { uid: string; name: string }[];
    /** Reload `upload` from the backend. */
    onChanged: () => Promise<void>;
    onClose: () => void;
  }

  let { upload, accounts, onChanged, onClose }: Props = $props();

  // Edited locally and saved shortly after each change; the editor is
  // recreated for each upload.
  const initial = () => ({ profile: parseProfile(upload.profile), uid: upload.uid });
  let profile = $state(initial().profile);
  let uid = $state(initial().uid);
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let saving: Promise<void> | null = null;
  let saveError = $state("");
  let posting = $state(false);
  let showCoverEditor = $state(false);
  let coverUrl = $state("");
  let coverFailed = $state(false);
  let videoUrl = $state("");

  let editable = $derived(upload.status === "draft" || upload.status === "failed");
  let savingDanmu = $state(false);
  // Turning it off stays possible even when there is nothing to burn.
  let canToggleDanmu = $derived(
    editable && !posting && (upload.danmu_available || upload.danmu) && !savingDanmu,
  );
  let active = $derived(upload.status === "exporting" || upload.status === "posting");
  let blocker = $derived(
    upload.source_state === "missing"
      ? "来源已不存在"
      : !profile.title.trim()
        ? "请填写标题"
        : !uid
          ? "请选择投稿账号"
          : "",
  );
  let SourceIcon = $derived(
    upload.source === "material" ? Film : upload.source === "project" ? Clapperboard : FileVideo,
  );

  $effect(() => {
    const cover = upload.display_cover;
    coverFailed = false;
    if (!cover) {
      coverUrl = "";
      return;
    }
    // Each saved cover is a new file, so its URL changes with it.
    void get_static_url("output", cover).then((url) => (coverUrl = url));
  });

  $effect(() => {
    const video = upload.video;
    if (!video) {
      videoUrl = "";
      return;
    }
    void get_static_url(video.base, video.path).then((url) => (videoUrl = url));
  });

  onDestroy(() => {
    if (saveTimer) {
      clearTimeout(saveTimer);
      void save().catch(() => {});
    }
  });

  /** Save the form, after any save already in flight so the newest lands last. */
  async function save() {
    saveTimer = null;
    while (saving) await saving;
    const snapshot = $state.snapshot(profile);
    const request = invoke("update_upload", { id: upload.id, uid, profile: snapshot });
    saving = request.then(
      () => {},
      () => {},
    ).finally(() => (saving = null));
    try {
      await request;
      saveError = "";
    } catch (error) {
      saveError = String(error);
      throw error;
    }
    saveRoomTemplate(upload.room_id, snapshot);
  }

  function scheduleSave() {
    if (!editable) return;
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => void save().catch(() => {}), 600);
  }

  // Saved at once on its own: a project shares it with its editor, so the
  // debounced form save must not carry a stale value.
  async function setDanmu(input: HTMLInputElement) {
    savingDanmu = true;
    try {
      await invoke("update_upload_danmu", { id: upload.id, danmu: input.checked });
      await onChanged();
    } catch (error) {
      input.checked = upload.danmu;
      alert("修改弹幕设置失败：" + error);
    } finally {
      savingDanmu = false;
    }
  }

  async function saveCover(covers: { cover: string; cover43: string }) {
    await invoke("update_upload_cover", { id: upload.id, ...covers });
    // The previous file is gone; show the new one now.
    await onChanged();
  }

  async function post() {
    if (!editable || blocker || posting || savingDanmu) return;
    posting = true;
    const eventId = generateEventId();
    try {
      // Waits for saves in flight: none may land once it is posting.
      if (saveTimer) clearTimeout(saveTimer);
      await save();
      // A project's video is rendered when posting, with its subtitles and
      // its texts and stickers as its editor draws them.
      let subtitleAss: string | null = null;
      let overlays: OverlayImage[] | null = null;
      if (upload.source === "project") {
        const project = await invoke<ProjectItem>("get_project", { id: upload.source_id });
        const timeline = parseTimeline(project.timeline);
        // Its subtitles on the frames the render has, as its editor puts them.
        const fps = timeline.clips.length
          ? await invoke<number>("get_project_frame_rate", frameRateQuery(timeline))
          : 0;
        subtitleAss = projectSubtitleAss(project, fps);
        try {
          overlays = await renderOverlayImages(timeline, stickerRefUrl);
        } catch (error) {
          throw error instanceof StickerLoadError
            ? new Error("切片中有贴纸没有加载出来，请在切片编辑窗口中删除后再投稿")
            : error;
        }
      }
      announceTask({
        id: eventId,
        task_type: "upload",
        metadata: JSON.stringify({ upload_id: upload.id, title: profile.title }),
      });
      await invoke("post_upload", { eventId, id: upload.id, subtitleAss, overlays });
      openTaskCenter(eventId);
      // Show the attempt as running before allowing another click.
      await onChanged();
    } catch (error) {
      alert("投稿失败：" + (error instanceof Error ? error.message : error));
    } finally {
      posting = false;
      settleTask(eventId);
    }
  }

  function close() {
    if (saveTimer) {
      clearTimeout(saveTimer);
      void save().catch(() => {});
    }
    onClose();
  }
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key === "Escape" && !showCoverEditor) {
      event.preventDefault();
      close();
    }
  }}
/>

<!-- Above full-window editors such as the clip editor (z-1000). -->
<div class="ui-modal-backdrop z-[1100]">
  <div
    class="ui-modal max-h-[90vh] w-[820px] max-w-full"
    role="dialog"
    aria-modal="true"
    aria-labelledby="upload-editor-title"
  >
    <div class="ui-modal-header">
      <h3 id="upload-editor-title" class="flex min-w-0 items-center gap-2">
        <Upload class="h-4 w-4 shrink-0 text-accent-fg" />
        <span class="truncate">{editable ? "编辑投稿" : "投稿详情"}</span>
      </h3>
      <button
        type="button"
        class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost -mr-1.5"
        aria-label="关闭"
        onclick={close}
      >
        <X class="h-4 w-4" />
      </button>
    </div>

    <div class="ui-modal-body custom-scrollbar-light space-y-4">
      {#if upload.status === "posted"}
        <div class="flex items-center gap-2.5 rounded-[10px] border border-success/40 bg-success-soft px-3.5 py-2.5 text-[12px] text-success-fg">
          <CheckCircle2 class="h-4 w-4 shrink-0" />
          <span class="min-w-0 flex-1">已投稿，B 站稿件审核通过后即可观看。</span>
          <a
            href={`https://www.bilibili.com/video/${upload.bvid}`}
            target="_blank"
            rel="noopener noreferrer"
            class="inline-flex items-center gap-1 font-medium hover:underline"
          >
            {upload.bvid}
            <ExternalLink class="h-3 w-3" />
          </a>
        </div>
      {:else if active}
        <div class="flex items-center gap-2.5 rounded-[10px] border border-accent/40 bg-accent-soft px-3.5 py-2.5 text-[12px] text-accent-fg">
          <Loader2 class="h-4 w-4 shrink-0 animate-spin" />
          <span class="min-w-0 flex-1">
            {upload.status === "exporting" ? "正在生成切片视频，完成后会自动投稿。" : "正在投稿，关闭窗口不会中断。"}
          </span>
          <button type="button" class="ui-btn ui-btn-sm" onclick={() => openTaskCenter()}>
            <ListChecks class="h-3.5 w-3.5" />
            查看进度
          </button>
        </div>
      {:else if upload.status === "failed" && upload.message}
        <div class="flex items-start gap-2.5 rounded-[10px] border border-danger/40 bg-danger-soft px-3.5 py-2.5 text-[12px] text-danger-fg">
          <AlertCircle class="mt-px h-4 w-4 shrink-0" />
          <span class="min-w-0 flex-1 break-words">上次投稿失败：{upload.message}</span>
        </div>
      {/if}

      <div class="grid grid-cols-[280px_minmax(0,1fr)] gap-5">
        <div class="space-y-4">
          <section class="space-y-2">
            <div class="flex items-center justify-between">
              <span class="ui-label">视频封面</span>
              {#if editable}
                <button type="button" class="ui-btn ui-btn-sm" disabled={posting} onclick={() => (showCoverEditor = true)}>
                  <Pen class="h-3 w-3" />
                  编辑封面
                </button>
              {/if}
            </div>
            <div class="aspect-video overflow-hidden rounded-[10px] border border-line bg-sunken">
              {#if coverUrl && !coverFailed}
                <img src={coverUrl} alt="视频封面" class="h-full w-full object-cover" onerror={() => (coverFailed = true)} />
              {:else}
                <div class="flex h-full items-center justify-center text-[11px] text-fg-faint">暂无封面</div>
              {/if}
            </div>
          </section>

          <section class="space-y-2">
            <span class="ui-label">投稿来源</span>
            <div class="ui-card space-y-2 bg-control/50 p-3">
              <div class="flex min-w-0 items-center gap-2">
                <SourceIcon class="h-4 w-4 shrink-0 text-fg-muted" />
                <span class="ui-badge">{sourceLabels[upload.source]}</span>
                <span class="min-w-0 truncate text-[12px] font-medium text-fg" title={upload.source_title}>
                  {upload.source_title || "已删除"}
                </span>
              </div>
              {#if upload.source === "file"}
                <p class="break-all font-mono text-[11px] text-fg-muted">{upload.file}</p>
              {/if}
              <p class="text-[11px] leading-relaxed {upload.source_state === 'missing' ? 'text-danger-fg' : 'text-fg-muted'}">
                {sourceDescription(upload)}
              </p>
              {#if upload.source !== "file" && upload.source_state !== "missing" && !upload.danmu_embedded}
                <label
                  class="flex items-start justify-between gap-3 border-t border-line pt-2.5 {canToggleDanmu
                    ? 'cursor-pointer'
                    : ''}"
                >
                  <span class="min-w-0">
                    <span class="block text-[12px] font-medium text-fg">压制弹幕</span>
                    <span class="ui-hint block">
                      {!upload.danmu_available
                        ? upload.source === "project"
                          ? "切片使用的素材都没有保存弹幕"
                          : "这个素材没有保存弹幕"
                        : upload.source === "project"
                          ? "与切片编辑窗口的弹幕设置相同"
                          : "开启后投稿压制了弹幕的视频"}
                    </span>
                  </span>
                  <span class="ui-switch mt-0.5">
                    <input
                      type="checkbox"
                      checked={upload.danmu}
                      disabled={!canToggleDanmu}
                      onchange={(event) => setDanmu(event.currentTarget)}
                    />
                    <span></span>
                  </span>
                </label>
              {/if}
            </div>
          </section>
        </div>

        <fieldset
          class="min-w-0 space-y-3.5 disabled:opacity-70"
          disabled={!editable || posting}
          oninput={scheduleSave}
          onchange={scheduleSave}
        >
          <label class="block space-y-1.5">
            <span class="ui-label">标题</span>
            <input type="text" bind:value={profile.title} placeholder="输入视频标题" maxlength="80" class="ui-input w-full" />
          </label>
          <div class="space-y-1.5">
            <span class="ui-label">视频分区</span>
            <TypeSelect bind:value={profile.tid} disabled={!editable} />
          </div>
          <label class="block space-y-1.5">
            <span class="ui-label">投稿账号</span>
            <select bind:value={uid} class="ui-input w-full">
              <option value="">选择账号</option>
              {#each accounts as account (account.uid)}
                <option value={account.uid}>{account.name}</option>
              {/each}
            </select>
            {#if accounts.length === 0}
              <span class="ui-hint block">还没有 B 站账号，请先在「账号」页登录。</span>
            {/if}
          </label>
          <label class="block space-y-1.5">
            <span class="ui-label">描述</span>
            <textarea bind:value={profile.desc} placeholder="输入视频描述" rows="3" class="ui-input w-full resize-none"></textarea>
          </label>
          <label class="block space-y-1.5">
            <span class="ui-label">标签</span>
            <input type="text" bind:value={profile.tag} placeholder="输入视频标签，用逗号分隔" class="ui-input w-full" />
          </label>
          <label class="block space-y-1.5">
            <span class="ui-label">动态</span>
            <textarea bind:value={profile.dynamic} placeholder="输入动态内容" rows="2" class="ui-input w-full resize-none"></textarea>
          </label>
        </fieldset>
      </div>
    </div>

    <div class="ui-modal-footer justify-between">
      <span class="min-w-0 truncate text-[11px] {saveError ? 'text-danger-fg' : 'text-fg-muted'}">
        {#if saveError}
          保存失败：{saveError}
        {:else if editable}
          修改会自动保存为草稿
        {/if}
      </span>
      <div class="flex shrink-0 gap-2">
        <button type="button" class="ui-btn" onclick={close}>关闭</button>
        {#if editable}
          <button
            type="button"
            class="ui-btn ui-btn-primary"
            disabled={Boolean(blocker) || posting || savingDanmu}
            title={blocker || undefined}
            onclick={post}
          >
            {#if posting}
              <Loader2 class="h-3.5 w-3.5 animate-spin" />
            {:else}
              <Upload class="h-3.5 w-3.5" />
            {/if}
            {upload.status === "failed" ? "重新投稿" : "投稿"}
          </button>
        {/if}
      </div>
    </div>
  </div>
</div>

<CoverEditor
  bind:show={showCoverEditor}
  video={videoUrl ? { file: videoUrl } : null}
  image={coverUrl}
  title={profile.title}
  onSave={saveCover}
/>
