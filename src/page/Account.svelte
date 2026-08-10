<script lang="ts">
  import { get, invoke, open } from "../lib/invoker";
  import { scale, fade } from "svelte/transition";
  import { Textarea } from "flowbite-svelte";
  import QRCode from "qrcode";
  import type { AccountItem, AccountInfo } from "../lib/db";
  import { Ellipsis, ExternalLink, Plus } from "lucide-svelte";
  import { onDestroy } from "svelte";

  let account_info: AccountInfo = {
    accounts: [],
  };

  let avatar_cache: Map<string, string> = new Map();

  async function update_accounts() {
    let new_account_info = (await invoke("get_accounts")) as AccountInfo;
    for (const account of new_account_info.accounts) {
      if (account.avatar === "") {
        account.avatar = platform_avatar(account.platform);
        continue;
      }
      if (avatar_cache.has(account.avatar)) {
        account.avatar = avatar_cache.get(account.avatar);
        continue;
      }
      const avatar_response = await get(account.avatar);
      const avatar_blob = await avatar_response.blob();
      const avatar_url = URL.createObjectURL(avatar_blob);
      avatar_cache.set(account.avatar, avatar_url);
      account.avatar = avatar_url;
    }
    account_info = new_account_info;
  }

  update_accounts();

  let addModal = false;
  let activeTab = "qr"; // 'qr', 'manual', or 'joi-button'
  let selectedPlatform = "bilibili"; // 'bilibili', 'douyin', or 'joi-button'
  let oauth_key = "";
  let check_interval = null;
  let cookie_str = "";

  type JoiAuthState = {
    state: string;
    challenge?: string;
    pollToken?: string;
    roomId?: number;
    expiresAt?: string;
    expiresInSeconds?: number;
    canAssertNotSeen?: boolean;
    resend?: boolean;
    pollAfterMs?: number;
    detail?: string;
    token?: string;
    submitter?: { openId: string; displayName: string };
  };

  let joiEndpoint = "";
  let joiAuth: JoiAuthState | null = null;
  let joiChallengeError = "";
  let joiPollTimer: ReturnType<typeof setInterval> | null = null;
  let joiPollBusy = false;
  let joiBiliUid = "";
  let joiMessage = "";
  let joiRemainingSeconds = 0;
  let biliAccounts: AccountItem[] = [];
  let joiPlatformButton: HTMLButtonElement | undefined;

  $: biliAccounts = account_info.accounts.filter(
    (account) => account.platform === "bilibili",
  );

  $: {
    if (biliAccounts.length === 0) {
      joiBiliUid = "";
    } else if (!biliAccounts.some((account) => account.uid === joiBiliUid)) {
      joiBiliUid = biliAccounts[0].uid;
    }
  }

  function clearJoiPoll() {
    if (joiPollTimer) {
      clearInterval(joiPollTimer);
      joiPollTimer = null;
    }
    joiPollBusy = false;
  }

  function updateJoiCountdown() {
    if (!joiAuth?.expiresAt) {
      joiRemainingSeconds = 0;
      return;
    }
    joiRemainingSeconds = Math.max(
      0,
      Math.ceil((Date.parse(joiAuth.expiresAt) - Date.now()) / 1000),
    );
    if (joiRemainingSeconds === 0 && joiAuth.state !== "verified") {
      clearJoiPoll();
      joiChallengeError = "验证窗口已结束，请重新获取口令。";
    }
  }

  function setJoiAuth(auth: JoiAuthState) {
    joiAuth = auth;
    updateJoiCountdown();
  }

  function revealJoiPlatform() {
    requestAnimationFrame(() => {
      joiPlatformButton?.scrollIntoView({
        inline: "nearest",
        block: "nearest",
      });
    });
  }

  async function pollJoiChallenge() {
    if (!joiAuth?.pollToken || joiPollBusy) return;
    joiPollBusy = true;
    try {
      const auth = (await invoke("joi_button_poll", {
        endpoint: joiEndpoint,
        pollToken: joiAuth.pollToken,
      })) as JoiAuthState;
      setJoiAuth(auth);
      if (auth.state === "verified" && auth.token && auth.submitter?.openId) {
        clearJoiPoll();
        await invoke("add_joi_button_account", {
          endpoint: joiEndpoint,
          accessToken: auth.token,
          tokenExpiresAt: auth.expiresAt || "",
          openId: auth.submitter.openId,
          displayName: auth.submitter.displayName,
        });
        await update_accounts();
        joiChallengeError = "验证成功，轴伊按钮账号已保存。";
        addModal = false;
      }
    } catch (error) {
      const message = String(error).replace(/^Error: /, "");
      if (message.includes("expired_poll_token")) {
        clearJoiPoll();
        joiChallengeError = "验证窗口已结束，请重新获取口令。";
      } else {
        joiChallengeError = message;
      }
    } finally {
      joiPollBusy = false;
    }
  }

  async function startJoiChallenge() {
    clearJoiPoll();
    joiChallengeError = "";
    joiAuth = null;
    if (!joiEndpoint.trim()) {
      joiChallengeError = "请先填写自托管轴伊按钮地址。";
      return;
    }
    try {
      const auth = (await invoke("joi_button_challenge", {
        endpoint: joiEndpoint,
      })) as JoiAuthState;
      setJoiAuth(auth);
      joiPollTimer = setInterval(() => {
        updateJoiCountdown();
        pollJoiChallenge();
      }, 2000);
    } catch (error) {
      joiChallengeError = String(error).replace(/^Error: /, "");
    }
  }

  async function copyJoiChallenge() {
    if (joiAuth?.challenge) {
      await navigator.clipboard.writeText(joiAuth.challenge);
    }
  }

  async function openJoiRoom() {
    if (joiAuth?.roomId) {
      await open(`https://live.bilibili.com/${joiAuth.roomId}`);
    }
  }

  async function sendJoiChallenge() {
    if (!joiAuth?.roomId || !joiAuth.challenge || !joiBiliUid) return;
    try {
      await invoke("send_danmaku", {
        uid: joiBiliUid,
        roomId: String(joiAuth.roomId),
        message: joiAuth.challenge,
      });
      joiMessage = "已发送到验证房间；发送成功不代表验证已完成。";
    } catch (error) {
      joiMessage = `发送失败：${String(error).replace(/^Error: /, "")}`;
    }
  }

  function tokenRemainingDays(value?: string | null) {
    if (!value) return "—";
    return Math.max(0, Math.ceil((Date.parse(value) - Date.now()) / 86400000));
  }

  let manualModal = false;

  let activeDropdown = null;

  function toggleDropdown(uid) {
    if (activeDropdown === uid) {
      activeDropdown = null;
    } else {
      activeDropdown = uid;
    }
  }

  // Close dropdown when clicking outside
  function handleClickOutside(event) {
    if (
      activeDropdown !== null &&
      !event.target.closest(".dropdown-container")
    ) {
      activeDropdown = null;
    }
  }

  function handleModalClickOutside(event) {
    const modal = document.querySelector(".mac-modal");
    if (
      modal &&
      !modal.contains(event.target) &&
      !event.target.closest("button")
    ) {
      addModal = false;
      clearJoiPoll();
    }
  }

  async function handle_qr() {
    if (check_interval) {
      clearInterval(check_interval);
    }
    let qr_info: { url: string; oauthKey: string } = await invoke("get_qr");
    oauth_key = qr_info.oauthKey;
    const canvas = document.getElementById("qr");
    QRCode.toCanvas(canvas, qr_info.url, function (error) {
      if (error) {
        console.log(error);
        return;
      }
      canvas.style.display = "block";
      check_interval = setInterval(check_qr, 2000);
    });
  }

  async function check_qr() {
    let qr_status: { code: number; cookies: string } = await invoke(
      "get_qr_status",
      { qrcodeKey: oauth_key },
    );
    if (qr_status.code == 0) {
      clearInterval(check_interval);
      await invoke("add_account", {
        cookies: qr_status.cookies,
        platform: selectedPlatform,
      });
      await update_accounts();
      addModal = false;
    }
  }

  async function add_cookie() {
    if (cookie_str == "") {
      return;
    }
    try {
      console.log("add_cookie", selectedPlatform);
      await invoke("add_account", {
        cookies: cookie_str,
        platform: selectedPlatform,
      });
      await update_accounts();
      cookie_str = "";
      addModal = false;
    } catch (e) {
      alert("添加账号失败：" + e);
    }
  }

  function platform_display(platform: string) {
    const platformMap = {
      bilibili: "B站",
      douyin: "抖音",
      huya: "虎牙",
      kuaishou: "快手",
      tiktok: "TikTok",
      "joi-button": "轴伊按钮",
    };
    return platformMap[platform] || platform;
  }

  function platform_avatar(platform: string) {
    const avatarMap = {
      bilibili: "/imgs/bilibili_avatar.png",
      douyin: "/imgs/douyin.png",
      huya: "/imgs/huya_avatar.png",
      kuaishou: "/imgs/kuaishou.svg",
      tiktok: "/imgs/tiktok.png",
      "joi-button": "/imgs/bilibili_avatar.png",
    };
    return avatarMap[platform] || "/imgs/bilibili_avatar.png";
  }

  onDestroy(clearJoiPoll);
</script>

<svelte:window
  on:click={handleClickOutside}
  on:mousedown={handleModalClickOutside}
/>

<div
  class="flex-1 p-6 overflow-auto custom-scrollbar-light bg-gray-50 dark:bg-black"
>
  <div class="space-y-6">
    <!-- Header -->
    <div class="flex justify-between items-center">
      <div class="flex items-center space-x-4">
        <h1 class="text-2xl font-semibold text-gray-900 dark:text-white">
          账号
        </h1>
        <div
          class="flex items-center space-x-2 text-sm text-gray-500 dark:text-gray-400"
        >
          <span> 共 {account_info.accounts.length} 个</span>
        </div>
      </div>
      <button
        on:click={() => {
          addModal = true;
          if (activeTab === "qr") {
            requestAnimationFrame(handle_qr);
          }
        }}
        class="px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors flex items-center space-x-2"
      >
        <Plus class="w-5 h-5 icon-white" />
        <span>添加账号</span>
      </button>
    </div>

    <!-- Account List -->
    <div class="space-y-4">
      <!-- Online Account -->
      {#each account_info.accounts as account (`${account.platform}:${account.uid}`)}
        <div
          class="p-4 rounded-xl bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 hover:border-blue-500 dark:hover:border-blue-400 transition-colors"
        >
          <div class="flex items-center justify-between">
            <div class="flex items-center space-x-4">
              <div class="relative shrink-0">
                {#if account.platform === "joi-button"}
                  <div
                    class="w-12 h-12 rounded-full bg-blue-500/10 flex items-center justify-center text-xl"
                    aria-label="轴伊按钮"
                  >
                    🎛️
                  </div>
                {:else}
                  <img
                    alt="avatar"
                    class="w-12 h-12 rounded-full object-cover"
                    src={account.avatar}
                  />
                {/if}
              </div>
              <div class="min-w-0">
                <div class="flex items-center space-x-2">
                  <span
                    class="inline-flex items-center px-2 py-1 text-xs font-medium rounded-full {account.platform ===
                    'bilibili'
                      ? 'bg-pink-100 text-pink-800 dark:bg-pink-900 dark:text-pink-200'
                      : account.platform === 'douyin' ||
                          account.platform === 'tiktok'
                        ? 'bg-black text-white'
                        : account.platform === 'huya'
                          ? 'text-white'
                          : 'bg-gray-100 text-gray-800 dark:bg-gray-700 dark:text-gray-200'}"
                    style={account.platform === "huya"
                      ? "background-color: #ff9600"
                      : ""}
                  >
                    {platform_display(account.platform)}
                  </span>
                  <h3 class="font-medium text-gray-900 dark:text-white">
                    {account.name || account.uid}
                  </h3>
                </div>
                {#if account.platform === "joi-button"}
                  <p class="text-sm text-gray-600 dark:text-gray-400 truncate">
                    {account.endpoint}
                  </p>
                  <p class="text-xs text-gray-500 dark:text-gray-400">
                    令牌剩余 {tokenRemainingDays(account.token_expires_at)} 天
                  </p>
                {:else}
                  <p class="text-sm text-gray-600 dark:text-gray-400">
                    UID: {account.uid}
                  </p>
                {/if}
              </div>
            </div>
            <div class="flex items-center space-x-3">
              <div class="relative dropdown-container">
                <button
                  class="p-2 rounded-lg hover:bg-[#e5e5e5] dark:hover:bg-[#3a3a3c]"
                  on:click|stopPropagation={() =>
                    toggleDropdown(`${account.platform}:${account.uid}`)}
                >
                  <Ellipsis class="w-5 h-5 dark:icon-white" />
                </button>
                {#if activeDropdown === `${account.platform}:${account.uid}`}
                  <div
                    class="absolute right-0 mt-2 w-48 rounded-lg shadow-lg bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 backdrop-blur-xl bg-opacity-90 dark:bg-opacity-90"
                    style="transform-origin: top right;"
                    in:scale={{ duration: 100, start: 0.95 }}
                    out:scale={{ duration: 100, start: 0.95 }}
                  >
                    <button
                      class="w-full px-4 py-2 text-left text-sm text-red-600 hover:bg-[#e5e5e5] dark:hover:bg-[#3a3a3c] rounded-t-lg rounded-b-lg"
                      on:click={async () => {
                        await invoke("remove_account", {
                          platform: account.platform,
                          uid: account.uid,
                        });
                        await update_accounts();
                        activeDropdown = null;
                      }}
                    >
                      注销账号
                    </button>
                  </div>
                {/if}
              </div>
            </div>
          </div>
        </div>
      {/each}

      <!-- Add Account Card -->
      <button
        class="w-full p-4 rounded-xl border-2 border-dashed border-gray-300 dark:border-gray-600 hover:border-blue-500 dark:hover:border-blue-400 transition-colors"
        on:click={() => {
          addModal = true;
          if (activeTab === "qr") {
            requestAnimationFrame(handle_qr);
          }
        }}
      >
        <div class="flex flex-col items-center justify-center space-y-2">
          <div
            class="w-12 h-12 rounded-full bg-blue-500/10 flex items-center justify-center"
          >
            <Plus class="w-6 h-6 icon-primary" />
          </div>
          <div class="text-center">
            <p class="text-sm font-medium text-blue-600 dark:text-blue-400">
              添加新账号
            </p>
            <p class="text-xs text-gray-500 dark:text-gray-400">
              添加一个新账号，用于获取直播流和投稿
            </p>
          </div>
        </div>
      </button>
    </div>
  </div>
</div>

{#if addModal}
  <div
    class="fixed inset-0 bg-black/20 dark:bg-black/40 backdrop-blur-sm z-50 flex items-center justify-center"
    transition:fade={{ duration: 200 }}
  >
    <div
      class="mac-modal w-[400px] bg-white dark:bg-[#323234] rounded-xl shadow-xl overflow-hidden"
      transition:scale={{ duration: 150, start: 0.95 }}
    >
      <!-- Header -->
      <div class="px-6 py-4 border-b border-gray-200 dark:border-gray-700/50">
        <h2 class="text-base font-medium text-gray-900 dark:text-white">
          添加账号
        </h2>
      </div>

      <div class="p-6 space-y-6">
        <!-- Platform Selection -->
        <div class="space-y-2">
          <label
            for="platform"
            class="block text-sm font-medium text-gray-700 dark:text-gray-300"
          >
            平台
          </label>
          <div
            class="flex items-center gap-2 p-0.5 bg-[#f5f5f7] dark:bg-[#1c1c1e] rounded-lg overflow-x-auto custom-scrollbar-light"
          >
            <button
              class="flex-none px-3 py-2 text-sm font-medium whitespace-nowrap rounded-md transition-colors {selectedPlatform ===
              'bilibili'
                ? 'bg-white dark:bg-[#3c3c3e] shadow-sm text-gray-900 dark:text-white'
                : 'text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white'}"
              on:click={() => {
                selectedPlatform = "bilibili";
                activeTab = "qr";
                requestAnimationFrame(handle_qr);
              }}
            >
              哔哩哔哩
            </button>
            <button
              class="flex-none px-3 py-2 text-sm font-medium whitespace-nowrap rounded-md transition-colors {selectedPlatform ===
              'douyin'
                ? 'bg-white dark:bg-[#3c3c3e] shadow-sm text-gray-900 dark:text-white'
                : 'text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white'}"
              on:click={() => {
                selectedPlatform = "douyin";
                activeTab = "manual";
              }}
            >
              抖音
            </button>
            <button
              class="flex-none px-3 py-2 text-sm font-medium whitespace-nowrap rounded-md transition-colors {selectedPlatform ===
              'huya'
                ? 'bg-white dark:bg-[#3c3c3e] shadow-sm text-gray-900 dark:text-white'
                : 'text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white'}"
              on:click={() => {
                selectedPlatform = "huya";
                activeTab = "manual";
              }}
            >
              虎牙
            </button>
            <button
              class="flex-none px-3 py-2 text-sm font-medium whitespace-nowrap rounded-md transition-colors {selectedPlatform ===
              'kuaishou'
                ? 'bg-white dark:bg-[#3c3c3e] shadow-sm text-gray-900 dark:text-white'
                : 'text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white'}"
              on:click={() => {
                selectedPlatform = "kuaishou";
                activeTab = "manual";
              }}
            >
              快手
            </button>
            <button
              class="flex-none px-3 py-2 text-sm font-medium whitespace-nowrap rounded-md transition-colors {selectedPlatform ===
              'tiktok'
                ? 'bg-white dark:bg-[#3c3c3e] shadow-sm text-gray-900 dark:text-white'
                : 'text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white'}"
              on:click={() => {
                selectedPlatform = "tiktok";
                activeTab = "manual";
              }}
            >
              TikTok
            </button>
            <button
              bind:this={joiPlatformButton}
              class="flex-none px-3 py-2 text-sm font-medium whitespace-nowrap rounded-md transition-colors {selectedPlatform ===
              'joi-button'
                ? 'bg-white dark:bg-[#3c3c3e] shadow-sm text-gray-900 dark:text-white'
                : 'text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white'}"
              on:click={() => {
                selectedPlatform = "joi-button";
                activeTab = "joi-button";
                if (check_interval) {
                  clearInterval(check_interval);
                  check_interval = null;
                }
                clearJoiPoll();
                revealJoiPlatform();
              }}
            >
              轴伊按钮
            </button>
          </div>
        </div>

        <!-- Login Methods (Only show for Bilibili) -->
        {#if selectedPlatform === "bilibili"}
          <div class="flex rounded-lg bg-[#f5f5f7] dark:bg-[#1c1c1e] p-1">
            <button
              class="flex-1 px-4 py-1.5 text-sm rounded-md transition-colors {activeTab ===
              'qr'
                ? 'bg-white dark:bg-[#3c3c3e] shadow-sm font-medium'
                : 'text-gray-600 dark:text-gray-400'}"
              on:click={() => {
                activeTab = "qr";
                requestAnimationFrame(handle_qr);
              }}
            >
              扫码登录
            </button>
            <button
              class="flex-1 px-4 py-1.5 text-sm rounded-md transition-colors {activeTab ===
              'manual'
                ? 'bg-white dark:bg-[#3c3c3e] shadow-sm font-medium'
                : 'text-gray-600 dark:text-gray-400'}"
              on:click={() => {
                activeTab = "manual";
              }}
            >
              手动输入
            </button>
          </div>
        {/if}

        <!-- Tab Content -->
        <div class="space-y-4">
          {#if selectedPlatform === "joi-button"}
            <div class="space-y-4">
              <div class="space-y-2">
                <label
                  for="joi-endpoint"
                  class="block text-sm font-medium text-gray-700 dark:text-gray-300"
                >自托管地址</label>
                <input
                  id="joi-endpoint"
                  type="url"
                  bind:value={joiEndpoint}
                  placeholder="https://button.example.com"
                  class="w-full min-w-0 px-3 py-2 bg-[#f5f5f7] dark:bg-[#1c1c1e] border-0 rounded-lg focus:ring-2 focus:ring-blue-500"
                />
              </div>
              {#if joiAuth?.challenge}
                <div class="space-y-3">
                  <p class="text-sm text-gray-600 dark:text-gray-400">
                    请在直播间发送这句话：
                  </p>
                  <div
                    class="px-4 py-3.5 rounded-lg text-center text-lg font-semibold bg-[#f5f5f7] dark:bg-[#1c1c1e] select-all"
                  >
                    {joiAuth.challenge}
                  </div>
                  <div class="flex flex-wrap gap-2">
                    <button
                      type="button"
                      class="text-sm text-blue-500 hover:underline inline-flex items-center gap-1"
                      on:click={openJoiRoom}
                    >打开直播间 {joiAuth.roomId} <ExternalLink class="w-4 h-4" /></button
                    >
                    <button
                      class="px-4 py-2 text-gray-600 dark:text-gray-400 rounded-lg hover:bg-[#e5e5e5] dark:hover:bg-[#3a3a3c]"
                      on:click={copyJoiChallenge}
                    >复制口令</button
                    >
                  </div>
                  {#if biliAccounts.length > 0}
                    <div class="grid grid-cols-[minmax(0,1fr)_auto] gap-3 items-end">
                      <div class="space-y-2 min-w-0">
                        <label
                          for="joi-bili-account"
                          class="block text-sm font-medium text-gray-700 dark:text-gray-300"
                        >发送账号</label>
                        <select
                          id="joi-bili-account"
                          bind:value={joiBiliUid}
                          class="w-full min-w-0 px-3 py-2 bg-[#f5f5f7] dark:bg-[#1c1c1e] border-0 rounded-lg focus:ring-2 focus:ring-blue-500 appearance-none"
                        >
                          <option value="">选择 B 站账号</option>
                          {#each biliAccounts as account}
                            <option value={account.uid}>{account.name}</option>
                          {/each}
                        </select>
                      </div>
                      <button
                        class="px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors disabled:opacity-50"
                        disabled={!joiBiliUid}
                        on:click={sendJoiChallenge}
                      >一键发送</button
                      >
                    </div>
                  {:else}
                    <p class="text-xs text-gray-500 dark:text-gray-400">
                      添加 B 站账号后可一键发送。
                    </p>
                  {/if}
                  <p class="text-xs text-gray-500 dark:text-gray-400">
                    房间号 {joiAuth.roomId} · 验证窗口 {joiRemainingSeconds} 秒
                  </p>
                  {#if joiAuth.canAssertNotSeen === false}
                    <div class="p-2.5 rounded-lg text-xs flex gap-2 items-start bg-yellow-100 dark:bg-yellow-900/30 text-yellow-800 dark:text-yellow-300">
                      连接刚才有过中断，这次窗口不能判断口令是否已看到，请再发送一次。
                    </div>
                  {/if}
                </div>
              {/if}
              <div class="flex justify-end">
                <button
                  class="px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors disabled:opacity-50"
                  disabled={!joiEndpoint.trim() || joiPollBusy}
                  on:click={startJoiChallenge}
                >{joiAuth ? "重新获取口令" : "获取验证口令"}</button
                >
              </div>
              {#if joiAuth?.state === "room-unreachable"}
                <div class="p-2.5 rounded-lg text-xs flex gap-2 items-start bg-yellow-100 dark:bg-yellow-900/30 text-yellow-800 dark:text-yellow-300">
                  暂时收不到弹幕。连接恢复后会自动继续轮询。
                </div>
              {:else if joiAuth?.state === "preparing"}
                <div class="p-2.5 rounded-lg text-xs flex gap-2 items-start bg-yellow-100 dark:bg-yellow-900/30 text-yellow-800 dark:text-yellow-300">
                  正在准备验证房间，请稍候。
                </div>
              {/if}
              {#if joiMessage}
                <p class="text-xs text-blue-500">{joiMessage}</p>
              {/if}
              {#if joiChallengeError}
                <div class="p-2.5 rounded-lg text-xs flex gap-2 items-start bg-red-100 dark:bg-red-900/30 text-red-800 dark:text-red-300">
                  {joiChallengeError}
                </div>
              {/if}
            </div>
          {:else if selectedPlatform === "bilibili" && activeTab === "qr"}
            <div class="flex flex-col items-center space-y-4">
              <div class="bg-white p-4 rounded-lg">
                <canvas id="qr" />
              </div>
              <p class="text-sm text-center text-gray-600 dark:text-gray-400">
                请使用 BiliBili App 扫描二维码登录
              </p>
            </div>
          {:else}
            <div class="space-y-4">
              <p class="text-sm text-gray-600 dark:text-gray-400">
                <Textarea
                  bind:value={cookie_str}
                  rows={4}
                  class="w-full px-3 py-2 bg-[#f5f5f7] dark:bg-[#1c1c1e] border-0 rounded-lg resize-none focus:ring-2 focus:ring-blue-500"
                  placeholder={`请粘贴 ${selectedPlatform} 账号的 Cookie`}
                />
              </p>
              <div class="flex justify-end items-center space-x-2">
                {#if selectedPlatform !== "bilibili"}
                  <a
                    href="https://bsr.xinrea.cn/getting-started/config/account.html"
                    class="text-blue-500 hover:underline text-sm"
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    Cookie 获取教程</a
                  >
                {/if}
                <button
                  class="px-4 py-2 bg-[#0A84FF] hover:bg-[#0A84FF]/90 text-white text-sm font-medium rounded-lg transition-colors"
                  on:click={() => {
                    add_cookie();
                  }}
                >
                  添加账号
                </button>
              </div>
            </div>
          {/if}
        </div>
      </div>
    </div>
  </div>
{/if}
