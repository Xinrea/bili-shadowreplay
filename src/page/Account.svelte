<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { scale, fade } from "svelte/transition";
  import {
    Button,
    Card,
    Table,
    TableHead,
    TableHeadCell,
    TableBody,
    TableBodyRow,
    TableBodyCell,
    ButtonGroup,
    SpeedDial,
    Listgroup,
    ListgroupItem,
    Textarea,
    Hr,
  } from "flowbite-svelte";
  import Image from "../lib/Image.svelte";
  import QRCode from "qrcode";
  import type { AccountItem, AccountInfo } from "../lib/db";
  import { PlusOutline, UserAddSolid } from "flowbite-svelte-icons";

  let account_info: AccountInfo = {
    primary_uid: 0,
    accounts: [],
  };

  async function update_accounts() {
    account_info = await invoke("get_accounts");
  }

  update_accounts();

  let addModal = false;
  let activeTab = "qr"; // 'qr' or 'manual'
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
      await invoke("add_account", { cookies: qr_status.cookies });
      await update_accounts();
      addModal = false;
    }
  }

  async function add_cookie() {
    if (cookie_str == "") {
      return;
    }
    try {
      await invoke("add_account", { cookies: cookie_str });
      await update_accounts();
      cookie_str = "";
      addModal = false;
    } catch (e) {
      alert("Err adding cookie:" + e);
    }
  }
</script>

<svelte:window
  on:click={handleClickOutside}
  on:mousedown={handleModalClickOutside}
/>

<div class="flex-1 p-6 overflow-hidden">
  <div class="space-y-6 h-screen overflow-y-auto">
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
        <img
          src="https://unpkg.com/lucide-static@latest/icons/plus.svg"
          class="w-5 h-5 icon-white"
          alt="添加账号"
        />
        <span>添加账号</span>
      </button>
    </div>

    <!-- Account List -->
    <div class="space-y-4">
      <!-- Online Account -->
      {#each account_info.accounts.sort((a, b) => b.uid === account_info.primary_uid ? 1 : a.uid === account_info.primary_uid ? -1 : 0) as account (account.uid)}
        <div
          class="p-4 rounded-xl bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 hover:border-blue-500 dark:hover:border-blue-400 transition-colors"
        >
          <div class="flex items-center justify-between">
            <div class="flex items-center space-x-4">
              <div class="relative">
                <Image
                  iclass="w-12 h-12 rounded-full object-cover"
                  src={account.avatar}
                />
              </div>
              <div>
                <div class="flex items-center space-x-2">
                  <h3 class="font-medium text-gray-900 dark:text-white">
                    {account.name}
                  </h3>
                  {#if account.uid == account_info.primary_uid}
                    <span
                      class="px-2 py-0.5 rounded-full bg-blue-100 dark:bg-blue-500/20 text-blue-600 dark:text-blue-400 text-xs"
                      >主账号</span
                    >
                  {/if}
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
                  <img
                    src="https://unpkg.com/lucide-static@latest/icons/ellipsis.svg"
                    class="w-5 h-5 dark:icon-white"
                    alt="options"
                  />
                </button>
                {#if activeDropdown === account.uid}
                  <div
                    class="absolute right-0 mt-2 w-48 rounded-lg shadow-lg bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 backdrop-blur-xl bg-opacity-90 dark:bg-opacity-90"
                    style="transform-origin: top right;"
                    in:scale={{ duration: 100, start: 0.95 }}
                    out:scale={{ duration: 100, start: 0.95 }}
                  >
                    {#if account.uid !== account_info.primary_uid}
                      <button
                        class="w-full px-4 py-2 text-left text-sm text-gray-700 dark:text-white hover:bg-[#e5e5e5] dark:hover:bg-[#3a3a3c] rounded-t-lg"
                        on:click={async () => {
                          await invoke("set_primary", { uid: account.uid });
                          await update_accounts();
                          activeDropdown = null;
                        }}
                      >
                        设为主账号
                      </button>
                    {/if}
                    <button
                      class="w-full px-4 py-2 text-left text-sm text-red-600 hover:bg-[#e5e5e5] dark:hover:bg-[#3a3a3c] {account.uid !==
                      account_info.primary_uid
                        ? ''
                        : 'rounded-t-lg'} rounded-b-lg"
                      on:click={async () => {
                        await invoke("remove_account", { uid: account.uid });
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
            <img
              src="https://unpkg.com/lucide-static@latest/icons/plus.svg"
              class="w-6 h-6 icon-primary"
              alt="添加账号"
            />
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
        <!-- Tab Buttons -->
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

        <!-- Tab Content -->
        <div class="space-y-4">
          {#if activeTab === "qr"}
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
              <Textarea
                bind:value={cookie_str}
                rows="4"
                class="w-full px-3 py-2 bg-[#f5f5f7] dark:bg-[#1c1c1e] border-0 rounded-lg resize-none focus:ring-2 focus:ring-blue-500"
                placeholder="请粘贴 BiliBili 账号的 Cookie"
              />
              <div class="flex justify-end">
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
