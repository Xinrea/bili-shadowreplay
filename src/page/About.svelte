<script lang="ts">
  import { onMount } from "svelte";
  import { open, TAURI_ENV } from "../lib/invoker";
  import { BookOpen, MessageCircle, Download, LoaderCircle, Heart } from "lucide-svelte";
  import { hasNewVersion, latestVersion, updateStatus, checkForUpdates, installUpdate, isNewerVersion } from "../lib/stores/version";
  let version = `v${__APP_VERSION__}`;
  let showDonateModal = $state(false);
  let releases = $state<{ version: string; date: string; description: string; url: string }[]>([]);
  let releasesError = $state("");
  let updateBusy = $derived(["checking", "downloading", "installing", "restarting"].includes($updateStatus.phase));
  let downloadPercent = $derived($updateStatus.total
    ? Math.min(100, Math.floor($updateStatus.downloaded / $updateStatus.total * 100))
    : undefined);

  // get releases from github api
  async function loadReleases(signal: AbortSignal) {
    try {
      const response = await fetch("https://api.github.com/repos/Xinrea/bili-shadowreplay/releases", { signal });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const data = await response.json();
      const stableReleases = data.filter((release) => !release.prerelease && !release.draft);
      const latest = stableReleases[0]?.tag_name;
      if (!TAURI_ENV) {
        latestVersion.set(latest || "");
        hasNewVersion.set(isNewerVersion(latest || "", version));
      }
      releases = stableReleases.slice(0, 3).map((release) => ({
        version: release.tag_name,
        date: new Date(release.published_at).toLocaleDateString(),
        description: release.body,
        url: release.html_url,
      }));
    } catch (error) {
      if (!signal.aborted) releasesError = `无法加载更新日志：${error}`;
    }
  }

  onMount(() => {
    const controller = new AbortController();
    void loadReleases(controller.signal);
    void checkForUpdates();
    return () => controller.abort();
  });

  function formatReleaseNotes(notes) {
    if (!notes) return [];
    return notes
      .split("\n")
      .filter(
        (line) => line.trim().startsWith("*") || line.trim().startsWith("-"),
      )
      .map((line) => {
        line = line.trim().replace(/^[*-]\s*/, "");
        // Remove commit hash at the end (- hash or hash)
        line = line
          .replace(/\s*-\s*[a-f0-9]{40}$/, "")
          .replace(/\s+[a-f0-9]{40}$/, "");
        return line;
      })
      .filter((line) => line.length > 0);
  }

  function toggleDonateModal() {
    showDonateModal = !showDonateModal;
  }

  function handleModalClickOutside(event) {
    const modal = document.querySelector(".mac-modal");
    if (modal && !modal.contains(event.target)) {
      showDonateModal = false;
    }
  }
</script>

<div class="flex-1 overflow-auto bg-canvas p-6 custom-scrollbar-light">
  <div class="mx-auto max-w-2xl space-y-8">
    <!-- App Info -->
    <div class="space-y-4 text-center">
      <img src="/icon.svg" alt="BiliBili ShadowReplay Logo" class="mx-auto h-20 w-20" />
      <div class="space-y-1">
        <h1 class="text-[22px] font-semibold text-fg">
          BiliBili ShadowReplay
        </h1>
        <p class="text-[12px] text-fg-muted tabular-nums">Version {version}</p>
      </div>
      <div class="space-y-2" aria-live="polite" aria-busy={updateBusy}>
        {#if TAURI_ENV}
          {#if $hasNewVersion || ["downloading", "installing", "restarting", "restart-required"].includes($updateStatus.phase)}
            <button
              class="ui-btn ui-btn-primary disabled:cursor-wait disabled:opacity-60"
              disabled={updateBusy}
              onclick={installUpdate}
            >
              {#if updateBusy}<LoaderCircle class="w-3.5 h-3.5 animate-spin" />{:else}<Download class="w-3.5 h-3.5" />{/if}
              {#if $updateStatus.phase === "downloading"}
                正在下载{downloadPercent === undefined ? "…" : ` ${downloadPercent}%`}
              {:else if $updateStatus.phase === "installing"}正在安装…
              {:else if $updateStatus.phase === "restarting"}正在重启…
              {:else if $updateStatus.phase === "restart-required"}重启应用
              {:else}更新至 {$latestVersion}{/if}
            </button>
            <p class="ui-hint">更新完成后将自动重启应用，请先结束录制和正在执行的任务。</p>
            {#if $updateStatus.phase === "downloading" && downloadPercent !== undefined}
              <progress class="h-1.5 w-48 accent-accent" value={downloadPercent} max="100" aria-label="更新下载进度"></progress>
            {/if}
          {/if}
          {#if !["downloading", "installing", "restarting", "restart-required"].includes($updateStatus.phase)}
            <button class="mx-auto block text-[12px] text-accent-fg hover:underline disabled:opacity-50" disabled={updateBusy} onclick={checkForUpdates}>
              {$updateStatus.phase === "checking" ? "正在检查更新…" : "检查更新"}
            </button>
            {#if $updateStatus.phase === "idle"}<p class="text-[11px] text-fg-muted">当前已是最新版本</p>{/if}
          {/if}
          {#if $updateStatus.error}
            <p class="text-[12px]" class:text-fg-muted={$updateStatus.phase === "check-error"} class:text-danger-fg={$updateStatus.phase !== "check-error"} role={$updateStatus.phase === "check-error" ? "status" : "alert"}>
              {$updateStatus.phase === "check-error" ? "暂时无法检查更新，请稍后重试。" : $updateStatus.phase === "restart-required" ? "更新已安装，请重试重启或手动重启应用。" : "更新失败，请重试或前往发布页下载安装。"}
            </p>
            <p class="break-words text-[11px] text-fg-faint">{$updateStatus.error}</p>
            <button class="text-[12px] text-accent-fg hover:underline" onclick={() => open("https://github.com/Xinrea/bili-shadowreplay/releases/latest")}>前往发布页</button>
          {/if}
        {:else if $hasNewVersion}
          <p class="text-[12px] text-accent-fg">发现新版本 {$latestVersion}，请更新服务端部署。</p>
        {/if}
      </div>
    </div>

    <!-- Quick Actions -->
    <div class="grid grid-cols-3 gap-3">
      <button
        class="ui-card p-4 transition-colors hover:border-line-strong hover:bg-control-hover/50"
        onclick={() => {
          // tauri open url
          open("https://bsr.xinrea.cn/");
        }}
      >
        <div class="flex flex-col items-center space-y-2">
          <div
            class="flex h-9 w-9 items-center justify-center rounded-full bg-accent-soft text-accent-fg"
          >
            <BookOpen class="h-[18px] w-[18px]" />
          </div>
          <span class="text-[13px] font-medium text-fg"
            >说明</span
          >
        </div>
      </button>
      <button
        class="ui-card p-4 transition-colors hover:border-line-strong hover:bg-control-hover/50"
        onclick={() => {
          // tauri open url
          open("https://qm.qq.com/q/v4lrE6gyum");
        }}
      >
        <div class="flex flex-col items-center space-y-2">
          <div
            class="flex h-9 w-9 items-center justify-center rounded-full bg-accent-soft text-accent-fg"
          >
            <MessageCircle class="h-[18px] w-[18px]" />
          </div>
          <span class="text-[13px] font-medium text-fg"
            >反馈交流群</span
          >
        </div>
      </button>
      <button
        class="ui-card p-4 transition-colors hover:border-line-strong hover:bg-control-hover/50"
        onclick={toggleDonateModal}
      >
        <div class="flex flex-col items-center space-y-2">
          <div
            class="flex h-9 w-9 items-center justify-center rounded-full bg-pink-500/15 text-pink-500"
          >
            <Heart class="h-[18px] w-[18px]" />
          </div>
          <span class="text-[13px] font-medium text-fg"
            >打赏支持</span
          >
        </div>
      </button>
    </div>

    <!-- What's New -->
    <div class="space-y-3">
      <h2 class="ui-section-title">
        What's New
      </h2>
      {#if releasesError}<p class="text-[12px] text-fg-muted" role="status">{releasesError}</p>{/if}
      <div class="ui-card overflow-hidden">
        {#each releases as release}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            class="cursor-pointer p-4 transition-colors hover:bg-control-hover/50 {release !== releases[releases.length - 1]
              ? 'border-b border-line'
              : ''}"
            role="button"
            tabindex="0"
            onclick={() => {
              open(release.url);
            }}
          >
            <div class="flex items-center justify-between gap-3">
              <h3 class="text-[13px] font-medium text-fg">
                Version {release.version}
              </h3>
              <span class="text-[11px] text-fg-muted"
                >Released on {release.date}</span
              >
            </div>
            <ul class="mt-2 space-y-1 text-[12px] text-fg-secondary">
              {#each formatReleaseNotes(release.description) as note}
                <li class="flex items-start space-x-2">
                  <span class="text-accent">•</span>
                  <span>{note}</span>
                </li>
              {/each}
            </ul>
          </div>
        {/each}
      </div>
    </div>
  </div>
</div>

{#if showDonateModal}
  <div
    class="ui-modal-backdrop"
    style="position: absolute; min-height: 100%; width: 100%; top: 0; left: 0;"
  >
    <div class="mac-modal ui-modal mx-4 w-full max-w-md">
      <div class="ui-modal-header">
        <h3>
          打赏支持
        </h3>
        <button
          class="ui-btn ui-btn-ghost ui-btn-sm ui-btn-icon -mr-1.5 text-fg-muted"
          onclick={toggleDonateModal}
        >
          ✕
        </button>
      </div>
      <div class="ui-modal-body">
        <div class="flex justify-center">
          <img
            src="/imgs/donate.png"
            class="h-auto max-w-full rounded-[8px]"
            alt="打赏二维码"
          />
        </div>
        <p class="mt-3 text-center text-[12px] text-fg-secondary">
          感谢您的支持！
        </p>
      </div>
    </div>
  </div>
{/if}

<svelte:window onmousedown={handleModalClickOutside} />
