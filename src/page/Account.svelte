<script lang="ts">
  import { get, invoke } from "../lib/invoker";
  import { scale, fade } from "svelte/transition";
  import { Textarea } from "flowbite-svelte";
  import QRCode from "qrcode";
  import type { AccountItem, AccountInfo } from "../lib/db";
  import { Ellipsis, Plus } from "lucide-svelte";

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
  let activeTab = "qr"; // 'qr' or 'manual'
  let selectedPlatform = "bilibili"; // 'bilibili' or 'douyin'
  let oauth_key = "";
  let check_interval = null;
  let cookie_str = "";

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
      { qrcodeKey: oauth_key }
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
      tiktok: "TikTok"
    };
    return platformMap[platform] || platform;
  }

  function platform_avatar(platform: string) {
    const avatarMap = {
      bilibili: "/imgs/bilibili_avatar.png",
      douyin: "/imgs/douyin.svg",
      huya: "/imgs/huya_avatar.png",
      kuaishou: "/imgs/kuaishou.svg",
      tiktok: "/imgs/Tiktok.svg"
    };
    return avatarMap[platform] || "/imgs/bilibili_avatar.png";
  }
</script>

<svelte:window
  on:click={handleClickOutside}
  on:mousedown={handleModalClickOutside}
/>

<div class="flex-1 p-6 overflow-auto custom-scrollbar-light bg-gray-50 dark:bg-black">
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
      {#each account_info.accounts as account (account.uid)}
        <div
          class="p-4 rounded-xl bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 hover:border-blue-500 dark:hover:border-blue-400 transition-colors"
        >
          <div class="flex items-center justify-between">
            <div class="flex items-center space-x-4">
              <div class="relative shrink-0">
                <img
                  alt="avatar"
                  class="w-12 h-12 rounded-full object-cover"
                  src={account.avatar}
                />
              </div>
              <div>
                <div class="flex items-center space-x-2">
                  <span
                    class="inline-flex items-center px-2 py-1 text-xs font-medium rounded-full {account.platform ===
                    'bilibili'
                      ? 'bg-pink-100 text-pink-800 dark:bg-pink-900 dark:text-pink-200'
                    : account.platform === 'douyin' || account.platform === 'tiktok'
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
                <p class="text-sm text-gray-600 dark:text-gray-400">
                  UID: {account.uid}
                </p>
              </div>
            </div>
            <div class="flex items-center space-x-3">
              <div class="relative dropdown-container">
                <button
                  class="p-2 rounded-lg hover:bg-[#e5e5e5] dark:hover:bg-[#3a3a3c]"
                  on:click|stopPropagation={() => toggleDropdown(account.uid)}
                >
                  <Ellipsis class="w-5 h-5 dark:icon-white" />
                </button>
                {#if activeDropdown === account.uid}
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
          <div class="grid grid-cols-5 gap-2 p-0.5 bg-[#f5f5f7] dark:bg-[#1c1c1e] rounded-lg">
            <button
              class="px-3 py-2 text-sm font-medium rounded-md transition-colors {selectedPlatform ===
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
              class="px-3 py-2 text-sm font-medium rounded-md transition-colors {selectedPlatform ===
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
              class="px-3 py-2 text-sm font-medium rounded-md transition-colors {selectedPlatform ===
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
              class="px-3 py-2 text-sm font-medium rounded-md transition-colors {selectedPlatform ===
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
              class="px-3 py-2 text-sm font-medium rounded-md transition-colors {selectedPlatform ===
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
          {#if selectedPlatform === "bilibili" && activeTab === "qr"}
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
