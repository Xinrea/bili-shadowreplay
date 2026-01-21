<script lang="ts">
  import { invoke } from "../lib/invoker";
  import { open } from "@tauri-apps/plugin-dialog";
  import { TAURI_ENV } from "../lib/invoker";

  import type { Config } from "../lib/interface";
  import {
    Bell,
    HardDrive,
    AlertTriangle,
    FileText,
    Captions,
    DiscAlbum,
    SquareBottomDashedScissors,
  } from "lucide-svelte";
  import { onMount } from "svelte";

  let setting_model: Config = {
    cache: "",
    output: "",
    primary_uid: 0,
    live_start_notify: true,
    live_end_notify: true,
    clip_notify: true,
    post_notify: true,
    auto_cleanup: true,
    auto_subtitle: false,
    subtitle_generator_type: "whisper",
    openai_api_endpoint: "",
    openai_api_key: "",
    powerlive_key: "",
    whisper_model: "",
    whisper_prompt: "",
    clip_name_format: "",
    auto_generate: {
      enabled: false,
      encode_danmu: false,
    },
    status_check_interval: 30, // 默认30秒
    whisper_language: "",
    webhook_url: "",
    danmu_ass_options: {
      font_size: 36,
      opacity: 0.8,
    },
    proxy_url: "",
  };

  let showModal = false;
  let endpoint = localStorage.getItem("endpoint") || "";
  let endpointValue = endpoint;
  let darkMode = localStorage.getItem("theme") === "dark";

  function updateTheme() {
    localStorage.setItem("theme", darkMode ? "dark" : "light");
    document.documentElement.classList.toggle("dark", darkMode);
  }

  function handleEndpointChange() {
    localStorage.setItem("endpoint", endpointValue);
    // reload page
    location.reload();
  }

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

  async function handleCacheChange() {
    showModal = true;
  }

  async function handleOutputChange() {
    const new_folder = await browse_folder();
    if (new_folder) {
      try {
        await invoke("set_output_path", {
          outputPath: new_folder,
        });
        setting_model.output = new_folder;
      } catch (e) {
        alert(e);
      }
    }
  }

  async function handleLogFolder() {
    await invoke("open_log_folder");
  }

  async function confirmChange() {
    showModal = false;
    const new_folder = await browse_folder();
    if (new_folder) {
      try {
        await invoke("set_cache_path", {
          cachePath: new_folder,
        });
        setting_model.cache = new_folder;
      } catch (e) {
        alert(e);
      }
    }
  }

  async function handleWhisperModelPathChange() {
    const selected = await open({
      multiple: false,
      filters: [
        {
          name: "Whisper Model",
          extensions: ["bin"],
        },
      ],
    });
    if (selected) {
      setting_model.whisper_model = Array.isArray(selected)
        ? selected[0]
        : selected;
      await invoke("update_whisper_model", {
        whisperModel: setting_model.whisper_model,
      });
    }
  }

  async function update_subtitle_setting() {
    await invoke("update_subtitle_setting", {
      autoSubtitle: setting_model.auto_subtitle,
    });
  }

  async function update_status_check_interval() {
    if (setting_model.status_check_interval < 10) {
      setting_model.status_check_interval = 10; // 最小值为10秒
    }
    await invoke("update_status_check_interval", {
      interval: setting_model.status_check_interval,
    });
  }

  async function update_webhook_url() {
    await invoke("update_webhook_url", {
      webhookUrl: setting_model.webhook_url,
    });
  }

  async function update_proxy_url() {
    await invoke("update_proxy_url", {
      proxyUrl: setting_model.proxy_url,
    });
  }

  async function update_danmu_ass_options() {
    await invoke("update_danmu_ass_options", {
      fontSize: setting_model.danmu_ass_options.font_size,
      opacity: setting_model.danmu_ass_options.opacity,
    });
  }

  onMount(async () => {
    await get_config();
  });
</script>

<div class="flex-1 overflow-auto custom-scrollbar-light bg-gray-50 dark:bg-black">
  <div class="h-screen">
    <div class="p-6 space-y-6">
      <!-- Header -->
      <div
        class="flex items-center justify-between dark:bg-black py-2 -mt-2 z-10"
      >
        <h1 class="text-2xl font-semibold text-gray-900 dark:text-white">
          Settings
        </h1>
      </div>

      <!-- Settings Sections -->
      <div class="space-y-6 pb-6">
        <div class="space-y-4">
          <h2
            class="text-lg font-medium text-gray-900 dark:text-white flex items-center space-x-2"
          >
            <span>外观设置</span>
          </h2>
          <div
            class="bg-white dark:bg-[#3c3c3e] rounded-xl border border-gray-200 dark:border-gray-700 divide-y divide-gray-200 dark:divide-gray-700"
          >
            <div class="p-4">
              <div class="flex items-center justify-between">
                <div>
                  <h3 class="text-sm font-medium text-gray-900 dark:text-white">
                    夜晚模式
                  </h3>
                  <p class="text-sm text-gray-500 dark:text-gray-400">
                    反转文字与背景，黑色背景，白色文字
                  </p>
                </div>
                <label class="relative inline-block w-11 h-6">
                  <input
                    type="checkbox"
                    class="peer opacity-0 w-0 h-0"
                    bind:checked={darkMode}
                    on:change={updateTheme}
                  />
                  <span
                    class="switch-slider absolute cursor-pointer top-0 left-0 right-0 bottom-0 bg-gray-300 dark:bg-gray-600 rounded-full transition-all duration-300 before:absolute before:h-4 before:w-4 before:left-1 before:bottom-1 before:bg-white before:rounded-full before:transition-all before:duration-300 peer-checked:bg-blue-500 peer-checked:before:translate-x-5"
                  ></span>
                </label>
              </div>
            </div>
          </div>
        </div>
        <div class="space-y-4">
          <h2
            class="text-lg font-medium text-gray-900 dark:text-white flex items-center space-x-2"
          >
            <FileText class="w-5 h-5 dark:icon-white" />
            <span>基础设置</span>
          </h2>
          <div
            class="bg-white dark:bg-[#3c3c3e] rounded-xl border border-gray-200 dark:border-gray-700 divide-y divide-gray-200 dark:divide-gray-700"
          >
            <div class="p-4">
              <div class="flex items-center justify-between">
                <div>
                  <h3 class="text-sm font-medium text-gray-900 dark:text-white">
                    直播间状态检查间隔
                  </h3>
                  <p class="text-sm text-gray-500 dark:text-gray-400">
                    设置直播间状态检查的时间间隔，单位为秒，过于频繁可能会触发风控
                  </p>
                </div>
                <div class="flex items-center space-x-2">
                  <input
                    type="number"
                    class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white w-24"
                    bind:value={setting_model.status_check_interval}
                    on:blur={update_status_check_interval}
                  />
                </div>
              </div>
            </div>
            <div class="p-4">
              <div class="flex items-center justify-between">
                <div>
                  <h3 class="text-sm font-medium text-gray-900 dark:text-white">
                    Webhook URL
                  </h3>
                  <p class="text-sm text-gray-500 dark:text-gray-400">
                    设置 Webhook URL，用于接收事件通知，见<a
                      href="https://bsr.xinrea.cn/usage/features/webhook.html"
                      class="text-blue-500 hover:text-blue-700"
                      target="_blank">Webhook 文档</a
                    >
                  </p>
                </div>
                <div class="flex items-center space-x-2">
                  <input
                    type="text"
                    class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white w-96"
                    bind:value={setting_model.webhook_url}
                    on:change={update_webhook_url}
                    placeholder="https://example.com/webhook"
                  />
                </div>
              </div>
            </div>
            <div class="p-4">
              <div class="flex items-center justify-between">
                <div>
                  <h3 class="text-sm font-medium text-gray-900 dark:text-white">
                    HTTP Proxy
                  </h3>
                  <p class="text-sm text-gray-500 dark:text-gray-400">
                    Configure HTTP/HTTPS proxy for platforms like TikTok
                  </p>
                </div>
                <div class="flex items-center space-x-2">
                  <input
                    type="text"
                    class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white w-96"
                    bind:value={setting_model.proxy_url}
                    on:change={update_proxy_url}
                    placeholder="http://127.0.0.1:7890"
                  />
                </div>
              </div>
            </div>
          </div>
        </div>
        <!-- API Server Settings -->
        {#if !TAURI_ENV}
          <div class="space-y-4">
            <h2
              class="text-lg font-medium text-gray-900 dark:text-white flex items-center space-x-2"
            >
              <FileText class="w-5 h-5 dark:icon-white" />
              <span>API 服务器配置</span>
            </h2>
            <div
              class="bg-white dark:bg-[#3c3c3e] rounded-xl border border-gray-200 dark:border-gray-700 divide-y divide-gray-200 dark:divide-gray-700"
            >
              <div class="p-4">
                <div class="flex items-center justify-between">
                  <div>
                    <h3
                      class="text-sm font-medium text-gray-900 dark:text-white"
                    >
                      API 服务器地址
                    </h3>
                    <p class="text-sm text-gray-500 dark:text-gray-400">
                      设置 API 服务器的地址
                    </p>
                  </div>
                  <div class="flex items-center space-x-2">
                    <input
                      type="text"
                      class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white w-96"
                      bind:value={endpointValue}
                      on:blur={handleEndpointChange}
                      placeholder="http://localhost:3000"
                    />
                  </div>
                </div>
              </div>
            </div>
          </div>
        {/if}

        {#if TAURI_ENV || endpoint != ""}
          <!-- Storage Settings -->
          {#if TAURI_ENV}
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
                      <h3
                        class="text-sm font-medium text-gray-900 dark:text-white"
                      >
                        缓存路径
                      </h3>
                      <p class="text-sm text-gray-500 dark:text-gray-400">
                        {setting_model.cache}
                      </p>
                    </div>
                    <button
                      class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white hover:bg-gray-200 dark:hover:bg-gray-600 transition-colors"
                      on:click={handleCacheChange}
                    >
                      变更
                    </button>
                  </div>
                </div>
                <div class="p-4">
                  <div class="flex items-center justify-between">
                    <div>
                      <h3
                        class="text-sm font-medium text-gray-900 dark:text-white"
                      >
                        切片保存路径
                      </h3>
                      <p class="text-sm text-gray-500 dark:text-gray-400">
                        {setting_model.output}
                      </p>
                    </div>
                    <button
                      class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white hover:bg-gray-200 dark:hover:bg-gray-600 transition-colors"
                      on:click={handleOutputChange}
                    >
                      变更
                    </button>
                  </div>
                </div>
                <div class="p-4">
                  <div class="flex items-center justify-between">
                    <div>
                      <h3
                        class="text-sm font-medium text-gray-900 dark:text-white"
                      >
                        日志文件夹
                      </h3>
                      <p class="text-sm text-gray-500 dark:text-gray-400">
                        查看应用程序日志文件
                      </p>
                    </div>
                    <button
                      class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white hover:bg-gray-200 dark:hover:bg-gray-600 transition-colors"
                      on:click={handleLogFolder}
                    >
                      打开
                    </button>
                  </div>
                </div>
              </div>
            </div>
          {/if}

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
                    <h3
                      class="text-sm font-medium text-gray-900 dark:text-white"
                    >
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
                    <h3
                      class="text-sm font-medium text-gray-900 dark:text-white"
                    >
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
                    <h3
                      class="text-sm font-medium text-gray-900 dark:text-white"
                    >
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
                    <h3
                      class="text-sm font-medium text-gray-900 dark:text-white"
                    >
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

          <!-- Subtitle Generation Settings -->
          <div class="space-y-4">
            <h2
              class="text-lg font-medium text-gray-900 dark:text-white flex items-center space-x-2"
            >
              <Captions class="w-5 h-5 dark:icon-white" />
              <span>字幕生成</span>
            </h2>
            <div
              class="bg-white dark:bg-[#3c3c3e] rounded-xl border border-gray-200 dark:border-gray-700 divide-y divide-gray-200 dark:divide-gray-700"
            >
              <!-- Auto Subtitle Generation -->
              <div class="p-4">
                <div class="flex items-center justify-between">
                  <div>
                    <h3
                      class="text-sm font-medium text-gray-900 dark:text-white"
                    >
                      自动生成字幕
                    </h3>
                    <p class="text-sm text-gray-500 dark:text-gray-400">
                      启用后，切片完成后会自动生成字幕
                    </p>
                  </div>
                  <label class="relative inline-block w-11 h-6">
                    <input
                      type="checkbox"
                      class="peer opacity-0 w-0 h-0"
                      bind:checked={setting_model.auto_subtitle}
                      on:change={update_subtitle_setting}
                    />
                    <span
                      class="switch-slider absolute cursor-pointer top-0 left-0 right-0 bottom-0 bg-gray-300 dark:bg-gray-600 rounded-full transition-all duration-300 before:absolute before:h-4 before:w-4 before:left-1 before:bottom-1 before:bg-white before:rounded-full before:transition-all before:duration-300 peer-checked:bg-blue-500 peer-checked:before:translate-x-5"
                    ></span>
                  </label>
                </div>
              </div>
              <!-- Subtitle Generator Type -->
              <div class="p-4">
                <div class="flex items-center justify-between">
                  <div>
                    <h3
                      class="text-sm font-medium text-gray-900 dark:text-white"
                    >
                      字幕生成器类型
                    </h3>
                    <p class="text-sm text-gray-500 dark:text-gray-400">
                      选择字幕生成的方式：本地模型，OpenAI 服务或 <a
                        href="https://www.powerlive.io/"
                        class="text-blue-500 hover:underline"
                        target="_blank"
                        rel="noopener noreferrer">PowerLive</a
                      > 服务（按量付费）
                    </p>
                  </div>
                  <div class="flex items-center space-x-2">
                    <select
                      class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white"
                      bind:value={setting_model.subtitle_generator_type}
                      on:change={async () => {
                        try {
                          await invoke("update_subtitle_generator_type", {
                            subtitleGeneratorType:
                              setting_model.subtitle_generator_type,
                          });
                        } catch (error) {
                          console.error(error);
                        }
                      }}
                    >
                      <option value="whisper">本地 Whisper</option>
                      <option value="whisper_online">在线 Whisper API</option>
                      <option value="powerlive">PowerLive</option>
                    </select>
                  </div>
                </div>
              </div>
              <!-- Whisper Model Path -->
              {#if setting_model.subtitle_generator_type === "powerlive"}
                <div class="p-4">
                  <div class="flex items-center justify-between">
                    <div>
                      <h3
                        class="text-sm font-medium text-gray-900 dark:text-white"
                      >
                        PowerLive API 密钥
                      </h3>
                      <p class="text-sm text-gray-500 dark:text-gray-400">
                        设置 PowerLive API 的访问密钥
                      </p>
                    </div>
                    <div class="flex items-center space-x-2">
                      <input
                        type="password"
                        class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white w-96"
                        bind:value={setting_model.powerlive_key}
                        on:change={async () => {
                          await invoke("update_powerlive_key", {
                            powerliveKey: setting_model.powerlive_key,
                          });
                        }}
                        placeholder="pk_..."
                      />
                    </div>
                  </div>
                </div>
              {:else}
                {#if TAURI_ENV && setting_model.subtitle_generator_type === "whisper"}
                  <div class="p-4">
                    <div class="flex items-center justify-between">
                      <div>
                        <h3
                          class="text-sm font-medium text-gray-900 dark:text-white"
                        >
                          Whisper 模型路径
                        </h3>
                        <p class="text-sm text-gray-500 dark:text-gray-400">
                          {setting_model.whisper_model || "未设置"}
                          <span class="block mt-1 text-xs"
                            >可前往 <a
                              href="https://huggingface.co/ggerganov/whisper.cpp/tree/main"
                              class="text-blue-500 hover:underline"
                              target="_blank"
                              rel="noopener noreferrer">ggerganov/whisper.cpp</a
                            > 下载模型文件</span
                          >
                        </p>
                      </div>
                      <button
                        class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white hover:bg-gray-200 dark:hover:bg-gray-600 transition-colors"
                        on:click={handleWhisperModelPathChange}
                      >
                        变更
                      </button>
                    </div>
                  </div>
                {/if}
                <!-- OpenAI API Settings -->
                {#if setting_model.subtitle_generator_type === "whisper_online"}
                  <div class="p-4">
                    <div class="flex items-center justify-between">
                      <div>
                        <h3
                          class="text-sm font-medium text-gray-900 dark:text-white"
                        >
                          OpenAI API 端点
                        </h3>
                        <p class="text-sm text-gray-500 dark:text-gray-400">
                          设置 OpenAI API 的端点地址，默认为官方地址
                        </p>
                      </div>
                      <div class="flex items-center space-x-2">
                        <input
                          type="text"
                          class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white w-96"
                          bind:value={setting_model.openai_api_endpoint}
                          on:change={async () => {
                            await invoke("update_openai_api_endpoint", {
                              openaiApiEndpoint:
                                setting_model.openai_api_endpoint,
                            });
                          }}
                          placeholder="https://api.openai.com/v1"
                        />
                      </div>
                    </div>
                  </div>
                  <div class="p-4">
                    <div class="flex items-center justify-between">
                      <div>
                        <h3
                          class="text-sm font-medium text-gray-900 dark:text-white"
                        >
                          OpenAI API 密钥
                        </h3>
                        <p class="text-sm text-gray-500 dark:text-gray-400">
                          设置 OpenAI API 的访问密钥
                        </p>
                      </div>
                      <div class="flex items-center space-x-2">
                        <input
                          type="password"
                          class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white w-96"
                          bind:value={setting_model.openai_api_key}
                          on:change={async () => {
                            await invoke("update_openai_api_key", {
                              openaiApiKey: setting_model.openai_api_key,
                            });
                          }}
                          placeholder="sk-..."
                        />
                      </div>
                    </div>
                  </div>
                {/if}
                <!-- Whisper Language -->
                <div class="p-4">
                  <div class="flex items-center justify-between">
                    <div>
                      <h3
                        class="text-sm font-medium text-gray-900 dark:text-white"
                      >
                        Whisper 语言
                      </h3>
                      <p class="text-sm text-gray-500 dark:text-gray-400">
                        （测试）生成字幕时使用的语言，默认自动识别
                      </p>
                    </div>
                    <div class="flex items-center space-x-2">
                      <input
                        type="text"
                        class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white w-96"
                        bind:value={setting_model.whisper_language}
                        on:change={async () => {
                          await invoke("update_whisper_language", {
                            whisperLanguage: setting_model.whisper_language,
                          });
                        }}
                      />
                    </div>
                  </div>
                </div>
                <div class="p-4">
                  <div class="flex items-center justify-between">
                    <div>
                      <h3
                        class="text-sm font-medium text-gray-900 dark:text-white"
                      >
                        Whisper 提示词
                      </h3>
                      <p class="text-sm text-gray-500 dark:text-gray-400">
                        生成字幕时使用的提示词，尽量简洁明了，提示音频内容偏向的领域以及字幕的风格
                      </p>
                    </div>
                    <div class="flex items-center space-x-2">
                      <input
                        type="text"
                        class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white w-96"
                        bind:value={setting_model.whisper_prompt}
                        on:change={async () => {
                          await invoke("update_whisper_prompt", {
                            whisperPrompt: setting_model.whisper_prompt,
                          });
                        }}
                      />
                    </div>
                  </div>
                </div>
              {/if}
            </div>
          </div>

          <!-- Clip Name Format Settings -->
          <div class="space-y-4">
            <h2
              class="text-lg font-medium text-gray-900 dark:text-white flex items-center space-x-2"
            >
              <DiscAlbum class="w-5 h-5 dark:icon-white" />
              <span>切片文件名格式</span>
            </h2>
            <div
              class="bg-white dark:bg-[#3c3c3e] rounded-xl border border-gray-200 dark:border-gray-700 divide-y divide-gray-200 dark:divide-gray-700"
            >
              <div class="p-4">
                <div class="flex items-center justify-between">
                  <div>
                    <h3
                      class="text-sm font-medium text-gray-900 dark:text-white"
                    >
                      文件名格式
                    </h3>
                    <p class="text-sm text-gray-500 dark:text-gray-400">
                      可用标签：{"{title}"}
                      {"{platform}"}
                      {"{room_id}"}
                      {"{live_id}"}
                      {"{x}"}
                      {"{y}"}
                      {"{created_at}"}
                      {"{length}"}
                      {"{note}"}
                    </p>
                  </div>
                  <div class="flex items-center space-x-2">
                    <input
                      type="text"
                      class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white w-96"
                      bind:value={setting_model.clip_name_format}
                      on:change={async () => {
                        await invoke("update_clip_name_format", {
                          clipNameFormat: setting_model.clip_name_format,
                        });
                      }}
                    />
                  </div>
                </div>
              </div>
            </div>
          </div>

          <!-- Danmu Style Settings -->
          <div class="space-y-4">
            <h2
              class="text-lg font-medium text-gray-900 dark:text-white flex items-center space-x-2"
            >
              <Captions class="w-5 h-5 dark:icon-white" />
              <span>弹幕压制样式</span>
            </h2>
            <div
              class="bg-white dark:bg-[#3c3c3e] rounded-xl border border-gray-200 dark:border-gray-700 divide-y divide-gray-200 dark:divide-gray-700"
            >
              <!-- Font Size -->
              <div class="p-4">
                <div class="flex items-center justify-between">
                  <div>
                    <h3
                      class="text-sm font-medium text-gray-900 dark:text-white"
                    >
                      字体大小
                    </h3>
                    <p class="text-sm text-gray-500 dark:text-gray-400">
                      设置弹幕字体大小
                    </p>
                  </div>
                  <div class="flex items-center space-x-2">
                    <input
                      type="number"
                      class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white w-24"
                      bind:value={setting_model.danmu_ass_options.font_size}
                      on:blur={update_danmu_ass_options}
                      min="12"
                      max="72"
                      step="1"
                    />
                  </div>
                </div>
              </div>
              <!-- Opacity -->
              <div class="p-4">
                <div class="flex items-center justify-between">
                  <div>
                    <h3
                      class="text-sm font-medium text-gray-900 dark:text-white"
                    >
                      不透明度
                    </h3>
                    <p class="text-sm text-gray-500 dark:text-gray-400">
                      设置弹幕不透明度，范围
                      0.0-1.0，0.0为完全透明，1.0为完全不透明
                    </p>
                  </div>
                  <div class="flex items-center space-x-2">
                    <input
                      type="number"
                      class="px-3 py-2 bg-gray-100 dark:bg-gray-700 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-900 dark:text-white w-24"
                      bind:value={setting_model.danmu_ass_options.opacity}
                      on:blur={update_danmu_ass_options}
                      min="0.0"
                      max="1.0"
                      step="0.1"
                    />
                  </div>
                </div>
              </div>
            </div>
          </div>

          <!-- Auto Clip Settings -->
          <div class="space-y-4">
            <h2
              class="text-lg font-medium text-gray-900 dark:text-white flex items-center space-x-2"
            >
              <SquareBottomDashedScissors class="w-5 h-5 dark:icon-white" />
              <span>自动切片</span>
            </h2>
            <div
              class="bg-white dark:bg-[#3c3c3e] rounded-xl border border-gray-200 dark:border-gray-700 divide-y divide-gray-200 dark:divide-gray-700"
            >
              <!-- Auto Clip Generation -->
              <div class="p-4">
                <div class="flex items-center justify-between">
                  <div>
                    <h3
                      class="text-sm font-medium text-gray-900 dark:text-white"
                    >
                      整场录播生成
                    </h3>
                    <p class="text-sm text-gray-500 dark:text-gray-400">
                      启用后，直播结束后会自动整场录播进入切片列表
                    </p>
                  </div>
                  <label class="relative inline-block w-11 h-6">
                    <input
                      type="checkbox"
                      class="peer opacity-0 w-0 h-0"
                      bind:checked={setting_model.auto_generate.enabled}
                      on:change={async () => {
                        await invoke("update_auto_generate", {
                          enabled: setting_model.auto_generate.enabled,
                          encodeDanmu: setting_model.auto_generate.encode_danmu,
                        });
                      }}
                    />
                    <span
                      class="switch-slider absolute cursor-pointer top-0 left-0 right-0 bottom-0 bg-gray-300 dark:bg-gray-600 rounded-full transition-all duration-300 before:absolute before:h-4 before:w-4 before:left-1 before:bottom-1 before:bg-white before:rounded-full before:transition-all before:duration-300 peer-checked:bg-blue-500 peer-checked:before:translate-x-5"
                    ></span>
                  </label>
                </div>
              </div>
              <!-- Auto Clip Encode Danmu -->
              <div class="p-4">
                <div class="flex items-center justify-between">
                  <div>
                    <h3
                      class="text-sm font-medium text-gray-900 dark:text-white"
                    >
                      自动切片压制弹幕
                    </h3>
                    <p class="text-sm text-gray-500 dark:text-gray-400">
                      启用后，自动切片时会同时压制弹幕，会显著增加生成时间
                    </p>
                  </div>
                  <label class="relative inline-block w-11 h-6">
                    <input
                      type="checkbox"
                      class="peer opacity-0 w-0 h-0"
                      disabled
                      bind:checked={setting_model.auto_generate.encode_danmu}
                      on:change={async () => {
                        await invoke("update_auto_generate", {
                          enabled: setting_model.auto_generate.enabled,
                          encodeDanmu: setting_model.auto_generate.encode_danmu,
                        });
                      }}
                    />
                    <span
                      class="switch-slider absolute cursor-pointer top-0 left-0 right-0 bottom-0 bg-gray-300 dark:bg-gray-600 rounded-full transition-all duration-300 before:absolute before:h-4 before:w-4 before:left-1 before:bottom-1 before:bg-white before:rounded-full before:transition-all before:duration-300 peer-checked:bg-blue-500 peer-checked:before:translate-x-5"
                    ></span>
                  </label>
                </div>
              </div>
            </div>
          </div>
        {/if}
      </div>
    </div>
  </div>
</div>

<!-- Modal -->
{#if showModal}
  <div
    class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50"
  >
    <div class="bg-white dark:bg-[#2c2c2e] rounded-xl p-6 max-w-md w-full mx-4">
      <div class="flex items-start space-x-3 mb-4">
        <AlertTriangle class="w-6 h-6 text-yellow-500 flex-shrink-0" />
        <div>
          <h3 class="text-lg font-medium text-gray-900 dark:text-white">
            确认变更
          </h3>
          <p class="text-gray-600 dark:text-gray-400 mt-2">
            根据文件大小，可能需要耗时较长时间，迁移期间直播间会暂时移除，迁移完成后直播间会自动恢复。
          </p>
          <p class="text-gray-600 dark:text-gray-400 mt-2 font-bold">
            迁移期间请不要关闭程序，且不要在迁移期间再次更改目录！
          </p>
          <p class="text-gray-600 dark:text-gray-400 mt-2">
            确认要进行变更吗？
          </p>
        </div>
      </div>
      <div class="flex justify-end space-x-4">
        <button
          class="px-4 py-2 text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg transition-colors"
          on:click={() => (showModal = false)}
        >
          取消
        </button>
        <button
          class="px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
          on:click={confirmChange}
        >
          确认
        </button>
      </div>
    </div>
  </div>
{/if}
