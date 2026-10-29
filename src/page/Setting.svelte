<script lang="ts">
  import { invoke, TAURI_ENV, normalizeEndpoint } from "../lib/invoker";
  import { open } from "@tauri-apps/plugin-dialog";
  import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
  import { clickOutside } from "../lib/actions/clickOutside";

  import type { Config, McpServerStatus } from "../lib/interface";
  import {
    Bell,
    HardDrive,
    AlertTriangle,
    FileText,
    Captions,
    DiscAlbum,
    SquareBottomDashedScissors,
    Bot,
    RefreshCw,
    Plug,
    Copy,
    FolderOpen,
    UserRound,
    Sun,
    Moon,
    Monitor,
  } from "lucide-svelte";
  import { onMount } from "svelte";
  import PowerLiveAccount from "../lib/components/PowerLiveAccount.svelte";
  import PowerLiveHotwords from "../lib/components/PowerLiveHotwords.svelte";
  import PowerLiveSentenceLength from "../lib/components/PowerLiveSentenceLength.svelte";
  import PageTitlebar from "../lib/components/PageTitlebar.svelte";
  import RenderSettingsFields from "../lib/components/RenderSettingsFields.svelte";
  import { defaultRenderSettings, normalizeRenderSettings } from "../lib/project-timeline";
  import { registerCommands } from "../lib/stores/commands";
  import {
    themePreference,
    setThemePreference,
    type ThemePreference,
  } from "../lib/stores/theme";

  let { active = true }: { active?: boolean } = $props();

  const themeOptions: { value: ThemePreference; label: string }[] = [
    { value: "system", label: "跟随系统" },
    { value: "light", label: "亮色" },
    { value: "dark", label: "暗色" },
  ];

  // Options the command palette can jump to, by section. Each id matches a
  // `data-setting` anchor below; titles follow the labels there.
  const paletteTargets = [
    {
      title: "PowerLive",
      icon: UserRound,
      options: [{ id: "powerlive", title: "PowerLive 账号", keywords: ["account", "credit"] }],
    },
    {
      title: "基础设置",
      icon: FileText,
      options: [
        { id: "theme", title: "外观主题", keywords: ["theme", "appearance", "dark", "light"] },
        { id: "launch-at-login", title: "开机自动启动", keywords: ["autostart", "startup", "launch"] },
        { id: "status-check-interval", title: "直播间状态检查间隔", keywords: ["interval", "status", "polling"] },
        { id: "webhook", title: "Webhook URL", keywords: ["webhook"] },
      ],
    },
    {
      title: "API 服务器配置",
      icon: FileText,
      options: [{ id: "api-endpoint", title: "API 服务器地址", keywords: ["api", "server", "endpoint"] }],
    },
    {
      title: "MCP Server",
      icon: Plug,
      options: [
        { id: "mcp-enabled", title: "启用 MCP Server", keywords: ["mcp", "agent"] },
        { id: "mcp-port", title: "MCP 端口", keywords: ["mcp", "port"] },
        { id: "mcp-url", title: "连接地址（Streamable HTTP）", keywords: ["mcp", "url", "config"] },
      ],
    },
    {
      title: "存储设置",
      icon: HardDrive,
      options: [
        { id: "cache-path", title: "缓存路径", keywords: ["cache", "storage", "path"] },
        { id: "output-path", title: "素材保存路径", keywords: ["output", "material", "path"] },
        { id: "log-folder", title: "日志文件夹", keywords: ["log", "logs"] },
      ],
    },
    {
      title: "通知设置",
      icon: Bell,
      options: [
        { id: "notify-live-start", title: "直播开始通知", keywords: ["notification", "live start"] },
        { id: "notify-live-end", title: "下播通知", keywords: ["notification", "live end"] },
        { id: "notify-clip", title: "素材生成通知", keywords: ["notification", "material"] },
        { id: "notify-post", title: "投稿完成通知", keywords: ["notification", "upload", "post"] },
      ],
    },
    {
      title: "AI 模型",
      icon: Bot,
      options: [
        { id: "llm-provider", title: "模型提供商", keywords: ["llm", "provider", "openai", "ollama"] },
        { id: "llm-endpoint", title: "API Endpoint", keywords: ["llm", "endpoint"] },
        { id: "llm-api-key", title: "API Key", keywords: ["llm", "key", "token"] },
        { id: "llm-model", title: "模型名称", keywords: ["llm", "model"] },
      ],
    },
    {
      title: "字幕生成",
      icon: Captions,
      options: [
        { id: "auto-subtitle", title: "自动生成字幕", keywords: ["subtitle", "auto"] },
        { id: "subtitle-generator", title: "字幕生成器类型", keywords: ["subtitle", "generator", "whisper"] },
        { id: "powerlive-hotwords", title: "PowerLive 全局词库", keywords: ["powerlive", "hotword", "subtitle", "热词"] },
        { id: "powerlive-sentence-length", title: "每条字幕最多字数", keywords: ["powerlive", "sentence", "length"] },
        { id: "whisper-model", title: "Whisper 模型路径", keywords: ["whisper", "model"] },
        { id: "whisper-endpoint", title: "OpenAI API 端点", keywords: ["whisper", "endpoint"] },
        { id: "whisper-api-key", title: "OpenAI API 密钥", keywords: ["whisper", "key"] },
        { id: "whisper-prompt", title: "Whisper 提示词", keywords: ["whisper", "prompt"] },
        { id: "whisper-language", title: "Whisper 语言", keywords: ["whisper", "language"] },
      ],
    },
    {
      title: "素材文件名格式",
      icon: DiscAlbum,
      options: [{ id: "clip-name-format", title: "文件名格式", keywords: ["material", "filename", "format"] }],
    },
    {
      title: "弹幕样式",
      icon: Captions,
      options: [
        { id: "danmu-font-size", title: "字体大小", keywords: ["danmaku", "font", "size", "preview"] },
        { id: "danmu-opacity", title: "不透明度", keywords: ["danmaku", "opacity", "preview"] },
      ],
    },
    {
      title: "自动素材生成",
      icon: SquareBottomDashedScissors,
      options: [
        { id: "auto-generate", title: "生成整场录播素材", keywords: ["auto", "material", "whole"] },
        { id: "auto-generate-danmu", title: "整场生成时压制弹幕", keywords: ["auto", "material", "danmaku"] },
        { id: "auto-generate-encoding", title: "自动生成编码设置", keywords: ["auto", "codec", "hevc", "h264", "bitrate", "画质", "码率"] },
      ],
    },
  ];

  const themeIcons = { system: Monitor, light: Sun, dark: Moon };

  let scroller: HTMLElement | null = $state(null);

  /** Scrolls to the option anchored as `data-setting={id}` and highlights it. */
  export function reveal(id: string) {
    const target = scroller?.querySelector<HTMLElement>(`[data-setting="${CSS.escape(id)}"]`);
    if (!scroller || !target) return;
    const top =
      target.getBoundingClientRect().top -
      scroller.getBoundingClientRect().top +
      scroller.scrollTop;
    // Centre the option, but keep the heading above a tall one in view.
    const offset = Math.max(72, (scroller.clientHeight - target.offsetHeight) / 2);
    const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;
    scroller.scrollTo({ top: top - offset, behavior: reduceMotion ? "auto" : "smooth" });
    // Held until the page has slid in, then fades back to the option's own background.
    const accentSoft = getComputedStyle(target).getPropertyValue("--c-accent-soft");
    const highlight = { backgroundColor: `rgb(${accentSoft})` };
    target.animate([highlight, { ...highlight, offset: 0.4 }], {
      duration: 2000,
      easing: "ease-out",
    });
  }

  onMount(() =>
    registerCommands([
      ...paletteTargets.flatMap((section) =>
        section.options.map((option) => ({
          id: `setting:${option.id}`,
          group: "setting" as const,
          title: option.title,
          detail: section.title,
          keywords: option.keywords,
          icon: section.icon,
          page: "设置",
          // Options the current configuration hides have no anchor.
          available: () => Boolean(scroller?.querySelector(`[data-setting="${option.id}"]`)),
          run: () => reveal(option.id),
        })),
      ),
      {
        id: "open-log-folder",
        group: "action",
        title: "打开日志文件夹",
        detail: "设置",
        icon: FolderOpen,
        keywords: ["log", "logs", "folder"],
        available: () => TAURI_ENV,
        run: handleLogFolder,
      },
      ...themeOptions.map((option) => ({
        id: `theme:${option.value}`,
        group: "action" as const,
        title: `外观主题：${option.label}`,
        detail: "设置",
        icon: themeIcons[option.value],
        keywords: ["theme", "appearance", option.value],
        available: () => $themePreference !== option.value,
        run: () => setThemePreference(option.value),
      })),
    ]),
  );

  let setting_model: Config = $state({
    cache: "",
    output: "",
    primary_uid: 0,
    live_start_notify: true,
    live_end_notify: true,
    clip_notify: true,
    post_notify: true,
    auto_cleanup: true,
    auto_subtitle: false,
    mcp_enabled: false,
    mcp_port: 18763,
    subtitle_generator_type: "whisper",
    openai_api_endpoint: "",
    openai_api_key: "",
    llm: {
      provider: "openai",
      endpoint: "https://api.openai.com/v1",
      api_key: "",
      model: "",
    },
    whisper_model: "",
    whisper_prompt: "",
    powerlive_hotwords: [],
    powerlive_room_hotwords: {},
    powerlive_sentence_max_length: 10,
    clip_name_format: "",
    auto_generate: {
      enabled: false,
      encode_danmu: false,
      render_settings: defaultRenderSettings(),
    },
    status_check_interval: 30, // 默认30秒
    whisper_language: "",

    webhook_url: "",
    danmu_ass_options: {
      font_size: 36,
      opacity: 0.8,
    },
  });

  let showModal = $state(false);
  let show_clip_name_help = $state(false);
  let endpoint = localStorage.getItem("endpoint") || "";
  let endpointValue = $state(endpoint);
  let llmModels: string[] = $state([]);
  let llmModelsLoading = $state(false);
  let llmSaving = $state(false);
  let llmSaveMessage = $state("");
  let llmError = $state("");
  let showModelList = $state(false);
  let llmSaveChain: Promise<void> = Promise.resolve();
  let autoGenerateLoading = $state(true);
  let autoGenerateSaving = $state(false);
  let autoGenerateError = $state("");
  let launchAtLogin = $state(false);
  let mcpStatus: McpServerStatus | null = $state(null);
  let mcpLoading = $state(true);
  let mcpError = $state("");
  let mcpPortDraft: number | undefined = $state(18763);
  let mcpPortError = $state("");
  let mcpSaveMessage = $state("");
  let mcpCopyMessage = $state("");
  let mcpPortChanged = $derived(
    mcpStatus !== null && mcpPortDraft !== mcpStatus.port,
  );
  let mcpUrl = $derived(
    mcpStatus?.url || `http://127.0.0.1:${setting_model.mcp_port}/mcp`,
  );
  let mcpClientConfig = $derived(
    JSON.stringify(
      { mcpServers: { bsr: { type: "http", url: mcpUrl } } },
      null,
      2,
    ),
  );

  function applyMcpServerStatus(
    status: McpServerStatus,
    resetPortDraft = true,
  ) {
    mcpStatus = status;
    setting_model.mcp_enabled = status.enabled;
    setting_model.mcp_port = status.port;
    if (resetPortDraft) mcpPortDraft = status.port;
    mcpError = status.error || "";
  }

  async function loadMcpServerStatus() {
    mcpLoading = true;
    mcpError = "";
    mcpPortError = "";
    mcpSaveMessage = "";
    try {
      applyMcpServerStatus(
        await invoke<McpServerStatus>("get_mcp_server_status"),
      );
    } catch (error) {
      mcpError = String(error);
    } finally {
      mcpLoading = false;
    }
  }

  async function updateMcpServer() {
    if (!mcpStatus) return;
    // 开关沿用已经保存的端口，输入框中的未保存内容不会改变监听地址。
    await saveMcpServer(setting_model.mcp_enabled);
  }

  async function updateMcpPort() {
    if (!mcpStatus) return;
    mcpSaveMessage = "";
    if (
      mcpPortDraft === undefined ||
      !Number.isInteger(mcpPortDraft) ||
      mcpPortDraft < 1024 ||
      mcpPortDraft > 65535
    ) {
      mcpPortError = "端口必须为 1024–65535 之间的整数";
      return;
    }
    mcpPortError = "";
    await saveMcpServer(mcpStatus.enabled, mcpPortDraft);
  }

  async function saveMcpServer(enabled: boolean, port?: number) {
    if (!mcpStatus || mcpLoading) return;
    const previousStatus = mcpStatus;
    const resetPortDraft = port !== undefined;
    mcpLoading = true;
    mcpError = "";
    mcpSaveMessage = "";
    mcpCopyMessage = "";
    try {
      const status = await invoke<McpServerStatus>("update_mcp_server", {
        enabled,
        ...(port === undefined ? {} : { port }),
      });
      applyMcpServerStatus(status, resetPortDraft);
      if (resetPortDraft) {
        mcpSaveMessage = status.port !== previousStatus.port
          ? `端口已保存${status.running ? "，服务已切换到新端口" : ""}。请将外部 Agent 的 MCP 地址更新为 ${status.url}`
          : "端口已保存";
      }
    } catch (error) {
      applyMcpServerStatus(previousStatus, resetPortDraft);
      // 更新失败后读取实际状态，避免将保存失败或服务启动失败显示为成功。
      try {
        applyMcpServerStatus(
          await invoke<McpServerStatus>("get_mcp_server_status"),
          resetPortDraft,
        );
      } catch {
        // 无法重新读取时保留最后一次确认过的状态。
      }
      mcpError = String(error);
    } finally {
      mcpLoading = false;
    }
  }

  async function copyMcpConnection(content: string, label: string) {
    mcpCopyMessage = "";
    try {
      await navigator.clipboard.writeText(content);
      mcpCopyMessage = `${label}已复制`;
    } catch {
      mcpCopyMessage = "复制失败，请手动选择并复制下方内容";
    }
  }

  function handleEndpointChange() {
    endpointValue = normalizeEndpoint(endpointValue);
    localStorage.setItem("endpoint", endpointValue);
    // reload page
    location.reload();
  }

  async function get_config() {
    let config: Config = await invoke("get_config");
    config.auto_generate.render_settings = normalizeRenderSettings(config.auto_generate.render_settings);
    setting_model = config;
    autoGenerateLoading = false;
    console.log(config);
  }

  async function updateAutoGenerate() {
    if (autoGenerateLoading || autoGenerateSaving) return;
    autoGenerateSaving = true;
    autoGenerateError = "";
    const settings = setting_model.auto_generate;
    try {
      await invoke("update_auto_generate", {
        enabled: settings.enabled,
        encodeDanmu: settings.encode_danmu,
        renderSettings: { ...settings.render_settings },
      });
    } catch (error) {
      autoGenerateError = "自动生成设置保存失败：" + String(error);
    } finally {
      autoGenerateSaving = false;
    }
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

  async function handleWhisperModelPathChange() {
    const selected = await open({
      multiple: false,
      filters: [{ name: "Whisper Model", extensions: ["bin"] }],
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

  async function update_danmu_ass_options() {
    await invoke("update_danmu_ass_options", {
      fontSize: setting_model.danmu_ass_options.font_size,
      opacity: setting_model.danmu_ass_options.opacity,
    });
  }

  function handleLlmProviderChange() {
    const endpoint = setting_model.llm.endpoint.trim();
    if (
      !endpoint ||
      endpoint === "https://api.openai.com/v1" ||
      endpoint === "http://localhost:11434"
    ) {
      setting_model.llm.endpoint =
        setting_model.llm.provider === "ollama"
          ? "http://localhost:11434"
          : "https://api.openai.com/v1";
    }
    llmModels = [];
    llmSaveMessage = "";
    llmError = "";
    void saveLlmConfig();
  }

  async function loadLlmModels() {
    if (!setting_model.llm.endpoint.trim()) return;
    llmModelsLoading = true;
    llmError = "";
    try {
      llmModels = await invoke<string[]>("list_llm_models", {
        provider: setting_model.llm.provider,
        endpoint: setting_model.llm.endpoint,
        apiKey: setting_model.llm.api_key,
      });
      if (llmModels.length === 0) {
        llmError = "未获取到可用模型，也可以手动输入模型名称";
      }
    } catch (error) {
      llmError = String(error);
    } finally {
      llmModelsLoading = false;
    }
  }

  async function toggleModelList() {
    if (!setting_model.llm.endpoint.trim()) return;
    if (!llmModels.length) {
      await loadLlmModels();
    }
    if (!llmError) {
      showModelList = !showModelList;
      llmSaveMessage = "";
    }
  }

  function selectModel(model: string) {
    setting_model.llm.model = model;
    showModelList = false;
    void saveLlmConfig();
  }

  async function saveLlmConfig() {
    llmSaving = true;
    llmError = "";
    llmSaveMessage = "";
    // endpoint/provider 任一变化都可能指向不同服务商，旧的模型列表立即失效
    llmModels = [];
    showModelList = false;
    // 串行保存：blur 触发与下拉选择触发的保存排队执行，
    // payload 在真正调用时读取最新状态，避免旧值覆盖新选中的模型
    const run = llmSaveChain.then(async () => {
      setting_model.llm.endpoint =
        setting_model.llm.endpoint.trim() ||
        (setting_model.llm.provider === "ollama"
          ? "http://localhost:11434"
          : "https://api.openai.com/v1");
      const payload = {
        provider: setting_model.llm.provider,
        endpoint: setting_model.llm.endpoint,
        apiKey: setting_model.llm.api_key,
        model: setting_model.llm.model,
      };
      try {
        await invoke("update_llm_config", payload);
        llmSaveMessage = "模型配置已保存，助手和 Summary 将共用此配置";
        window.dispatchEvent(new CustomEvent("llm-config-updated"));
      } catch (error) {
        llmError = String(error);
      }
    });
    llmSaveChain = run;
    await run;
    llmSaving = false;
  }

  async function loadLaunchAtLogin() {
    if (!TAURI_ENV) return;
    try {
      launchAtLogin = await isEnabled();
    } catch (e) {
      console.error("Failed to read autostart status:", e);
    }
  }

  async function updateLaunchAtLogin() {
    const next = launchAtLogin;
    try {
      if (next) {
        await enable();
      } else {
        await disable();
      }
    } catch (e) {
      launchAtLogin = !next;
      alert(e);
    }
  }

  onMount(async () => {
    await get_config();
    await Promise.all([
      loadLaunchAtLogin(),
      ...(TAURI_ENV || endpoint !== "" ? [loadMcpServerStatus()] : []),
    ]);
  });
</script>

<PageTitlebar title="设置" />

<div bind:this={scroller} class="flex-1 overflow-auto custom-scrollbar-light bg-canvas">
  <div>
    <div class="p-5 space-y-5">
      <!-- Settings Sections -->
      <div class="ui-card" data-setting="powerlive">
        <PowerLiveAccount {active} />
      </div>
      <div class="space-y-5 pb-6">
        <div class="space-y-3">
          <h2 class="flex items-center gap-2 text-[14px] font-semibold text-fg">
            <FileText class="h-4 w-4 text-fg-muted" />
            <span>基础设置</span>
          </h2>
          <div class="ui-card divide-y divide-line">
            <div class="px-4 py-3" data-setting="theme">
              <div class="flex items-center justify-between gap-6">
                <div>
                  <h3 class="ui-label">外观主题</h3>
                  <p class="ui-hint mt-0.5">
                    选择主窗口的外观；预览与切片编辑窗口将始终使用暗色。
                  </p>
                </div>
                <div class="ui-tabs shrink-0" role="radiogroup" aria-label="外观主题">
                  {#each themeOptions as option (option.value)}
                    <button
                      type="button"
                      role="radio"
                      class="ui-tab"
                      class:active={$themePreference === option.value}
                      aria-checked={$themePreference === option.value}
                      onclick={() => setThemePreference(option.value)}
                    >
                      {option.label}
                    </button>
                  {/each}
                </div>
              </div>
            </div>
            {#if TAURI_ENV}
              <div class="px-4 py-3" data-setting="launch-at-login">
                <div class="flex items-center justify-between gap-6">
                  <div>
                    <h3 class="ui-label">
                      开机自动启动
                    </h3>
                    <p class="ui-hint mt-0.5">
                      登录系统后自动打开应用
                    </p>
                  </div>
                  <label class="ui-switch">
                    <input
                      type="checkbox"
                      bind:checked={launchAtLogin}
                      onchange={updateLaunchAtLogin}
                    />
                    <span></span>
                  </label>
                </div>
              </div>
            {/if}
            <div class="px-4 py-3" data-setting="status-check-interval">
              <div class="flex items-center justify-between gap-6">
                <div>
                  <h3 class="ui-label">
                    直播间状态检查间隔
                  </h3>
                  <p class="ui-hint mt-0.5">
                    设置直播间状态检查的时间间隔，单位为秒，过于频繁可能会触发风控
                  </p>
                </div>
                <div class="flex items-center space-x-2">
                  <input
                    type="number"
                    class="ui-input w-24"
                    bind:value={setting_model.status_check_interval}
                    onblur={update_status_check_interval}
                  />
                </div>
              </div>
            </div>
            <div class="px-4 py-3" data-setting="webhook">
              <div class="flex items-center justify-between gap-6">
                <div>
                  <h3 class="ui-label">
                    Webhook URL
                  </h3>
                  <p class="ui-hint mt-0.5">
                    设置 Webhook URL，用于接收事件通知，见<a
                      href="https://bsr.xinrea.cn/usage/features/webhook.html"
                      class="text-accent-fg hover:underline"
                      target="_blank">Webhook 文档</a
                    >
                  </p>
                </div>
                <div class="flex items-center space-x-2">
                  <input
                    type="text"
                    class="ui-input w-96"
                    bind:value={setting_model.webhook_url}
                    onchange={update_webhook_url}
                    placeholder="https://example.com/webhook"
                  />
                </div>
              </div>
            </div>
          </div>
        </div>
        <!-- API Server Settings -->
        {#if !TAURI_ENV}
          <div class="space-y-3">
            <h2 class="flex items-center gap-2 text-[14px] font-semibold text-fg">
              <FileText class="h-4 w-4 text-fg-muted" />
              <span>API 服务器配置</span>
            </h2>
            <div class="ui-card divide-y divide-line">
              <div class="px-4 py-3" data-setting="api-endpoint">
                <div class="flex items-center justify-between gap-6">
                  <div>
                    <h3 class="ui-label">
                      API 服务器地址
                    </h3>
                    <p class="ui-hint mt-0.5">
                      设置 API 服务器的地址
                    </p>
                  </div>
                  <div class="flex items-center space-x-2">
                    <input
                      type="text"
                      class="ui-input w-96"
                      bind:value={endpointValue}
                      onblur={handleEndpointChange}
                      placeholder="http://localhost:3000"
                    />
                  </div>
                </div>
              </div>
            </div>
          </div>
        {/if}

        {#if TAURI_ENV || endpoint != ""}
          <!-- MCP Server Settings -->
          <div class="space-y-3">
            <h2 class="flex items-center gap-2 text-[14px] font-semibold text-fg">
              <Plug class="h-4 w-4 text-fg-muted" />
              <span>MCP Server</span>
            </h2>
            <div class="ui-card divide-y divide-line">
              <div class="px-4 py-3 space-y-3" data-setting="mcp-enabled">
                <div class="flex items-center justify-between gap-6">
                  <div>
                    <h3 class="ui-label">
                      启用 MCP Server
                    </h3>
                    <p class="ui-hint mt-0.5">
                      允许本机外部 Agent 查询录播、管理录制及生成素材等操作，修改立即生效并自动保存
                    </p>
                  </div>
                  <label class="ui-switch">
                    <input
                      type="checkbox"
                      aria-label="启用 MCP Server"
                      bind:checked={setting_model.mcp_enabled}
                      onchange={updateMcpServer}
                      disabled={mcpLoading || !mcpStatus}
                    />
                    <span></span>
                  </label>
                </div>
                <div class="flex items-center gap-2" aria-live="polite">
                  <span
                    class={mcpStatus?.running
                      ? "ui-badge ui-badge-success"
                      : "ui-badge"}
                  >
                    {mcpLoading
                      ? "正在更新状态…"
                      : !mcpStatus
                        ? "无法获取服务状态"
                        : mcpStatus.running
                          ? "运行中"
                          : mcpStatus.enabled
                            ? "已启用，服务未运行"
                            : "已关闭"}
                  </span>
                  <button
                    type="button"
                    class="ui-btn ui-btn-sm ui-btn-ghost"
                    onclick={loadMcpServerStatus}
                    disabled={mcpLoading}
                  >
                    <RefreshCw class="w-3.5 h-3.5" />
                    刷新状态
                  </button>
                </div>
                {#if mcpError}
                  <p class="text-[12px] text-danger-fg whitespace-pre-wrap" role="alert">
                    {mcpError}
                  </p>
                {/if}
              </div>
              <div class="px-4 py-3 space-y-3" data-setting="mcp-port">
                <div class="flex flex-wrap items-center justify-between gap-4">
                  <div>
                    <label
                      for="mcp-server-port"
                      class="ui-label"
                    >
                      MCP 端口
                    </label>
                    <p class="ui-hint mt-0.5">
                      默认 18763，可使用 1024–65535；保存后立即生效，服务关闭时也会保存
                    </p>
                  </div>
                  <div class="flex items-center gap-2">
                    <input
                      id="mcp-server-port"
                      type="number"
                      min="1024"
                      max="65535"
                      step="1"
                      class="ui-input w-28"
                      bind:value={mcpPortDraft}
                      disabled={mcpLoading || !mcpStatus}
                      aria-invalid={mcpPortError ? "true" : "false"}
                      aria-describedby={mcpPortError ? "mcp-port-error" : undefined}
                      oninput={() => {
                        mcpPortError = "";
                        mcpSaveMessage = "";
                      }}
                    />
                    <button
                      type="button"
                      class="ui-btn ui-btn-primary"
                      onclick={updateMcpPort}
                      disabled={mcpLoading || !mcpStatus}
                    >
                      保存端口
                    </button>
                  </div>
                </div>
                {#if mcpPortChanged}
                  <p class="text-[12px] text-fg-muted">
                    端口更改尚未保存，开关仍使用已保存的端口 {mcpStatus?.port}。
                  </p>
                {/if}
                {#if mcpPortError}
                  <p id="mcp-port-error" class="text-[12px] text-danger-fg" role="alert">
                    {mcpPortError}
                  </p>
                {/if}
                {#if mcpSaveMessage}
                  <p class="text-[12px] text-success-fg break-all" role="status">
                    {mcpSaveMessage}
                  </p>
                {/if}
              </div>
              <div class="px-4 py-3 space-y-3" data-setting="mcp-url">
                <div class="flex items-center justify-between gap-3">
                  <h3 class="ui-label">
                    连接地址（Streamable HTTP）
                  </h3>
                  <button
                    type="button"
                    class="ui-btn ui-btn-sm"
                    onclick={() => copyMcpConnection(mcpUrl, "地址")}
                  >
                    <Copy class="w-3.5 h-3.5" />
                    复制地址
                  </button>
                </div>
                <code
                  class="block select-text break-all rounded-[8px] border border-line bg-sunken px-3 py-2 text-[12px] text-fg"
                >{mcpUrl}</code>
                <p class="ui-hint">
                  服务仅监听 BSR 所在机器的 127.0.0.1。外部客户端负责操作确认；BSR 只读模式下仅提供查询工具。见<a
                    href="https://bsr.xinrea.cn/usage/features/mcp.html"
                    class="text-accent-fg hover:underline"
                    target="_blank"
                    rel="noreferrer">MCP 使用文档</a
                  >。
                </p>
                <details>
                  <summary class="cursor-pointer text-[12px] text-fg-secondary hover:text-fg">
                    客户端配置示例
                  </summary>
                  <div class="mt-3 space-y-2">
                    <pre
                      class="overflow-auto select-text rounded-[8px] border border-line bg-sunken p-3 text-[11px] text-fg"
                    ><code>{mcpClientConfig}</code></pre>
                    <div class="flex flex-wrap items-center justify-between gap-2">
                      <p class="ui-hint">
                        配置格式因客户端而异，请选择 HTTP 传输并使用上方 URL
                      </p>
                      <button
                        type="button"
                        class="ui-btn ui-btn-sm"
                        onclick={() => copyMcpConnection(mcpClientConfig, "配置")}
                      >
                        <Copy class="w-3.5 h-3.5" />
                        复制配置
                      </button>
                    </div>
                  </div>
                </details>
                {#if mcpCopyMessage}
                  <p class="text-[12px] text-fg-muted" aria-live="polite">
                    {mcpCopyMessage}
                  </p>
                {/if}
              </div>
            </div>
          </div>
          <!-- Storage Settings -->
          {#if TAURI_ENV}
            <div class="space-y-3">
              <h2 class="flex items-center gap-2 text-[14px] font-semibold text-fg">
                <HardDrive class="h-4 w-4 text-fg-muted" />
                <span>存储设置</span>
              </h2>
              <div class="ui-card divide-y divide-line">
                <!-- Cache Location -->
                <div class="px-4 py-3" data-setting="cache-path">
                  <div class="flex items-center justify-between gap-6">
                    <div>
                      <h3 class="ui-label">
                        缓存路径
                      </h3>
                      <p class="ui-hint mt-0.5 break-all">
                        {setting_model.cache}
                      </p>
                    </div>
                    <button class="ui-btn" onclick={handleCacheChange}>
                      变更
                    </button>
                  </div>
                </div>
                <div class="px-4 py-3" data-setting="output-path">
                  <div class="flex items-center justify-between gap-6">
                    <div>
                      <h3 class="ui-label">
                        素材保存路径
                      </h3>
                      <p class="ui-hint mt-0.5">
                        生成的素材和切片工程文件存放在这里
                      </p>
                      <p class="ui-hint mt-0.5 break-all">
                        {setting_model.output}
                      </p>
                    </div>
                    <button class="ui-btn" onclick={handleOutputChange}>
                      变更
                    </button>
                  </div>
                </div>
                <div class="px-4 py-3" data-setting="log-folder">
                  <div class="flex items-center justify-between gap-6">
                    <div>
                      <h3 class="ui-label">
                        日志文件夹
                      </h3>
                      <p class="ui-hint mt-0.5">
                        查看应用程序日志文件
                      </p>
                    </div>
                    <button class="ui-btn" onclick={handleLogFolder}>
                      打开
                    </button>
                  </div>
                </div>
              </div>
            </div>
          {/if}

          <!-- Notification Settings -->
          <div class="space-y-3">
            <h2 class="flex items-center gap-2 text-[14px] font-semibold text-fg">
              <Bell class="h-4 w-4 text-fg-muted" />
              <span>通知设置</span>
            </h2>
            <div class="ui-card divide-y divide-line">
              <!-- Stream Start -->
              <div class="px-4 py-3" data-setting="notify-live-start">
                <div class="flex items-center justify-between gap-6">
                  <div>
                    <h3 class="ui-label">
                      直播开始通知
                    </h3>
                    <p class="ui-hint mt-0.5">
                      当直播间开始直播时，会收到通知
                    </p>
                  </div>
                  <label class="ui-switch">
                    <input
                      type="checkbox"
                      bind:checked={setting_model.live_start_notify}
                      onchange={update_notify}
                    />
                    <span></span>
                  </label>
                </div>
              </div>
              <div class="px-4 py-3" data-setting="notify-live-end">
                <div class="flex items-center justify-between gap-6">
                  <div>
                    <h3 class="ui-label">
                      下播通知
                    </h3>
                    <p class="ui-hint mt-0.5">
                      当直播间结束直播时，会收到通知
                    </p>
                  </div>
                  <label class="ui-switch">
                    <input
                      type="checkbox"
                      bind:checked={setting_model.live_end_notify}
                      onchange={update_notify}
                    />
                    <span></span>
                  </label>
                </div>
              </div>
              <div class="px-4 py-3" data-setting="notify-clip">
                <div class="flex items-center justify-between gap-6">
                  <div>
                    <h3 class="ui-label">
                      素材生成通知
                    </h3>
                    <p class="ui-hint mt-0.5">
                      生成新素材后，会收到通知
                    </p>
                  </div>
                  <label class="ui-switch">
                    <input
                      type="checkbox"
                      bind:checked={setting_model.clip_notify}
                      onchange={update_notify}
                    />
                    <span></span>
                  </label>
                </div>
              </div>
              <div class="px-4 py-3" data-setting="notify-post">
                <div class="flex items-center justify-between gap-6">
                  <div>
                    <h3 class="ui-label">
                      投稿完成通知
                    </h3>
                    <p class="ui-hint mt-0.5">
                      当投稿完成时，会收到通知
                    </p>
                  </div>
                  <label class="ui-switch">
                    <input
                      type="checkbox"
                      bind:checked={setting_model.post_notify}
                      onchange={update_notify}
                    />
                    <span></span>
                  </label>
                </div>
              </div>
            </div>
          </div>

          <!-- Shared LLM Settings -->
          <div class="space-y-3">
            <h2 class="flex items-center gap-2 text-[14px] font-semibold text-fg">
              <Bot class="h-4 w-4 text-fg-muted" />
              <span>AI 模型</span>
            </h2>
            <div class="ui-card divide-y divide-line" data-setting="llm">
              <div class="px-4 py-3" data-setting="llm-provider">
                <div class="flex items-center justify-between gap-6">
                  <div>
                    <h3 class="ui-label">
                      模型提供商
                    </h3>
                    <p class="ui-hint mt-0.5">
                      此模型配置由 AI 助手和录播 Summary 共用
                    </p>
                  </div>
                  <select
                    class="ui-input w-96"
                    bind:value={setting_model.llm.provider}
                    onchange={handleLlmProviderChange}
                  >
                    <option value="openai">OpenAI 兼容 API</option>
                    <option value="ollama">Ollama</option>
                  </select>
                </div>
              </div>

              <div class="px-4 py-3" data-setting="llm-endpoint">
                <div class="flex items-center justify-between gap-6">
                  <div>
                    <h3 class="ui-label">
                      API Endpoint
                    </h3>
                    <p class="ui-hint mt-0.5">
                      {setting_model.llm.provider === "ollama"
                        ? "Ollama 服务地址"
                        : "兼容 OpenAI Chat Completions 的 API 地址"}
                    </p>
                  </div>
                  <input
                    type="text"
                    class="ui-input w-96"
                    bind:value={setting_model.llm.endpoint}
                    onblur={saveLlmConfig}
                    placeholder={setting_model.llm.provider === "ollama"
                      ? "http://localhost:11434"
                      : "https://api.openai.com/v1"}
                  />
                </div>
              </div>

              {#if setting_model.llm.provider === "openai"}
                <div class="px-4 py-3" data-setting="llm-api-key">
                  <div class="flex items-center justify-between gap-6">
                    <div>
                      <h3 class="ui-label">
                        API Key
                      </h3>
                      <p class="ui-hint mt-0.5">
                        密钥保存在 BSR 本地配置文件中
                      </p>
                    </div>
                    <input
                      type="password"
                      class="ui-input w-96"
                      bind:value={setting_model.llm.api_key}
                      onblur={saveLlmConfig}
                      placeholder="sk-..."
                    />
                  </div>
                </div>
              {/if}

              <div class="px-4 py-3" data-setting="llm-model">
                <div class="flex items-center justify-between gap-6">
                  <div>
                    <h3 class="ui-label">
                      模型名称
                    </h3>
                    <p class="ui-hint mt-0.5">
                      可以刷新模型列表或直接输入模型名称
                    </p>
                  </div>
                  <div class="relative flex w-96 flex-col">
                    <div class="flex items-center gap-2">
                      <input
                        type="text"
                        class="ui-input min-w-0 flex-1"
                        bind:value={setting_model.llm.model}
                        onblur={saveLlmConfig}
                        list="llm-model-options"
                        placeholder={setting_model.llm.provider === "ollama"
                          ? "qwen3, llama3.2..."
                          : "gpt-4.1-mini..."}
                      />
                      <datalist id="llm-model-options">
                        {#each llmModels as model}
                          <option value={model}>{model}</option>
                        {/each}
                      </datalist>
                      <button
                        class="ui-btn ui-btn-icon"
                        onclick={toggleModelList}
                        disabled={llmModelsLoading ||
                          !setting_model.llm.endpoint.trim() ||
                          (setting_model.llm.provider === "openai" &&
                            !setting_model.llm.api_key.trim())}
                        title="获取模型列表"
                      >
                        <RefreshCw
                          class="h-4 w-4 {llmModelsLoading
                            ? 'animate-spin'
                            : ''}"
                        />
                      </button>
                    </div>
                    {#if showModelList}
                      <div
                        use:clickOutside={() => (showModelList = false)}
                        role="listbox"
                        class="ui-menu absolute right-0 top-full z-20 mt-1 w-96 overflow-hidden"
                      >
                        {#if llmModelsLoading}
                          <div
                            class="flex items-center justify-center gap-2 px-2.5 py-2.5 text-[12px] text-fg-muted"
                          >
                            <RefreshCw class="h-3.5 w-3.5 animate-spin" />
                            获取中...
                          </div>
                        {:else if llmModels.length === 0}
                          <div
                            class="px-2.5 py-2.5 text-[12px] text-fg-muted"
                          >
                            未获取到可用模型，也可以手动输入模型名称
                          </div>
                        {:else}
                          <ul class="max-h-60 overflow-auto">
                            {#each llmModels as model}
                              <li role="presentation">
                                <button
                                  type="button"
                                  class="ui-menu-item whitespace-normal break-all"
                                  onclick={() => selectModel(model)}
                                >
                                  {model}
                                </button>
                              </li>
                            {/each}
                          </ul>
                        {/if}
                      </div>
                    {/if}
                  </div>
                </div>
                {#if llmError}
                  <div class="mt-2 min-h-[1.25rem] text-right">
                    <span class="text-[11px] text-danger-fg">
                      {llmError}
                    </span>
                  </div>
                {:else if llmSaving}
                  <div class="mt-2 min-h-[1.25rem] text-right">
                    <span class="text-[11px] text-fg-muted">
                      保存中...
                    </span>
                  </div>
                {:else if llmSaveMessage}
                  <div class="mt-2 min-h-[1.25rem] text-right">
                    <span class="text-[11px] text-success-fg">
                      {llmSaveMessage}
                    </span>
                  </div>
                {/if}
              </div>
            </div>
          </div>

          <!-- Subtitle Generation Settings -->
          <div class="space-y-3">
            <h2 class="flex items-center gap-2 text-[14px] font-semibold text-fg">
              <Captions class="h-4 w-4 text-fg-muted" />
              <span>字幕生成</span>
            </h2>
            <div class="ui-card divide-y divide-line">
              <!-- Auto Subtitle Generation -->
              <div class="px-4 py-3" data-setting="auto-subtitle">
                <div class="flex items-center justify-between gap-6">
                  <div>
                    <h3 class="ui-label">
                      自动生成字幕
                    </h3>
                    <p class="ui-hint mt-0.5">
                      启用后，素材生成后会自动识别字幕
                    </p>
                  </div>
                  <label class="ui-switch">
                    <input
                      type="checkbox"
                      bind:checked={setting_model.auto_subtitle}
                      onchange={update_subtitle_setting}
                    />
                    <span></span>
                  </label>
                </div>
              </div>
              <!-- Subtitle Generator Type -->
              <div class="px-4 py-3" data-setting="subtitle-generator">
                <div class="flex items-center justify-between gap-6">
                  <div>
                    <h3 class="ui-label">
                      字幕生成器类型
                    </h3>
                    <p class="ui-hint mt-0.5">
                      选择字幕生成的方式：本地模型，OpenAI 服务或 <a
                        href="https://www.powerlive.io/"
                        class="text-accent-fg hover:underline"
                        target="_blank"
                        rel="noopener noreferrer">PowerLive</a
                      > 服务（按量付费）
                    </p>
                  </div>
                  <div class="flex items-center space-x-2">
                    <select
                      class="ui-input"
                      bind:value={setting_model.subtitle_generator_type}
                      onchange={async () => {
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
              {#if setting_model.subtitle_generator_type === "powerlive"}
                <div class="px-4 py-3" data-setting="powerlive-hotwords">
                  <h3 class="ui-label">PowerLive 全局词库</h3>
                  <p class="ui-hint mt-0.5 mb-2">
                    所有直播间共用的热词，帮助识别和校对专有名词。直播间专属热词请在 AI 字幕生成窗口的「直播间词库」中设置。
                  </p>
                  <PowerLiveHotwords bind:hotwords={setting_model.powerlive_hotwords} />
                </div>
                <div class="px-4 py-3" data-setting="powerlive-sentence-length">
                  <div class="flex items-center justify-between gap-6">
                    <div>
                      <h3 class="ui-label">每条字幕最多字数</h3>
                      <p class="ui-hint mt-0.5">
                        PowerLive 按这个长度切分字幕，数值小时每条更短、更适合竖屏，范围 4–50
                      </p>
                    </div>
                    <PowerLiveSentenceLength bind:value={setting_model.powerlive_sentence_max_length} />
                  </div>
                </div>
              {/if}
              <!-- Whisper Model Path -->
              {#if setting_model.subtitle_generator_type !== "powerlive"}
                {#if TAURI_ENV && setting_model.subtitle_generator_type === "whisper"}
                  <div class="px-4 py-3" data-setting="whisper-model">
                    <div class="flex items-center justify-between gap-6">
                      <div>
                        <h3 class="ui-label">
                          Whisper 模型路径
                        </h3>
                        <p class="ui-hint mt-0.5 break-all">
                          {setting_model.whisper_model || "未设置"}
                          <span class="block mt-1"
                            >可前往 <a
                              href="https://huggingface.co/ggerganov/whisper.cpp/tree/main"
                              class="text-accent-fg hover:underline"
                              target="_blank"
                              rel="noopener noreferrer">ggerganov/whisper.cpp</a
                            > 下载模型文件</span
                          >
                        </p>
                      </div>
                      <button
                        class="ui-btn"
                        onclick={handleWhisperModelPathChange}>变更</button
                      >
                    </div>
                  </div>
                {/if}
                <!-- OpenAI API Settings -->
                {#if setting_model.subtitle_generator_type === "whisper_online"}
                  <div class="px-4 py-3" data-setting="whisper-endpoint">
                    <div class="flex items-center justify-between gap-6">
                      <div>
                        <h3 class="ui-label">
                          OpenAI API 端点
                        </h3>
                        <p class="ui-hint mt-0.5">
                          设置 OpenAI API 的端点地址，默认为官方地址
                        </p>
                      </div>
                      <div class="flex items-center space-x-2">
                        <input
                          type="text"
                          class="ui-input w-96"
                          bind:value={setting_model.openai_api_endpoint}
                          onchange={async () => {
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
                  <div class="px-4 py-3" data-setting="whisper-api-key">
                    <div class="flex items-center justify-between gap-6">
                      <div>
                        <h3 class="ui-label">
                          OpenAI API 密钥
                        </h3>
                        <p class="ui-hint mt-0.5">
                          设置 OpenAI API 的访问密钥
                        </p>
                      </div>
                      <div class="flex items-center space-x-2">
                        <input
                          type="password"
                          class="ui-input w-96"
                          bind:value={setting_model.openai_api_key}
                          onchange={async () => {
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
                {#if setting_model.subtitle_generator_type !== "powerlive"}
                  <div class="px-4 py-3" data-setting="whisper-prompt">
                    <div class="flex items-center justify-between gap-6">
                      <div>
                        <h3 class="ui-label">
                          Whisper 提示词
                        </h3>
                        <p class="ui-hint mt-0.5">
                          生成字幕时使用的提示词，尽量简洁明了，提示音频内容偏向的领域以及字幕的风格
                        </p>
                      </div>
                      <div class="flex items-center space-x-2">
                        <input
                          type="text"
                          class="ui-input w-96"
                          bind:value={setting_model.whisper_prompt}
                          onchange={async () => {
                            await invoke("update_whisper_prompt", {
                              whisperPrompt: setting_model.whisper_prompt,
                            });
                          }}
                        />
                      </div>
                    </div>
                  </div>
                {/if}
                <!-- Whisper Language -->
                <div class="px-4 py-3" data-setting="whisper-language">
                  <div class="flex items-center justify-between gap-6">
                    <div>
                      <h3 class="ui-label">
                        Whisper 语言
                      </h3>
                      <p class="ui-hint mt-0.5">
                        生成字幕时使用的语言，默认自动识别
                      </p>
                    </div>
                    <div class="flex items-center space-x-2">
                      <input
                        type="text"
                        class="ui-input w-96"
                        bind:value={setting_model.whisper_language}
                        onchange={async () => {
                          await invoke("update_whisper_language", {
                            whisperLanguage: setting_model.whisper_language,
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
          <div class="space-y-3">
            <h2 class="flex items-center gap-2 text-[14px] font-semibold text-fg">
              <DiscAlbum class="h-4 w-4 text-fg-muted" />
              <span>素材文件名格式</span>
            </h2>
            <div class="ui-card divide-y divide-line">
              <div class="px-4 py-3" data-setting="clip-name-format">
                <div class="flex items-center justify-between gap-6">
                  <div>
                    <h3 class="ui-label">
                      文件名格式
                    </h3>
                    <div class="mt-0.5 flex items-center space-x-2">
                      <p class="ui-hint">
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
                      <div
                        class="relative"
                        role="presentation"
                        use:clickOutside={() => (show_clip_name_help = false)}
                        onmouseenter={() => (show_clip_name_help = true)}
                        onmouseleave={() => (show_clip_name_help = false)}
                      >
                        <button
                          type="button"
                          class="text-[11px] text-accent-fg hover:underline"
                          onclick={() =>
                            (show_clip_name_help = !show_clip_name_help)}
                        >
                          详情
                        </button>
                        {#if show_clip_name_help}
                          <div
                            class="ui-menu absolute left-0 top-6 z-20 w-96 p-3 text-[11px] text-fg-secondary"
                          >
                            <div class="space-y-1">
                              <div>{"{title}"}: 直播标题</div>
                              <div>{"{platform}"}: 平台标识</div>
                              <div>{"{room_id}"}: 房间号</div>
                              <div>{"{live_id}"}: 录播 ID</div>
                              <div>{"{x}"}: 素材起始秒</div>
                              <div>{"{y}"}: 素材结束秒</div>
                              <div>{"{created_at}"}: 创建时间</div>
                              <div>{"{length}"}: 素材时长（秒）</div>
                              <div>{"{note}"}: 备注</div>
                            </div>
                          </div>
                        {/if}
                      </div>
                    </div>
                  </div>
                  <div class="flex items-center space-x-2">
                    <input
                      type="text"
                      class="ui-input w-96"
                      bind:value={setting_model.clip_name_format}
                      onchange={async () => {
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
          <div class="space-y-3">
            <h2 class="flex items-center gap-2 text-[14px] font-semibold text-fg">
              <Captions class="h-4 w-4 text-fg-muted" />
              <span>弹幕样式</span>
            </h2>
            <div class="ui-card divide-y divide-line">
              <!-- Font Size -->
              <div class="px-4 py-3" data-setting="danmu-font-size">
                <div class="flex items-center justify-between gap-6">
                  <div>
                    <h3 class="ui-label">
                      字体大小
                    </h3>
                    <p class="ui-hint mt-0.5">
                      设置预览和压制时的弹幕字体大小
                    </p>
                  </div>
                  <div class="flex items-center space-x-2">
                    <input
                      type="number"
                      class="ui-input w-24"
                      bind:value={setting_model.danmu_ass_options.font_size}
                      onblur={update_danmu_ass_options}
                      min="12"
                      max="72"
                      step="1"
                    />
                  </div>
                </div>
              </div>
              <!-- Opacity -->
              <div class="px-4 py-3" data-setting="danmu-opacity">
                <div class="flex items-center justify-between gap-6">
                  <div>
                    <h3 class="ui-label">
                      不透明度
                    </h3>
                    <p class="ui-hint mt-0.5">
                      设置预览和压制时的弹幕不透明度，范围
                      0.0-1.0，0.0为完全透明，1.0为完全不透明
                    </p>
                  </div>
                  <div class="flex items-center space-x-2">
                    <input
                      type="number"
                      class="ui-input w-24"
                      bind:value={setting_model.danmu_ass_options.opacity}
                      onblur={update_danmu_ass_options}
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
          <div class="space-y-3">
            <h2 class="flex items-center gap-2 text-[14px] font-semibold text-fg">
              <SquareBottomDashedScissors class="h-4 w-4 text-fg-muted" />
              <span>自动素材生成</span>
            </h2>
            <fieldset class="ui-card min-w-0 overflow-hidden" disabled={autoGenerateLoading || autoGenerateSaving}>
              <!-- Auto Clip Generation -->
              <div class="px-4 py-3" data-setting="auto-generate">
                <div class="flex items-center justify-between gap-6">
                  <div>
                    <h3 class="ui-label">
                      生成整场录播素材
                    </h3>
                    <p class="ui-hint mt-0.5">
                      直播结束后，自动生成整场录播素材
                    </p>
                  </div>
                  <label class="ui-switch">
                    <input
                      type="checkbox"
                      aria-label="生成整场录播素材"
                      bind:checked={setting_model.auto_generate.enabled}
                      onchange={updateAutoGenerate}
                    />
                    <span></span>
                  </label>
                </div>
              </div>
              <!-- A quiet settings area, not another card inside the card. -->
              {#if setting_model.auto_generate.enabled}
                <div
                  role="group"
                  aria-label="自动生成子选项"
                  class="space-y-4 border-t border-line bg-sunken/30 py-4 pl-6 pr-4 sm:pl-8"
                >
                  <div data-setting="auto-generate-danmu">
                    <div class="flex items-center justify-between gap-4">
                      <div>
                        <h3 class="text-[12px] font-normal text-fg-secondary">
                          压制弹幕
                        </h3>
                        <p class="ui-hint mt-0.5">
                          将弹幕写入视频画面，会增加生成时间
                        </p>
                      </div>
                      <label class="ui-switch">
                        <input
                          type="checkbox"
                          aria-label="整场生成时压制弹幕"
                          bind:checked={setting_model.auto_generate.encode_danmu}
                          onchange={updateAutoGenerate}
                        />
                        <span></span>
                      </label>
                    </div>
                  </div>
                  <div data-setting="auto-generate-encoding">
                    <RenderSettingsFields
                      layout="rows"
                      showHint={false}
                      bind:value={setting_model.auto_generate.render_settings}
                      disabled={autoGenerateLoading || autoGenerateSaving}
                      onChange={updateAutoGenerate}
                    />
                  </div>
                  <p class="ui-hint">
                    素材会按以上设置重新编码。修改自动保存，仅影响之后创建的任务。
                  </p>
                </div>
              {/if}
            </fieldset>
            {#if autoGenerateError}
              <div class="flex flex-wrap items-center gap-2">
                <p class="text-[12px] text-danger-fg" role="alert">{autoGenerateError}</p>
                <button class="ui-btn ui-btn-sm" onclick={updateAutoGenerate} disabled={autoGenerateSaving}>
                  重试保存
                </button>
              </div>
            {:else if autoGenerateSaving}
              <p class="ui-hint" role="status">正在保存自动生成设置…</p>
            {/if}
          </div>
        {/if}
      </div>
    </div>
  </div>
</div>

<!-- Modal -->
{#if showModal}
  <div class="ui-modal-backdrop">
    <div class="ui-modal w-full max-w-md">
      <div class="ui-modal-header">
        <h3 class="flex items-center gap-2">
          <AlertTriangle class="h-4 w-4 shrink-0 text-warning" />
          确认变更
        </h3>
      </div>
      <div class="ui-modal-body space-y-2 text-[13px] leading-relaxed text-fg-secondary">
        <p>
          根据文件大小，可能需要耗时较长时间，迁移期间直播间会暂时移除，迁移完成后直播间会自动恢复。
        </p>
        <p class="font-semibold text-fg">
          迁移期间请不要关闭程序，且不要在迁移期间再次更改目录！
        </p>
        <p>
          确认要进行变更吗？
        </p>
      </div>
      <div class="ui-modal-footer">
        <button
          class="ui-btn ui-btn-ghost"
          onclick={() => (showModal = false)}
        >
          取消
        </button>
        <button
          class="ui-btn ui-btn-primary"
          onclick={confirmChange}
        >
          确认
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  /* Option rows sit flush in rounded cards; round their highlight to match. */
  .divide-y > [data-setting]:first-child {
    border-top-left-radius: inherit;
    border-top-right-radius: inherit;
  }

  .divide-y > [data-setting]:last-child {
    border-bottom-left-radius: inherit;
    border-bottom-right-radius: inherit;
  }
</style>
