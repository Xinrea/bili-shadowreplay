<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";

  import type { Config } from "../lib/interface";
  import { Bell, HardDrive } from "lucide-svelte";

  let setting_model: Config = {
    cache: "",
    output: "",
    primary_uid: 0,
    live_start_notify: true,
    live_end_notify: true,
    clip_notify: true,
    post_notify: true,
    auto_cleanup: true,
  };

  async function get_config() {
    let config: Config = await invoke("get_config");
    setting_model = config;
    console.log(config);
  }

  async function browse_folder() {
    const selected = await open({ directory: true });
    return Array.isArray(selected) ? selected[0] : selected;
  }

  async function update_notify() {
    await invoke("update_notify", {
      liveStartNotify: setting_model.live_start_notify,
      liveEndNotify: setting_model.live_end_notify,
      clipNotify: setting_model.clip_notify,
      postNotify: setting_model.post_notify,
    });
  }

  get_config();
</script>

<div class="flex-1 overflow-auto">
  <div class="h-screen">
    <div class="p-6 space-y-6">
      <!-- Header -->
      <div
        class="flex items-center justify-between dark:bg-[#1c1c1e] py-2 -mt-2 z-10"
      >
        <h1 class="text-2xl font-semibold text-gray-900 dark:text-white">
          Settings
        </h1>
      </div>

      <!-- Settings Sections -->
      <div class="space-y-6 pb-6">

        <!-- Storage Settings -->
        <div class="space-y-4">
          <h2
            class="text-lg font-medium text-gray-900 dark:text-white flex items-center space-x-2"
          >
            <HardDrive class="w-5 h-5 dark:icon-white" />
            <span>存储设置</span>
          </h2>
          <div
            class="bg-white dark:bg-[#3c3c3e] rounded-xl border border-gray-200 dark:border-gray-700 divide-y divide-gray-200 dark:divide-gray-700"
          >
            <!-- Cache Location -->
            <div class="p-4">
              <div class="flex items-center justify-between">
                <div>
                  <h3 class="text-sm font-medium text-gray-900 dark:text-white">
                    缓存路径
                  </h3>
                  <p class="text-sm text-gray-500 dark:text-gray-400">
                    {setting_model.cache}
                  </p>
                </div>
                <button
                  class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white hover:bg-gray-200 dark:hover:bg-gray-600 transition-colors"
                  on:click={async () => {
                    const new_folder = await browse_folder();
                    if (new_folder) {
                      setting_model.cache = new_folder;
                      await invoke("set_cache_path", {
                        cachePath: setting_model.cache,
                      });
                    }
                  }}
                >
                  变更
                </button>
              </div>
            </div>
            <div class="p-4">
              <div class="flex items-center justify-between">
                <div>
                  <h3 class="text-sm font-medium text-gray-900 dark:text-white">
                    切片保存路径
                  </h3>
                  <p class="text-sm text-gray-500 dark:text-gray-400">
                    {setting_model.output}
                  </p>
                </div>
                <button
                  class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white hover:bg-gray-200 dark:hover:bg-gray-600 transition-colors"
                  on:click={async () => {
                    const new_folder = await browse_folder();
                    if (new_folder) {
                      setting_model.output = new_folder;
                      await invoke("set_output_path", {
                        outputPath: setting_model.output,
                      });
                    }
                  }}
                >
                  变更
                </button>
              </div>
            </div>
          </div>
        </div>

        <!-- Notification Settings -->
        <div class="space-y-4">
          <h2
            class="text-lg font-medium text-gray-900 dark:text-white flex items-center space-x-2"
          >
            <Bell class="w-5 h-5 dark:icon-white" />
            <span>通知设置</span>
          </h2>
          <div
            class="bg-white dark:bg-[#3c3c3e] rounded-xl border border-gray-200 dark:border-gray-700 divide-y divide-gray-200 dark:divide-gray-700"
          >
            <!-- Stream Start -->
            <div class="p-4">
              <div class="flex items-center justify-between">
                <div>
                  <h3 class="text-sm font-medium text-gray-900 dark:text-white">
                    直播开始通知
                  </h3>
                  <p class="text-sm text-gray-500 dark:text-gray-400">
                    当直播间开始直播时，会收到通知
                  </p>
                </div>
                <label class="relative inline-block w-11 h-6">
                  <input
                    type="checkbox"
                    class="peer opacity-0 w-0 h-0"
                    bind:checked={setting_model.live_start_notify}
                    on:change={update_notify}
                  />
                  <span
                    class="switch-slider absolute cursor-pointer top-0 left-0 right-0 bottom-0 bg-gray-300 dark:bg-gray-600 rounded-full transition-all duration-300 before:absolute before:h-4 before:w-4 before:left-1 before:bottom-1 before:bg-white before:rounded-full before:transition-all before:duration-300 peer-checked:bg-blue-500 peer-checked:before:translate-x-5"
                  ></span>
                </label>
              </div>
            </div>
            <div class="p-4">
              <div class="flex items-center justify-between">
                <div>
                  <h3 class="text-sm font-medium text-gray-900 dark:text-white">
                    下播通知
                  </h3>
                  <p class="text-sm text-gray-500 dark:text-gray-400">
                    当直播间结束直播时，会收到通知
                  </p>
                </div>
                <label class="relative inline-block w-11 h-6">
                  <input
                    type="checkbox"
                    class="peer opacity-0 w-0 h-0"
                    bind:checked={setting_model.live_end_notify}
                    on:change={update_notify}
                  />
                  <span
                    class="switch-slider absolute cursor-pointer top-0 left-0 right-0 bottom-0 bg-gray-300 dark:bg-gray-600 rounded-full transition-all duration-300 before:absolute before:h-4 before:w-4 before:left-1 before:bottom-1 before:bg-white before:rounded-full before:transition-all before:duration-300 peer-checked:bg-blue-500 peer-checked:before:translate-x-5"
                  ></span>
                </label>
              </div>
            </div>
            <div class="p-4">
              <div class="flex items-center justify-between">
                <div>
                  <h3 class="text-sm font-medium text-gray-900 dark:text-white">
                    切片完成通知
                  </h3>
                  <p class="text-sm text-gray-500 dark:text-gray-400">
                    当切片完成时，会收到通知
                  </p>
                </div>
                <label class="relative inline-block w-11 h-6">
                  <input
                    type="checkbox"
                    class="peer opacity-0 w-0 h-0"
                    bind:checked={setting_model.clip_notify}
                    on:change={update_notify}
                  />
                  <span
                    class="switch-slider absolute cursor-pointer top-0 left-0 right-0 bottom-0 bg-gray-300 dark:bg-gray-600 rounded-full transition-all duration-300 before:absolute before:h-4 before:w-4 before:left-1 before:bottom-1 before:bg-white before:rounded-full before:transition-all before:duration-300 peer-checked:bg-blue-500 peer-checked:before:translate-x-5"
                  ></span>
                </label>
              </div>
            </div>
            <div class="p-4">
              <div class="flex items-center justify-between">
                <div>
                  <h3 class="text-sm font-medium text-gray-900 dark:text-white">
                    投稿完成通知
                  </h3>
                  <p class="text-sm text-gray-500 dark:text-gray-400">
                    当投稿完成时，会收到通知
                  </p>
                </div>
                <label class="relative inline-block w-11 h-6">
                  <input
                    type="checkbox"
                    class="peer opacity-0 w-0 h-0"
                    bind:checked={setting_model.post_notify}
                    on:change={update_notify}
                  />
                  <span
                    class="switch-slider absolute cursor-pointer top-0 left-0 right-0 bottom-0 bg-gray-300 dark:bg-gray-600 rounded-full transition-all duration-300 before:absolute before:h-4 before:w-4 before:left-1 before:bottom-1 before:bg-white before:rounded-full before:transition-all before:duration-300 peer-checked:bg-blue-500 peer-checked:before:translate-x-5"
                  ></span>
                </label>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</div>
