<script lang="ts">
  import { Loader2, Pen, Upload } from "lucide-svelte";
  import { onDestroy, onMount } from "svelte";
  import { invoke, get_static_url } from "../../invoker";
  import {
    default_profile,
    generateEventId,
    type Profile,
    type ProjectExportState,
    type ProjectItem,
  } from "../../interface";
  import type { AccountInfo } from "../../db";
  import { announceTask, openTaskCenter, settleTask } from "../../stores/tasks";
  import CoverEditor from "../CoverEditor.svelte";
  import TypeSelect from "../TypeSelect.svelte";

  interface Props {
    project: ProjectItem;
    exportState: ProjectExportState;
    /** Video the cover editor grabs frames from. */
    coverVideoUrl: string;
    exporting: boolean;
    onExport: () => void;
    onChanged: () => void;
  }

  let { project, exportState, coverVideoUrl, exporting, onExport, onChanged }: Props = $props();

  /** Upload form for a project: its own, or a copy of the room's last one. */
  function initialProfile(): Profile {
    const fallback = default_profile();
    let profile = fallback;
    try {
      if (project.profile) {
        profile = { ...fallback, ...JSON.parse(project.profile) };
      } else {
        const template = localStorage.getItem("profile-" + project.room_id);
        if (template?.includes("videos")) profile = { ...fallback, ...JSON.parse(template) };
      }
    } catch {
      profile = fallback;
    }
    if (!project.profile && project.title) profile.title = project.title;
    return { ...profile, videos: [] };
  }

  let profile: Profile = $state(initialProfile());
  let accounts: { value: string; name: string }[] = $state([]);
  let uid = $state("");
  let posting: string | null = $state(null);
  let showCoverEditor = $state(false);
  let coverUrl = $state("");
  let coverFailed = $state(false);
  let saveTimer: ReturnType<typeof setTimeout> | null = null;

  let canPost = $derived(exportState === "material" || exportState === "ready");

  onMount(async () => {
    try {
      const info: AccountInfo = await invoke("get_accounts");
      accounts = info.accounts
        .filter((account) => account.platform === "bilibili")
        .map((account) => ({ value: String(account.uid), name: account.name }));
    } catch (error) {
      console.error("Failed to load accounts:", error);
    }
  });

  // Keep edits typed just before switching tabs or closing the window.
  onDestroy(() => {
    if (saveTimer) {
      clearTimeout(saveTimer);
      void saveProfile();
    }
  });

  $effect(() => {
    const cover = project.display_cover;
    coverFailed = false;
    if (!cover) {
      coverUrl = "";
      return;
    }
    // Bust the cache: the project cover keeps its file name when replaced.
    void get_static_url("output", cover).then((url) => (coverUrl = `${url}?t=${encodeURIComponent(project.updated_at)}`));
  });

  async function saveProfile() {
    saveTimer = null;
    const snapshot = $state.snapshot(profile);
    try {
      await invoke("update_project_profile", { id: project.id, profile: snapshot });
      // The room keeps the latest form as a template for its next project.
      localStorage.setItem("profile-" + project.room_id, JSON.stringify(snapshot));
    } catch (error) {
      console.error("Failed to save upload profile:", error);
    }
  }

  function scheduleSave() {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(saveProfile, 600);
  }

  async function post() {
    if (posting || !uid || !canPost) return;
    if (saveTimer) clearTimeout(saveTimer);
    await saveProfile();
    const eventId = generateEventId();
    posting = eventId;
    announceTask({
      id: eventId,
      task_type: "upload_project",
      metadata: JSON.stringify({ project_id: project.id, profile: { title: profile.title } }),
    });
    openTaskCenter(eventId);
    try {
      const bvid = await invoke<string>("upload_project", {
        eventId,
        uid,
        id: project.id,
        profile: $state.snapshot(profile),
      });
      onChanged();
      alert(`投稿成功：${bvid}`);
    } catch (error) {
      alert("投稿失败：" + error);
    } finally {
      posting = null;
      settleTask(eventId);
    }
  }

  async function saveCover(cover: string) {
    await invoke("update_project_cover", { id: project.id, cover });
    onChanged();
  }
</script>

<div class="upload-form" oninput={scheduleSave} onchange={scheduleSave}>
  {#if project.bvid}
    <div class="posted">
      已投稿：
      <a href={`https://www.bilibili.com/video/${project.bvid}`} target="_blank" rel="noopener noreferrer">
        {project.bvid}
      </a>
    </div>
  {/if}

  <section>
    <div class="section-heading">
      <h3>视频封面</h3>
      <button type="button" class="link-button" onclick={() => (showCoverEditor = true)}>
        <Pen class="h-3 w-3" />
        创建新封面
      </button>
    </div>
    <div class="cover-frame">
      {#if coverUrl && !coverFailed}
        <img src={coverUrl} alt="视频封面" onerror={() => (coverFailed = true)} />
      {:else}
        <div class="cover-placeholder">暂无封面</div>
      {/if}
    </div>
  </section>

  <section>
    <div class="section-heading"><h3>基本信息</h3></div>
    <div class="field">
      <label for="upload-title">标题</label>
      <input id="upload-title" type="text" bind:value={profile.title} placeholder="输入视频标题" class="panel-input" />
    </div>
    <div class="field">
      <label for="upload-tid">视频分区</label>
      <div class="w-full" id="upload-tid">
        <TypeSelect bind:value={profile.tid} />
      </div>
    </div>
    <div class="field">
      <label for="upload-uid">投稿账号</label>
      <select id="upload-uid" bind:value={uid} class="panel-input">
        <option value="">选择账号</option>
        {#each accounts as account}
          <option value={account.value}>{account.name}</option>
        {/each}
      </select>
    </div>
  </section>

  <section>
    <div class="section-heading"><h3>详细信息</h3></div>
    <div class="field">
      <label for="upload-desc">描述</label>
      <textarea id="upload-desc" bind:value={profile.desc} placeholder="输入视频描述" class="panel-input h-20 resize-none"></textarea>
    </div>
    <div class="field">
      <label for="upload-tag">标签</label>
      <input id="upload-tag" type="text" bind:value={profile.tag} placeholder="输入视频标签，用逗号分隔" class="panel-input" />
    </div>
    <div class="field">
      <label for="upload-dynamic">动态</label>
      <textarea id="upload-dynamic" bind:value={profile.dynamic} placeholder="输入动态内容" class="panel-input h-20 resize-none"></textarea>
    </div>
  </section>

  {#if !canPost}
    <div class="notice">
      {exportState === "stale" ? "切片在导出后又被修改过，需要重新导出后才能投稿。" : "切片需要先导出才能投稿。"}
      <button type="button" class="footer-button" disabled={exporting} onclick={onExport}>
        {#if exporting}
          <Loader2 class="h-3.5 w-3.5 animate-spin" />
        {/if}
        导出切片
      </button>
    </div>
  {/if}

  <button
    type="button"
    class="footer-button primary w-full justify-center"
    onclick={post}
    disabled={posting !== null || !uid || !canPost}
    title={!uid ? "请先选择投稿账号" : undefined}
  >
    {#if posting}
      <Loader2 class="h-3.5 w-3.5 animate-spin" />
      投稿中
    {:else}
      <Upload class="h-3.5 w-3.5" />
      投稿
    {/if}
  </button>
</div>

<CoverEditor bind:show={showCoverEditor} video={{ file: coverVideoUrl }} onSave={saveCover} />

<style>
  .upload-form {
    display: flex;
    flex-direction: column;
    gap: 18px;
  }

  .upload-form section {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .posted {
    border: 1px solid rgb(48 209 88 / 35%);
    border-radius: 8px;
    background: rgb(48 209 88 / 10%);
    padding: 8px 10px;
    color: #a7f3d0;
    font-size: 11px;
  }

  .posted a {
    color: #55b9ff;
  }

  .notice {
    display: flex;
    flex-direction: column;
    gap: 8px;
    border: 1px solid rgb(255 159 10 / 35%);
    border-radius: 8px;
    background: rgb(255 159 10 / 10%);
    padding: 10px;
    color: #fcd9a6;
    font-size: 11px;
  }

  .section-heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .section-heading h3 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
  }

  .link-button {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: #55b9ff;
    font-size: 10px;
    font-weight: 500;
  }

  .cover-frame {
    overflow: hidden;
    border: 1px solid #293442;
    border-radius: 9px;
    background: #0c1016;
  }

  .cover-frame img {
    width: 100%;
  }

  .cover-placeholder {
    display: flex;
    aspect-ratio: 16 / 9;
    align-items: center;
    justify-content: center;
    color: #657186;
    font-size: 10px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .field label {
    color: #9aa6b8;
    font-size: 10px;
  }
</style>
