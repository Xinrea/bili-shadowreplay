<script type="ts">
  import { getVersion } from "@tauri-apps/api/app";
  import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
  import { open } from "@tauri-apps/plugin-shell";
  import { BookOpen, MessageCircle, Video } from "lucide-svelte";
  const appWindow = getCurrentWebviewWindow();
  let version = "";
  getVersion().then((v) => {
    version = v;
    appWindow.setTitle(`BiliBili ShadowReplay - v${version}`);
    console.log(version);
  });
  let latest_version = "";
  let releases = [];

  // get releases from github api
  fetch("https://api.github.com/repos/Xinrea/bili-shadowreplay/releases")
    .then((response) => response.json())
    .then((data) => {
      latest_version = data[0].tag_name;
      releases = data.slice(0, 3).map(release => ({
        version: release.tag_name,
        date: new Date(release.published_at).toLocaleDateString(),
        description: release.body
      }));
    });

  function formatReleaseNotes(notes) {
    if (!notes) return [];
    return notes.split('\n')
      .filter(line => line.trim().startsWith('*') || line.trim().startsWith('-'))
      .map(line => {
        line = line.trim().replace(/^[*-]\s*/, '');
        // Remove commit hash at the end (- hash or hash)
        line = line.replace(/\s*-\s*[a-f0-9]{40}$/, '').replace(/\s+[a-f0-9]{40}$/, '');
        return line;
      })
      .filter(line => line.length > 0);
  }
</script>

<div class="flex-1 p-6">
  <div class="max-w-2xl mx-auto space-y-8">
    <!-- App Info -->
    <div class="text-center space-y-4">
      <div
        class="w-24 h-24 mx-auto bg-gradient-to-br from-blue-500 to-purple-600 rounded-2xl shadow-lg flex items-center justify-center"
      >
        <Video class="w-12 h-12 icon-white" />
      </div>
      <div>
        <h1 class="text-2xl font-semibold text-gray-900 dark:text-white">
          BiliBili ShadowReplay
        </h1>
        <p class="text-gray-500 dark:text-gray-400">Version {version}</p>
      </div>
    </div>

    <!-- Quick Actions -->
    <div class="grid grid-cols-2 gap-4">
      <button
        class="p-4 rounded-xl bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-700/50 transition-colors"
        on:click={() => {
          // tauri open url
          open("https://github.com/Xinrea/bili-shadowreplay/wiki");
        }}
      >
        <div class="flex flex-col items-center space-y-2">
          <div
            class="w-10 h-10 rounded-full bg-blue-500/10 flex items-center justify-center"
          >
            <BookOpen class="w-5 h-5 icon-primary" />
          </div>
          <span class="text-sm font-medium text-gray-900 dark:text-white"
            >Documentation</span
          >
        </div>
      </button>
      <button
        class="p-4 rounded-xl bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-700/50 transition-colors"
        on:click={() => {
          // tauri open url
          open("https://github.com/Xinrea/bili-shadowreplay/issues");
        }}
      >
        <div class="flex flex-col items-center space-y-2">
          <div
            class="w-10 h-10 rounded-full bg-blue-500/10 flex items-center justify-center"
          >
            <MessageCircle class="w-5 h-5 icon-primary" />
          </div>
          <span class="text-sm font-medium text-gray-900 dark:text-white"
            >Support</span
          >
        </div>
      </button>
    </div>

    <!-- What's New -->
    <div class="space-y-4">
      <h2 class="text-lg font-medium text-gray-900 dark:text-white">
        What's New
      </h2>
      <div
        class="bg-white dark:bg-[#3c3c3e] rounded-xl border border-gray-200 dark:border-gray-700"
      >
        {#each releases as release}
          <div class="p-4 {release !== releases[releases.length - 1] ? 'border-b border-gray-200 dark:border-gray-700' : ''}">
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