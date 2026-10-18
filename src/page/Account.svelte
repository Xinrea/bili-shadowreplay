<script lang="ts">

  import { get, invoke } from "../lib/invoker";
  import { scale, fade } from "svelte/transition";
  import { Textarea } from "flowbite-svelte";
  import QRCode from "qrcode";
  import type { AccountItem, AccountInfo } from "../lib/db";
  import { Ellipsis, Plus } from "lucide-svelte";
  import PlatformIconTabs from "../lib/components/PlatformIconTabs.svelte";
  import PageTitlebar from "../lib/components/PageTitlebar.svelte";

  let account_info: AccountInfo = $state({
    accounts: [],
  });

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

  let addModal = $state(false);
  let activeTab = $state("qr"); // 'qr' or 'manual'
  let selectedPlatform = $state("bilibili"); // 'bilibili' or 'douyin'
  let oauth_key = "";
  let check_interval = null;
  let cookie_str = $state("");

  let manualModal = false;

  let activeDropdown = $state(null);

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
      youtube: "YouTube",
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
      youtube: "/imgs/youtube.svg",
    };
    return avatarMap[platform] || "/imgs/bilibili_avatar.png";
  }
</script>

<svelte:window
  onclick={handleClickOutside}
  onmousedown={handleModalClickOutside}
/>

<PageTitlebar title="账号">
  {#snippet subtitle()}
    <span class="whitespace-nowrap text-[12px] text-fg-muted">
      共 {account_info.accounts.length} 个
    </span>
  {/snippet}
  <button
    onclick={() => {
      addModal = true;
      if (activeTab === "qr") {
        requestAnimationFrame(handle_qr);
      }
    }}
    class="ui-btn ui-btn-primary h-7 gap-1 rounded-[7px] px-2.5"
  >
    <Plus class="h-3.5 w-3.5" />
    添加账号
  </button>
</PageTitlebar>

<div class="flex-1 overflow-auto bg-canvas p-6 custom-scrollbar-light">
  <div class="space-y-6">
    <!-- Account List -->
    <div class="space-y-3">
      <!-- Online Account -->
      {#each account_info.accounts as account (account.uid)}
        <div class="ui-card p-4 transition-colors hover:border-line-strong">
          <div class="flex items-center justify-between">
            <div class="flex min-w-0 items-center gap-3">
              <div class="relative shrink-0">
                <img
                  alt="avatar"
                  class="h-11 w-11 rounded-full border border-line object-cover"
                  src={account.avatar}
                />
              </div>
              <div class="min-w-0">
                <div class="flex items-center gap-2">
                  <span
                    class="ui-badge {account.platform === 'bilibili'
                      ? 'bg-pink-500/15 text-pink-500'
                      : account.platform === 'douyin' ||
                          account.platform === 'tiktok'
                        ? 'bg-black text-white'
                        : account.platform === 'huya'
                          ? 'text-white'
                          : ''}"
                    style={account.platform === "huya"
                      ? "background-color: #ff9600"
                      : ""}
                  >
                    {platform_display(account.platform)}
                  </span>
                  <h3 class="text-[13px] font-medium text-fg">
                    {account.name || account.uid}
                  </h3>
                </div>
                <p class="mt-1 text-[11px] text-fg-muted tabular-nums">
                  UID: {account.uid}
                </p>
              </div>
            </div>
            <div class="flex items-center gap-2">
              <div class="relative dropdown-container">
                <button
                  class="ui-btn ui-btn-ghost ui-btn-icon"
                  onclick={(event) => {
                    event.stopPropagation();
                    toggleDropdown(account.uid);
                  }}
                >
                  <Ellipsis class="h-4 w-4" />
                </button>
                {#if activeDropdown === account.uid}
                  <div
                    class="ui-menu absolute right-0 z-20 mt-1 w-40"
                    style="transform-origin: top right;"
                    in:scale={{ duration: 100, start: 0.95 }}
                    out:scale={{ duration: 100, start: 0.95 }}
                  >
                    <button
                      class="ui-menu-item danger"
                      onclick={async () => {
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
        class="w-full rounded-[10px] border border-dashed border-line-strong p-4 transition-colors hover:border-accent/60 hover:bg-accent-soft/40"
        onclick={() => {
          addModal = true;
          if (activeTab === "qr") {
            requestAnimationFrame(handle_qr);
          }
        }}
      >
        <div class="flex flex-col items-center justify-center space-y-2">
          <div
            class="flex h-10 w-10 items-center justify-center rounded-full bg-accent-soft text-accent-fg"
          >
            <Plus class="h-5 w-5" />
          </div>
          <div class="space-y-0.5 text-center">
            <p class="text-[13px] font-medium text-accent-fg">
              添加新账号
            </p>
            <p class="ui-hint">
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
    class="ui-modal-backdrop"
    transition:fade={{ duration: 200 }}
  >
    <div
      class="mac-modal ui-modal w-[400px]"
      transition:scale={{ duration: 150, start: 0.95 }}
    >
      <!-- Header -->
      <div class="ui-modal-header">
        <h2>
          添加账号
        </h2>
      </div>

      <div class="ui-modal-body space-y-5">
        <PlatformIconTabs
          bind:value={selectedPlatform}
          platforms={[
            { id: "bilibili", label: "哔哩哔哩" },
            { id: "douyin", label: "抖音" },
            { id: "huya", label: "虎牙" },
            { id: "kuaishou", label: "快手" },
            { id: "tiktok", label: "TikTok" },
          ]}
          onchange={(id) => {
            if (id === "bilibili") {
              activeTab = "qr";
              requestAnimationFrame(handle_qr);
            } else {
              activeTab = "manual";
            }
          }}
        />

        <!-- Login Methods (Only show for Bilibili) -->
        {#if selectedPlatform === "bilibili"}
          <div class="ui-tabs flex w-full">
            <button
              class="ui-tab flex-1"
              class:active={activeTab === "qr"}
              onclick={() => {
                activeTab = "qr";
                requestAnimationFrame(handle_qr);
              }}
            >
              扫码登录
            </button>
            <button
              class="ui-tab flex-1"
              class:active={activeTab === "manual"}
              onclick={() => {
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
            <div class="flex flex-col items-center space-y-3">
              <!-- QR codes need a white quiet zone to scan in either theme. -->
              <div class="rounded-[8px] border border-line bg-white p-3">
                <canvas id="qr"></canvas>
              </div>
              <p class="text-center text-[12px] text-fg-muted">
                请使用 BiliBili App 扫描二维码登录
              </p>
            </div>
          {:else}
            <div class="space-y-3">
              <p class="text-[12px] text-fg-secondary">
                <Textarea
                  bind:value={cookie_str}
                  rows={4}
                  class="ui-input w-full resize-none rounded-[8px] border-line-strong bg-control text-fg dark:border-line-strong dark:bg-control dark:text-fg dark:placeholder-fg-faint"
                  unWrappedClass="px-2.5 py-2 text-[12px] focus:border-accent focus:ring-0 dark:focus:border-accent dark:focus:ring-0 disabled:cursor-not-allowed disabled:opacity-50"
                  placeholder={`请粘贴 ${selectedPlatform} 账号的 Cookie`}
                />
              </p>
              <div class="flex items-center justify-end gap-3">
                {#if selectedPlatform !== "bilibili"}
                  <a
                    href="https://bsr.xinrea.cn/getting-started/config/account.html"
                    class="text-[12px] text-accent-fg hover:underline"
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    Cookie 获取教程</a
                  >
                {/if}
                <button
                  class="ui-btn ui-btn-primary"
                  onclick={() => {
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
