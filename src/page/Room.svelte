<script lang="ts">
  import { invoke, open, onOpenUrl, get_cover, get } from "../lib/invoker";
  import { message } from "@tauri-apps/plugin-dialog";
  import { fade, scale } from "svelte/transition";
  import { Dropdown, DropdownItem } from "flowbite-svelte";
  import type { RecorderList, RecorderInfo } from "../lib/interface";
  import type { RecordItem } from "../lib/db";
  import {
    Ellipsis,
    Play,
    Plus,
    FileVideo,
    Search,
    Trash2,
    X,
    History,
    PlayIcon,
  } from "lucide-svelte";
  import BilibiliIcon from "../lib/components/BilibiliIcon.svelte";
  import DouyinIcon from "../lib/components/DouyinIcon.svelte";
  import AutoRecordIcon from "../lib/components/AutoRecordIcon.svelte";
  import { onMount } from "svelte";

  export let room_count = 0;
  let room_active = 0;
  let room_inactive = 0;

  let summary: RecorderList = {
    count: 0,
    recorders: [],
  };

  let searchQuery = "";
  $: filteredRecorders = summary.recorders.filter((room) => {
    const query = searchQuery.toLowerCase();
    return (
      room.room_info.room_title.toLowerCase().includes(query) ||
      room.user_info.user_name.toLowerCase().includes(query) ||
      room.room_id.toString().includes(query)
    );
  });

  function default_avatar(platform: string) {
    if (platform === "bilibili") {
      return "/imgs/bilibili_avatar.png";
    } else if (platform === "douyin") {
      return "/imgs/douyin_avatar.png";
    }
  }

  function default_cover(platform: string) {
    if (platform === "bilibili") {
      return "/imgs/bilibili.png";
    } else if (platform === "douyin") {
      return "/imgs/douyin.png";
    }
  }

  async function update_summary() {
    let new_summary = (await invoke("get_recorder_list")) as RecorderList;
    room_count = new_summary.count;
    room_active = new_summary.recorders.filter(
      (room) => room.live_status
    ).length;
    room_inactive = new_summary.recorders.filter(
      (room) => !room.live_status
    ).length;

    // sort new_summary.recorders by live_status
    new_summary.recorders.sort((a, b) => {
      if (a.live_status && !b.live_status) return -1;
      if (!a.live_status && b.live_status) return 1;
      return 0;
    });

    // process room cover
    for (const room of new_summary.recorders) {
      if (room.room_info.room_cover != "") {
        const cover_response = await get(room.room_info.room_cover);
        const cover_blob = await cover_response.blob();
        room.room_info.room_cover = URL.createObjectURL(cover_blob);
      } else {
        room.room_info.room_cover = default_cover(room.platform);
      }

      if (room.user_info.user_avatar != "") {
        const avatar_response = await get(room.user_info.user_avatar);
        const avatar_blob = await avatar_response.blob();
        room.user_info.user_avatar = URL.createObjectURL(avatar_blob);
      } else {
        room.user_info.user_avatar = default_avatar(room.platform);
      }
    }

    summary = new_summary;
  }
  update_summary();
  setInterval(update_summary, 5000);

  // modals
  let deleteModal = false;
  let deleteRoom = null;

  let addModal = false;
  let addRoom = "";
  let addSecUserId = "";
  let addValid = false;
  let addErrorMsg = "";
  let selectedPlatform = "bilibili";

  let archiveModal = false;
  let archiveRoom = null;
  let archives: RecordItem[] = [];

  // 分页相关状态
  let currentPage = 0;
  let pageSize = 20;
  let hasMore = true;
  let isLoading = false;
  let loadError = "";

  async function showArchives(room_id: number) {
    // 重置分页状态
    currentPage = 0;
    archives = [];
    hasMore = true;
    isLoading = false;
    loadError = "";

    updateArchives();
    archiveModal = true;
    console.log(archives);
  }

  async function updateArchives() {
    if (isLoading || !hasMore) return;

    isLoading = true;

    try {
      let new_archives = (await invoke("get_archives", {
        roomId: archiveRoom.room_id,
        offset: currentPage * pageSize,
        limit: pageSize,
      })) as RecordItem[];

      for (const archive of new_archives) {
        archive.cover = await get_cover("cache", archive.cover);
      }

      // 如果是第一页，直接替换；否则追加数据
      if (currentPage === 0) {
        archives = new_archives;
      } else {
        archives = [...archives, ...new_archives];
      }

      // 检查是否还有更多数据
      hasMore = new_archives.length === pageSize;

      // 按时间排序
      archives.sort((a, b) => {
        return (
          new Date(b.created_at).getTime() - new Date(a.created_at).getTime()
        );
      });

      // 更新总数（如果后端支持的话）
      // totalCount = await invoke("get_archives_count", { roomId: archiveRoom.room_id });

      currentPage++;
    } catch (error) {
      console.error("Failed to load archives:", error);
      loadError = "加载失败，请重试";
    } finally {
      isLoading = false;
    }
  }

  async function loadMoreArchives() {
    await updateArchives();
  }

  function handleScroll(event: Event) {
    const target = event.target as HTMLElement;
    const { scrollTop, scrollHeight, clientHeight } = target;

    // 当滚动到距离底部100px时，自动加载更多
    if (
      scrollHeight - scrollTop - clientHeight < 100 &&
      hasMore &&
      !isLoading
    ) {
      loadMoreArchives();
    }
  }

  function format_ts(ts_string: string) {
    const date = new Date(ts_string);
    return date.toLocaleString();
  }

  function format_duration(duration: number) {
    const hours = Math.floor(duration / 3600)
      .toString()
      .padStart(2, "0");
    const minutes = Math.floor((duration % 3600) / 60)
      .toString()
      .padStart(2, "0");
    const seconds = (duration % 60).toString().padStart(2, "0");

    return `${hours}:${minutes}:${seconds}`;
  }

  function format_size(size: number) {
    if (size < 1024) {
      return `${size} B`;
    } else if (size < 1024 * 1024) {
      return `${(size / 1024).toFixed(2)} KiB`;
    } else if (size < 1024 * 1024 * 1024) {
      return `${(size / 1024 / 1024).toFixed(2)} MiB`;
    } else {
      return `${(size / 1024 / 1024 / 1024).toFixed(2)} GiB`;
    }
  }

  function calc_bitrate(size: number, duration: number) {
    return ((size * 8) / duration / 1024).toFixed(0);
  }

  function handleModalClickOutside(event) {
    // 检查点击是否在任何modal内部
    const clickedElement = event.target;

    // 检查是否点击了按钮，如果是则不关闭modal
    if (clickedElement.closest("button")) {
      return;
    }

    // 按层级顺序检查modal，优先处理最上层的modal
    // 如果点击在最上层modal内部，则不处理任何modal关闭

    // 最上层：generateWholeClipModal
    if (generateWholeClipModal) {
      const generateWholeClipModalEl = document.querySelector(
        ".generate-whole-clip-modal"
      );
      if (generateWholeClipModalEl) {
        if (generateWholeClipModalEl.contains(clickedElement)) {
          // 点击在generateWholeClipModal内部，不关闭任何modal
          return;
        } else {
          // 点击在generateWholeClipModal外部，关闭它
          generateWholeClipModal = false;
          return;
        }
      }
    }

    // 第二层：archiveModal
    if (archiveModal) {
      const archiveModalEl = document.querySelector(".archive-modal");
      if (archiveModalEl) {
        if (archiveModalEl.contains(clickedElement)) {
          // 点击在archiveModal内部，不关闭任何modal
          return;
        } else {
          // 点击在archiveModal外部，关闭它
          archiveModal = false;
          return;
        }
      }
    }

    // 第三层：addModal
    if (addModal) {
      const addModalEl = document.querySelector(".add-modal");
      if (addModalEl) {
        if (addModalEl.contains(clickedElement)) {
          // 点击在addModal内部，不关闭任何modal
          return;
        } else {
          // 点击在addModal外部，关闭它
          addModal = false;
          return;
        }
      }
    }

    // 第四层：deleteModal
    if (deleteModal) {
      const deleteModalEl = document.querySelector(".delete-modal");
      if (deleteModalEl) {
        if (deleteModalEl.contains(clickedElement)) {
          // 点击在deleteModal内部，不关闭任何modal
          return;
        } else {
          // 点击在deleteModal外部，关闭它
          deleteModal = false;
          return;
        }
      }
    }
  }

  // Function to toggle auto-record state
  function toggleEnabled(room: RecorderInfo) {
    invoke("set_enable", {
      roomId: room.room_id,
      platform: room.platform,
      enabled: !room.auto_start,
    });
  }

  function openUserUrl(room: RecorderInfo) {
    if (room.platform === "bilibili") {
      open("https://space.bilibili.com/" + room.user_info.user_id);
    } else if (room.platform === "douyin") {
      console.log(room.user_info);
      open("https://www.douyin.com/user/" + room.user_info.user_id);
    }
  }

  function openLiveUrl(room: RecorderInfo) {
    if (room.platform === "bilibili") {
      open("https://live.bilibili.com/" + room.room_id);
    }

    if (room.platform === "douyin") {
      open("https://live.douyin.com/" + room.room_id);
    }
  }

  function addNewRecorder(room_id: number, platform: string, extra: string) {
    // if extra contains ?, remove it
    if (extra.includes("?")) {
      extra = extra.split("?")[0];
    }

    invoke("add_recorder", {
      roomId: room_id,
      platform: platform,
      extra: extra,
    })
      .then(() => {
        addModal = false;
        addRoom = "";
        addSecUserId = "";
      })
      .catch(async (e) => {
        await message(e);
      });
  }

  let generateWholeClipModal = false;
  let generateWholeClipArchive = null;
  let wholeClipArchives: RecordItem[] = [];
  let isLoadingWholeClip = false;

  async function openGenerateWholeClipModal(archive: RecordItem) {
    generateWholeClipModal = true;
    generateWholeClipArchive = archive;
    await loadWholeClipArchives(archiveRoom.room_id, archive.parent_id);
  }

  async function loadWholeClipArchives(roomId: number, parentId: string) {
    if (isLoadingWholeClip) return;

    isLoadingWholeClip = true;
    try {
      // 获取与当前archive具有相同parent_id的所有archives
      let sameParentArchives = (await invoke("get_archives_by_parent_id", {
        roomId: roomId,
        parentId: parentId,
      })) as RecordItem[];

      // 处理封面
      for (const archive of sameParentArchives) {
        archive.cover = await get_cover("cache", archive.cover);
      }

      // 按时间排序
      sameParentArchives.sort((a, b) => {
        return (
          new Date(a.created_at).getTime() - new Date(b.created_at).getTime()
        );
      });

      wholeClipArchives = sameParentArchives;
    } catch (error) {
      console.error("Failed to load whole clip archives:", error);
      wholeClipArchives = [];
    } finally {
      isLoadingWholeClip = false;
    }
  }

  async function generateWholeClip() {
    generateWholeClipModal = false;
    await invoke("generate_whole_clip", {
      platform: generateWholeClipArchive.platform,
      roomId: generateWholeClipArchive.room_id,
      parentId: generateWholeClipArchive.parent_id,
    });
  }

  onMount(async () => {
    await onOpenUrl((urls: string[]) => {
      console.log("Received Deep Link:", urls);
      if (urls.length > 0) {
        const url = urls[0];
        // extract platform and room_id from url
        // url example:
        // bsr://live.bilibili.com/167537?live_from=85001&spm_id_from=333.1365.live_users.item.click
        // bsr://live.douyin.com/200525029536

        let platform = "";
        let room_id = "";

        if (url.startsWith("bsr://live.bilibili.com/")) {
          // 1. remove bsr://live.bilibili.com/
          // 2. remove all query params
          room_id = url.replace("bsr://live.bilibili.com/", "").split("?")[0];
          platform = "bilibili";
        }

        if (url.startsWith("bsr://live.douyin.com/")) {
          room_id = url.replace("bsr://live.douyin.com/", "").split("?")[0];
          platform = "douyin";
        }

        if (platform && room_id) {
          addModal = true;
          addRoom = room_id;
          selectedPlatform = platform;

          if (Number.isInteger(Number(room_id))) {
            addValid = true;
          } else {
            addErrorMsg = "ID格式错误，请检查输入";
            addValid = false;
          }
        }
      }
    });
  });
</script>

<div class="flex-1 p-6 overflow-auto custom-scrollbar-light bg-gray-50">
  <div class="space-y-6">
    <!-- Header -->
    <div class="flex justify-between items-center">
      <div class="flex items-center space-x-4">
        <h1 class="text-2xl font-semibold text-gray-900 dark:text-white">
          直播间
        </h1>
        <div
          class="flex items-center space-x-2 text-sm text-gray-500 dark:text-gray-400"
        >
          <span class="flex items-center space-x-1">
            <span class="w-2 h-2 rounded-full bg-green-500"></span>
            <span>{room_active} 直播中</span>
          </span>
          <span>•</span>
          <span>{room_inactive} 未直播</span>
        </div>
      </div>
      <div class="flex items-center space-x-3">
        <div class="relative">
          <Search
            class="w-5 h-5 absolute left-3 top-1/2 transform -translate-y-1/2 text-gray-400 dark:text-gray-500"
          />
          <input
            type="text"
            bind:value={searchQuery}
            placeholder="搜索直播间..."
            class="pl-10 pr-4 py-2 rounded-lg bg-gray-100 dark:bg-gray-700/50 border border-gray-200 dark:border-gray-600 focus:outline-none focus:ring-2 focus:ring-blue-500 dark:focus:ring-blue-400 text-gray-900 dark:text-white"
          />
        </div>
        <button
          class="px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors flex items-center space-x-2"
          on:click={() => {
            addModal = true;
          }}
        >
          <Plus class="w-5 h-5 icon-white" />
          <span>添加新直播间</span>
        </button>
      </div>
    </div>

    <!-- Room Grid -->
    <div class="grid grid-cols-3 gap-4">
      <!-- Active Room Card -->
      {#each filteredRecorders as room (room.room_id)}
        <div
          class="p-4 rounded-xl bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 hover:border-blue-500 dark:hover:border-blue-400 transition-colors"
        >
          <div class="relative">
            <img
              src={room.room_info.room_cover}
              alt="cover"
              class={"w-full h-40 object-cover rounded-lg " +
                (room.live_status ? "" : "brightness-75")}
            />
            <!-- Room ID watermark -->
            <div
              class="absolute bottom-2 left-2 px-2 py-1 rounded-md bg-black/30 backdrop-blur-sm flex items-center"
            >
              <span class="text-xs text-white/80 font-mono"
                >{room.platform.toUpperCase()}#{room.room_id}</span
              >
            </div>
            {#if room.auto_start}
              <div
                class={"absolute top-2 left-2 p-1.5 px-2 rounded-md text-white text-xs flex items-center justify-center " +
                  (room.is_recording ? "bg-red-500" : "bg-gray-700/90")}
              >
                <AutoRecordIcon class="w-4 h-4 text-white" />
                {#if room.is_recording}
                  <span class="text-white ml-1">录制中</span>
                {/if}
              </div>
            {/if}
            {#if !room.live_status}
              <div
                class={"absolute bottom-2 right-2 p-1.5 px-2 rounded-md text-white text-xs flex items-center justify-center bg-gray-700"}
              >
                <span>直播未开始</span>
              </div>
            {:else}
              <div
                class={"absolute bottom-2 right-2 p-1.5 px-2 rounded-md text-white text-xs flex items-center justify-center bg-green-500"}
              >
                <span>直播进行中</span>
              </div>
            {/if}
            <button
              class="absolute top-2 right-2 p-1.5 rounded-lg bg-gray-900/50 hover:bg-gray-900/70 transition-colors"
            >
              <Ellipsis class="w-5 h-5 icon-white" />
            </button>
            <Dropdown class="whitespace-nowrap">
              <button
                class="px-4 py-2 flex items-center justify-between hover:bg-gray-100 dark:hover:bg-gray-700 cursor-pointer"
                on:click={() => toggleEnabled(room)}
              >
                <span
                  class="text-sm text-gray-700 dark:text-gray-200 font-medium"
                  >启用直播间</span
                >
                <label class="toggle-switch ml-1">
                  <input type="checkbox" checked={room.auto_start} />
                  <span class="toggle-slider"></span>
                </label>
              </button>
              <DropdownItem
                on:click={() => {
                  openLiveUrl(room);
                }}>打开网页直播间</DropdownItem
              >
              <DropdownItem
                class="text-red-500"
                on:click={() => {
                  deleteRoom = room;
                  deleteModal = true;
                }}>移除直播间</DropdownItem
              >
            </Dropdown>
          </div>
          <div class="mt-3 space-y-2">
            <div class="flex items-start justify-between">
              <div>
                <div class="flex items-center space-x-2">
                  {#if room.platform === "bilibili"}
                    <BilibiliIcon class="w-4 h-4" />
                  {:else if room.platform === "douyin"}
                    <DouyinIcon class="w-4 h-4" />
                  {/if}
                  <h3 class="font-medium text-gray-900 dark:text-white">
                    {room.room_info.room_title}
                  </h3>
                </div>
              </div>
            </div>
            <div class="flex items-center justify-between">
              <div
                class="flex items-center space-x-2 text-sm text-gray-600 dark:text-gray-400"
              >
                <button
                  class="flex items-center space-x-2 p-1.5 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-700"
                  on:click={() => {
                    openUserUrl(room);
                  }}
                >
                  <img
                    src={room.user_info.user_avatar}
                    alt="avatar"
                    class="w-8 h-8 rounded-full"
                  />
                  <span>{room.user_info.user_name}</span>
                </button>
              </div>
              <div class="flex items-center space-x-1">
                {#if room.is_recording}
                  <button
                    class="p-1.5 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-700"
                    on:click={() => {
                      invoke("open_live", {
                        platform: room.platform,
                        roomId: room.room_id,
                        liveId: room.current_live_id,
                      });
                    }}
                  >
                    <Play class="w-5 h-5 dark:icon-white" />
                  </button>
                {/if}
                <button
                  class="p-1.5 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-700"
                  on:click={() => {
                    archiveRoom = room;
                    showArchives(room.room_id);
                  }}
                >
                  <History class="w-5 h-5 dark:icon-white" />
                </button>
              </div>
            </div>
          </div>
        </div>
      {/each}

      <!-- Add Room Card -->
      <button
        class="p-4 rounded-xl border-2 border-dashed border-gray-300 dark:border-gray-600 hover:border-blue-500 dark:hover:border-blue-400 transition-colors flex flex-col items-center justify-center space-y-2"
        on:click={() => {
          addModal = true;
        }}
      >
        <div
          class="w-12 h-12 rounded-full bg-blue-500/10 flex items-center justify-center"
        >
          <Plus class="w-6 h-6 icon-primary" />
        </div>
        <span class="text-sm font-medium text-blue-600 dark:text-blue-400"
          >添加新直播间</span
        >
        <span class="text-xs text-gray-500 dark:text-gray-400"
          >配置一个新直播间以及其相关设置</span
        >
      </button>
    </div>
  </div>
</div>
{#if deleteModal}
  <div
    class="fixed inset-0 bg-black/20 dark:bg-black/40 backdrop-blur-sm z-50 flex items-center justify-center"
    transition:fade={{ duration: 200 }}
  >
    <div
      class="mac-modal delete-modal w-[320px] bg-white dark:bg-[#323234] rounded-xl shadow-xl overflow-hidden"
      transition:scale={{ duration: 150, start: 0.95 }}
    >
      <div class="p-6 space-y-4">
        <div class="text-center space-y-2">
          <h3 class="text-base font-medium text-gray-900 dark:text-white">
            移除直播间
          </h3>
          <p class="text-sm text-gray-500 dark:text-gray-400">
            此操作将移除所有相关的录制记录
          </p>
        </div>
        <div class="flex justify-center space-x-3">
          <button
            class="w-24 px-4 py-2 text-sm font-medium text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-600 rounded-lg transition-colors"
            on:click={() => {
              deleteModal = false;
            }}
          >
            取消
          </button>
          <button
            class="w-24 px-4 py-2 bg-red-600 hover:bg-red-700 text-white text-sm font-medium rounded-lg transition-colors"
            on:click={async () => {
              await invoke("remove_recorder", {
                roomId: deleteRoom.room_id,
                platform: deleteRoom.platform,
              });
              deleteModal = false;
            }}
          >
            移除
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

{#if addModal}
  <div
    class="fixed inset-0 bg-black/20 dark:bg-black/40 backdrop-blur-sm z-50 flex items-center justify-center"
    transition:fade={{ duration: 200 }}
  >
    <div
      class="mac-modal add-modal w-[400px] bg-white dark:bg-[#323234] rounded-xl shadow-xl overflow-hidden"
      transition:scale={{ duration: 150, start: 0.95 }}
    >
      <!-- Header -->
      <div class="px-6 py-4 border-b border-gray-200 dark:border-gray-700/50">
        <h2 class="text-base font-medium text-gray-900 dark:text-white">
          添加直播间
        </h2>
      </div>

      <div class="p-6 space-y-6">
        <div class="space-y-4">
          <div class="space-y-2">
            <label
              for="platform"
              class="block text-sm font-medium text-gray-700 dark:text-gray-300"
            >
              平台
            </label>
            <div class="flex p-0.5 bg-[#f5f5f7] dark:bg-[#1c1c1e] rounded-lg">
              <button
                class="flex-1 px-4 py-2 text-sm font-medium rounded-md transition-colors {selectedPlatform ===
                'bilibili'
                  ? 'bg-white dark:bg-[#323234] shadow-sm text-gray-900 dark:text-white'
                  : 'text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white'}"
                on:click={() => (selectedPlatform = "bilibili")}
              >
                哔哩哔哩
              </button>
              <button
                class="flex-1 px-4 py-2 text-sm font-medium rounded-md transition-colors {selectedPlatform ===
                'douyin'
                  ? 'bg-white dark:bg-[#323234] shadow-sm text-gray-900 dark:text-white'
                  : 'text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white'}"
                on:click={() => (selectedPlatform = "douyin")}
              >
                抖音
              </button>
            </div>
          </div>

          <div class="space-y-2">
            {#if selectedPlatform === "douyin"}
              <label
                for="sec_user_id"
                class="block text-sm font-medium text-gray-700 dark:text-gray-300"
              >
                主播 SEC_UID (
                <a
                  href="https://bsr.xinrea.cn/usage/features/room.html#%E6%89%8B%E5%8A%A8%E6%B7%BB%E5%8A%A0%E7%9B%B4%E6%92%AD%E9%97%B4"
                  target="_blank"
                  class="text-blue-500">如何获取</a
                >
                )
              </label>
              <input
                id="sec_user_id"
                type="text"
                bind:value={addSecUserId}
                placeholder="请输入主播的 SEC_UID（选填）"
                class="w-full px-3 py-2 bg-[#f5f5f7] dark:bg-[#1c1c1e] border-0 rounded-lg focus:ring-2 focus:ring-blue-500 text-gray-900 dark:text-white placeholder-gray-500 dark:placeholder-gray-400"
              />
            {/if}
            <label
              for="room_id"
              class="block text-sm font-medium text-gray-700 dark:text-gray-300"
            >
              {selectedPlatform === "bilibili" ? "房间号" : "直播间ID"}
            </label>
            <input
              id="room_id"
              type="text"
              bind:value={addRoom}
              class="w-full px-3 py-2 bg-[#f5f5f7] dark:bg-[#1c1c1e] border-0 rounded-lg focus:ring-2 focus:ring-blue-500 text-gray-900 dark:text-white placeholder-gray-500 dark:placeholder-gray-400"
              placeholder={selectedPlatform === "bilibili"
                ? "请输入直播间房间号"
                : "请输入抖音直播间ID"}
              on:change={() => {
                if (!addRoom) {
                  addErrorMsg = "";
                  addValid = false;
                  return;
                }
                const room_id = Number(addRoom);
                if (Number.isInteger(room_id) && room_id > 0) {
                  addErrorMsg = "";
                  addValid = true;
                } else {
                  addErrorMsg = "ID格式错误，请检查输入";
                  addValid = false;
                }
              }}
            />
            {#if addErrorMsg}
              <p class="text-sm text-red-600 dark:text-red-500">
                {addErrorMsg}
              </p>
            {/if}
          </div>

          <div class="flex justify-end space-x-3">
            <button
              class="px-4 py-2 text-sm font-medium text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-600 rounded-lg transition-colors"
              on:click={() => {
                addModal = false;
              }}
            >
              取消
            </button>
            <button
              class="px-4 py-2 bg-[#0A84FF] hover:bg-[#0A84FF]/90 text-white text-sm font-medium rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              disabled={!addValid}
              on:click={() => {
                addNewRecorder(Number(addRoom), selectedPlatform, addSecUserId);
                addModal = false;
                addRoom = "";
              }}
            >
              添加
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
{/if}

{#if archiveModal}
  <div
    class="fixed inset-0 bg-black/20 dark:bg-black/40 backdrop-blur-sm z-50 flex items-center justify-center"
    transition:fade={{ duration: 200 }}
  >
    <div
      class="mac-modal archive-modal w-[900px] bg-white dark:bg-[#323234] rounded-xl shadow-xl overflow-hidden flex flex-col max-h-[80vh]"
      transition:scale={{ duration: 150, start: 0.95 }}
    >
      <!-- Header -->
      <div
        class="flex justify-between items-center px-6 py-4 border-b border-gray-200 dark:border-gray-700/50"
      >
        <div class="flex items-center space-x-3">
          <h2 class="text-base font-medium text-gray-900 dark:text-white">
            直播间记录
          </h2>
          <span class="text-sm text-gray-500 dark:text-gray-400">
            {archiveRoom?.user_info.user_name} · {archiveRoom?.room_id}
          </span>
        </div>
        <button
          class="p-1.5 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-700/50 transition-colors"
          on:click={() => (archiveModal = false)}
        >
          <X class="w-5 h-5 dark:icon-white" />
        </button>
      </div>

      <div
        class="flex-1 overflow-auto custom-scrollbar-light"
        on:scroll={handleScroll}
      >
        <div class="p-6">
          <div class="overflow-x-auto custom-scrollbar-light">
            <table class="w-full">
              <thead>
                <tr class="border-b border-gray-200 dark:border-gray-700/50">
                  <th
                    class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400"
                    >直播时间</th
                  >
                  <th
                    class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400"
                    >标题</th
                  >
                  <th
                    class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400"
                    >时长</th
                  >
                  <th
                    class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400"
                    >大小</th
                  >
                  <th
                    class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400"
                    >码率</th
                  >
                  <th
                    class="px-4 py-3 text-left text-sm font-medium text-gray-500 dark:text-gray-400"
                    >操作</th
                  >
                </tr>
              </thead>
              <tbody class="divide-y divide-gray-200 dark:divide-gray-700/50">
                {#each archives as archive (archive.live_id)}
                  <tr
                    class="group hover:bg-[#f5f5f7] dark:hover:bg-[#3a3a3c] transition-colors"
                  >
                    <td class="px-4 py-3">
                      <div class="flex flex-col">
                        <span class="text-sm text-gray-900 dark:text-white"
                          >{format_ts(archive.created_at).split(" ")[0]}</span
                        >
                        <span class="text-xs text-gray-500 dark:text-gray-400"
                          >{format_ts(archive.created_at).split(" ")[1]}</span
                        >
                      </div>
                    </td>
                    <td class="px-4 py-3">
                      <div class="flex items-center space-x-3">
                        {#if archive.cover}
                          <img
                            src={archive.cover}
                            alt="cover"
                            class="w-12 h-8 rounded object-cover"
                          />
                        {/if}
                        <span class="text-sm text-gray-900 dark:text-white"
                          >{archive.title}</span
                        >
                      </div>
                    </td>
                    <td class="px-4 py-3 text-sm text-gray-900 dark:text-white"
                      >{format_duration(archive.length)}</td
                    >
                    <td class="px-4 py-3 text-sm text-gray-900 dark:text-white"
                      >{format_size(archive.size)}</td
                    >
                    <td
                      class="px-4 py-3 text-sm text-gray-500 dark:text-gray-400"
                      >{calc_bitrate(archive.size, archive.length)} Kbps</td
                    >
                    <td class="px-4 py-3">
                      <div class="flex items-center space-x-2">
                        <button
                          class="p-1.5 rounded-lg hover:bg-blue-500/10 transition-colors"
                          title="预览录播"
                          on:click={() => {
                            invoke("open_live", {
                              platform: archiveRoom.platform,
                              roomId: archiveRoom.room_id,
                              liveId: archive.live_id,
                            });
                          }}
                        >
                          <PlayIcon class="w-4 h-4 icon-primary" />
                        </button>
                        <button
                          class="p-1.5 rounded-lg hover:bg-blue-500/10 transition-colors"
                          title="生成完整切片"
                          on:click={() => {
                            openGenerateWholeClipModal(archive);
                          }}
                        >
                          <FileVideo class="w-4 h-4 icon-primary" />
                        </button>
                        <button
                          class="p-1.5 rounded-lg hover:bg-red-500/10 transition-colors"
                          title="删除记录"
                          on:click={() => {
                            invoke("delete_archive", {
                              platform: archiveRoom.platform,
                              roomId: archiveRoom.room_id,
                              liveId: archive.live_id,
                            })
                              .then(async () => {
                                // 删除后重新加载第一页数据
                                currentPage = 0;
                                archives = [];
                                hasMore = true;
                                loadError = "";
                                await updateArchives();
                              })
                              .catch((e) => {
                                alert(e);
                              });
                          }}
                        >
                          <Trash2 class="w-4 h-4 text-red-500" />
                        </button>
                      </div>
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>

          <!-- 加载更多区域 -->
          <div class="mt-6 flex justify-center">
            {#if loadError}
              <div class="text-red-500 dark:text-red-400 text-sm mb-3">
                {loadError}
                <button
                  class="ml-2 text-blue-500 hover:text-blue-600 underline"
                  on:click={() => {
                    loadError = "";
                    loadMoreArchives();
                  }}
                >
                  重试
                </button>
              </div>
            {/if}

            {#if isLoading && currentPage === 0}
              <div
                class="flex items-center space-x-2 text-gray-500 dark:text-gray-400"
              >
                <div
                  class="animate-spin rounded-full h-5 w-5 border-b-2 border-blue-500"
                ></div>
                <span>加载中...</span>
              </div>
            {:else if isLoading}
              <div
                class="flex items-center space-x-2 text-gray-500 dark:text-gray-400"
              >
                <div
                  class="animate-spin rounded-full h-4 w-4 border-b-2 border-blue-500"
                ></div>
                <span>加载更多...</span>
              </div>
            {:else if archives.length === 0 && !isLoading}
              <div class="text-gray-500 dark:text-gray-400 text-sm">
                暂无录制记录
              </div>
            {/if}
          </div>
        </div>
      </div>
    </div>
  </div>
{/if}

{#if generateWholeClipModal}
  <div
    class="fixed inset-0 bg-black/20 dark:bg-black/40 backdrop-blur-sm z-50 flex items-center justify-center"
    transition:fade={{ duration: 200 }}
  >
    <div
      class="mac-modal generate-whole-clip-modal w-[800px] bg-white dark:bg-[#323234] rounded-xl shadow-xl overflow-hidden flex flex-col max-h-[80vh]"
      transition:scale={{ duration: 150, start: 0.95 }}
    >
      <!-- Header -->
      <div
        class="flex justify-between items-center px-6 py-4 border-b border-gray-200 dark:border-gray-700/50"
      >
        <div class="flex items-center space-x-3">
          <h2 class="text-base font-medium text-gray-900 dark:text-white">
            生成完整直播切片
          </h2>
          <span class="text-sm text-gray-500 dark:text-gray-400">
            {generateWholeClipArchive?.title || "直播片段"}
          </span>
        </div>
        <button
          class="p-1.5 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-700/50 transition-colors"
          on:click={() => (generateWholeClipModal = false)}
        >
          <X class="w-5 h-5 dark:icon-white" />
        </button>
      </div>

      <!-- Content -->
      <div class="flex-1 flex flex-col min-h-0">
        <!-- Description -->
        <div class="px-6 pt-6 pb-4">
          <p class="text-sm text-gray-600 dark:text-gray-400">
            以下是属于同一场直播的所有片段，将按时间顺序合成为一个完整的视频文件：
          </p>
        </div>

        <!-- Scrollable List -->
        <div class="flex-1 overflow-auto custom-scrollbar-light px-6 min-h-0">
          {#if isLoadingWholeClip}
            <div class="flex items-center justify-center py-8">
              <div
                class="flex items-center space-x-2 text-gray-500 dark:text-gray-400"
              >
                <div
                  class="animate-spin rounded-full h-5 w-5 border-b-2 border-blue-500"
                ></div>
                <span>加载中...</span>
              </div>
            </div>
          {:else if wholeClipArchives.length === 0}
            <div class="text-center py-8 text-gray-500 dark:text-gray-400">
              未找到相关片段
            </div>
          {:else}
            <div class="space-y-3 pb-4">
              {#each wholeClipArchives as archive, index (archive.live_id)}
                <div
                  class="flex items-center space-x-4 p-4 rounded-lg bg-gray-50 dark:bg-gray-700/30"
                >
                  <div
                    class="flex-shrink-0 w-8 h-8 rounded-full bg-blue-500 flex items-center justify-center text-white text-sm font-medium"
                  >
                    {index + 1}
                  </div>

                  {#if archive.cover}
                    <img
                      src={archive.cover}
                      alt="cover"
                      class="w-16 h-10 rounded object-cover flex-shrink-0"
                    />
                  {/if}

                  <div class="flex-1 min-w-0">
                    <div
                      class="text-sm font-medium text-gray-900 dark:text-white truncate"
                    >
                      {archive.title}
                    </div>
                    <div class="text-xs text-gray-500 dark:text-gray-400 mt-1">
                      {format_ts(archive.created_at)} · {format_duration(
                        archive.length
                      )} · {format_size(archive.size)}
                    </div>
                  </div>
                </div>
              {/each}
            </div>
          {/if}
        </div>

        <!-- Fixed Summary -->
        {#if !isLoadingWholeClip && wholeClipArchives.length > 0}
          <div class="px-6 pb-6">
            <div
              class="p-4 rounded-lg bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800"
            >
              <div class="flex items-center space-x-2 mb-2">
                <FileVideo class="w-4 h-4 text-blue-600 dark:text-blue-400" />
                <span
                  class="text-sm font-medium text-blue-900 dark:text-blue-100"
                  >合成信息</span
                >
              </div>
              <div class="text-sm text-blue-800 dark:text-blue-200">
                共 {wholeClipArchives.length} 个片段 · 总时长 {format_duration(
                  wholeClipArchives.reduce(
                    (sum, archive) => sum + archive.length,
                    0
                  )
                )} · 总大小 {format_size(
                  wholeClipArchives.reduce(
                    (sum, archive) => sum + archive.size,
                    0
                  )
                )}
              </div>
              <div class="text-sm text-gray-500 dark:text-gray-400">
                如果片段分辨率不一致，将会消耗更多时间用于重新编码
              </div>
            </div>
          </div>
        {/if}
      </div>

      <!-- Footer -->
      <div
        class="px-6 py-4 border-t border-gray-200 dark:border-gray-700/50 flex justify-end space-x-3"
      >
        <button
          class="px-4 py-2 text-sm font-medium text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-600 rounded-lg transition-colors"
          on:click={() => (generateWholeClipModal = false)}
        >
          取消
        </button>
        <button
          class="px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          disabled={isLoadingWholeClip || wholeClipArchives.length === 0}
          on:click={generateWholeClip}
        >
          开始合成
        </button>
      </div>
    </div>
  </div>
{/if}

<svelte:window on:mousedown={handleModalClickOutside} />

<style>
  /* macOS style toggle switch */
  .toggle-switch {
    position: relative;
    display: inline-block;
    width: 36px;
    height: 20px;
  }

  .toggle-switch input {
    opacity: 0;
    width: 0;
    height: 0;
  }

  .toggle-slider {
    position: absolute;
    cursor: pointer;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background-color: rgba(120, 120, 128, 0.32);
    transition: 0.2s;
    border-radius: 20px;
  }

  .toggle-slider:before {
    position: absolute;
    content: "";
    height: 16px;
    width: 16px;
    left: 2px;
    bottom: 2px;
    background-color: white;
    transition: 0.2s;
    border-radius: 50%;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
  }

  input:checked + .toggle-slider {
    background-color: #34c759;
  }

  input:checked + .toggle-slider:before {
    transform: translateX(16px);
  }

  @keyframes spin-slow {
    from {
      transform: rotate(0deg);
    }
    to {
      transform: rotate(360deg);
    }
  }

  .animate-spin-slow {
    animation: spin-slow 3s linear infinite;
  }
</style>
