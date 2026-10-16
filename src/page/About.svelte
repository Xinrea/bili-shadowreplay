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

<div class="flex-1 p-6 overflow-auto custom-scrollbar-light bg-gray-50">
  <div class="max-w-2xl mx-auto space-y-8">
    <!-- App Info -->
    <div class="text-center space-y-4">
      <img src="/icon.svg" alt="BiliBili ShadowReplay Logo" class="w-24 h-24 mx-auto" />
      <div>
        <h1 class="text-2xl font-semibold text-gray-900 dark:text-white">
          BiliBili ShadowReplay
        </h1>
        <p class="text-gray-500 dark:text-gray-400">Version {version}</p>
      </div>
      <div class="space-y-2" aria-live="polite" aria-busy={updateBusy}>
        {#if TAURI_ENV}
          {#if $hasNewVersion || ["downloading", "installing", "restarting", "restart-required"].includes($updateStatus.phase)}
            <button
              class="inline-flex items-center gap-2 rounded-lg bg-blue-500 px-4 py-2 text-sm font-medium text-white hover:bg-blue-600 disabled:opacity-60 disabled:cursor-wait"
              disabled={updateBusy}
              onclick={installUpdate}
            >
              {#if updateBusy}<LoaderCircle class="w-4 h-4 animate-spin" />{:else}<Download class="w-4 h-4" />{/if}
              {#if $updateStatus.phase === "downloading"}
                正在下载{downloadPercent === undefined ? "…" : ` ${downloadPercent}%`}
              {:else if $updateStatus.phase === "installing"}正在安装…
              {:else if $updateStatus.phase === "restarting"}正在重启…
              {:else if $updateStatus.phase === "restart-required"}重启应用
              {:else}更新至 {$latestVersion}{/if}
            </button>
            <p class="text-xs text-gray-500">更新完成后将自动重启应用，请先结束录制和正在执行的任务。</p>
            {#if $updateStatus.phase === "downloading" && downloadPercent !== undefined}
              <progress class="w-48 h-1.5 accent-blue-500" value={downloadPercent} max="100" aria-label="更新下载进度"></progress>
            {/if}
          {/if}
          {#if !["downloading", "installing", "restarting", "restart-required"].includes($updateStatus.phase)}
            <button class="block mx-auto text-sm text-blue-500 hover:text-blue-600 disabled:opacity-50" disabled={updateBusy} onclick={checkForUpdates}>
              {$updateStatus.phase === "checking" ? "正在检查更新…" : "检查更新"}
            </button>
            {#if $updateStatus.phase === "idle"}<p class="text-xs text-gray-500">当前已是最新版本</p>{/if}
          {/if}
          {#if $updateStatus.error}
            <p class="text-sm" class:text-gray-500={$updateStatus.phase === "check-error"} class:text-red-500={$updateStatus.phase !== "check-error"} role={$updateStatus.phase === "check-error" ? "status" : "alert"}>
              {$updateStatus.phase === "check-error" ? "暂时无法检查更新，请稍后重试。" : $updateStatus.phase === "restart-required" ? "更新已安装，请重试重启或手动重启应用。" : "更新失败，请重试或前往发布页下载安装。"}
            </p>
            <p class="text-xs text-gray-500 break-words">{$updateStatus.error}</p>
            <button class="text-sm text-blue-500 hover:text-blue-600" onclick={() => open("https://github.com/Xinrea/bili-shadowreplay/releases/latest")}>前往发布页</button>
          {/if}
        {:else if $hasNewVersion}
          <p class="text-sm text-blue-500">发现新版本 {$latestVersion}，请更新服务端部署。</p>
        {/if}
      </div>
    </div>

    <!-- Quick Actions -->
    <div class="grid grid-cols-3 gap-4">
      <button
        class="p-4 rounded-xl bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-700/50 transition-colors"
        onclick={() => {
          // tauri open url
          open("https://bsr.xinrea.cn/");
        }}
      >
        <div class="flex flex-col items-center space-y-2">
          <div
            class="w-10 h-10 rounded-full bg-blue-500/10 flex items-center justify-center"
          >
            <BookOpen class="w-5 h-5 icon-primary" />
          </div>
          <span class="text-sm font-medium text-gray-900 dark:text-white"
            >说明</span
          >
        </div>
      </button>
      <button
        class="p-4 rounded-xl bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-700/50 transition-colors"
        onclick={() => {
          // tauri open url
          open("https://qm.qq.com/q/v4lrE6gyum");
        }}
      >
        <div class="flex flex-col items-center space-y-2">
          <div
            class="w-10 h-10 rounded-full bg-blue-500/10 flex items-center justify-center"
          >
            <MessageCircle class="w-5 h-5 icon-primary" />
          </div>
          <span class="text-sm font-medium text-gray-900 dark:text-white"
            >反馈交流群</span
          >
        </div>
      </button>
      <button
        class="p-4 rounded-xl bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-700/50 transition-colors"
        onclick={toggleDonateModal}
      >
        <div class="flex flex-col items-center space-y-2">
          <div
            class="w-10 h-10 rounded-full bg-pink-500/10 flex items-center justify-center"
          >
            <Heart class="w-5 h-5 text-pink-500" />
          </div>
          <span class="text-sm font-medium text-gray-900 dark:text-white"
            >打赏支持</span
          >
        </div>
      </button>
    </div>

    <!-- What's New -->
    <div class="space-y-4">
      <h2 class="text-lg font-medium text-gray-900 dark:text-white">
        What's New
      </h2>
      {#if releasesError}<p class="text-sm text-gray-500" role="status">{releasesError}</p>{/if}
      <div
        class="bg-white dark:bg-[#3c3c3e] rounded-xl border border-gray-200 dark:border-gray-700"
      >
        {#each releases as release}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            class="p-4 cursor-pointer {release !== releases[releases.length - 1]
              ? 'border-b border-gray-200 dark:border-gray-700'
              : ''}"
            role="button"
            tabindex="0"
            onclick={() => {
              open(release.url);
            }}
          >
            <div class="flex items-center justify-between">
              <h3 class="text-sm font-medium text-gray-900 dark:text-white">
                Version {release.version}
              </h3>
              <span class="text-xs text-gray-500 dark:text-gray-400"
                >Released on {release.date}</span
              >
            </div>
            <ul class="mt-2 space-y-1 text-sm text-gray-600 dark:text-gray-300">
              {#each formatReleaseNotes(release.description) as note}
                <li class="flex items-start space-x-2">
                  <span class="text-blue-500">•</span>
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
    class="absolute inset-0 bg-black bg-opacity-50 flex items-center justify-center"
    style="position: absolute; min-height: 100%; width: 100%; top: 0; left: 0;"
  >
    <div
      class="bg-white dark:bg-[#3c3c3e] rounded-lg p-6 max-w-md w-full mx-4 mac-modal"
    >
      <div class="flex justify-between items-center mb-4">
        <h3 class="text-lg font-medium text-gray-900 dark:text-white">
          打赏支持
        </h3>
        <button
          class="text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200"
          onclick={toggleDonateModal}
        >
          ✕
        </button>
      </div>
      <div class="flex justify-center">
        <img
          src="/imgs/donate.png"
          class="max-w-full h-auto rounded-lg"
          alt="打赏二维码"
        />
      </div>
      <p class="mt-4 text-center text-sm text-gray-600 dark:text-gray-300">
        感谢您的支持！
      </p>
    </div>
  </div>
{/if}

<svelte:window onmousedown={handleModalClickOutside} />
