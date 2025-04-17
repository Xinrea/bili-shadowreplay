<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { message } from "@tauri-apps/plugin-dialog";
  import { fade, scale } from "svelte/transition";
  import { Dropdown, DropdownItem } from "flowbite-svelte";
  import { open } from "@tauri-apps/plugin-shell";
  import type { RecorderList, RecorderInfo } from "../lib/interface";
  import Image from "../lib/Image.svelte";
  import type { RecordItem } from "../lib/db";
  import {
    Ellipsis,
    Play,
    Plus,
    Scissors,
    Search,
    Trash2,
    X,
    History,
    Activity,
  } from "lucide-svelte";
  import BilibiliIcon from "../lib/BilibiliIcon.svelte";
  import DouyinIcon from "../lib/DouyinIcon.svelte";
  import AutoRecordIcon from "../lib/AutoRecordIcon.svelte";

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

    summary = new_summary;
  }
  update_summary();
  setInterval(update_summary, 1000);

  function format_time(time: number) {
    let hours = Math.floor(time / 3600);
    let minutes = Math.floor((time % 3600) / 60);
    let seconds = Math.floor(time % 60);
    return `${hours.toString().padStart(2, "0")}:${minutes.toString().padStart(2, "0")}:${seconds.toString().padStart(2, "0")}`;
  }

  // modals
  let deleteModal = false;
  let deleteRoom = null;

  let addModal = false;
  let addRoom = "";
  let addValid = false;
  let addErrorMsg = "";
  let selectedPlatform = "bilibili";

  let archiveModal = false;
  let archiveRoom = null;
  let archives: RecordItem[] = [];
  async function showArchives(room_id: number) {
    archives = await invoke("get_archives", { roomId: room_id });
    // sort archives by ts in descending order
    archives.sort((a, b) => {
      return (
        new Date(b.created_at).getTime() - new Date(a.created_at).getTime()
      );
    });
    archiveModal = true;
    console.log(archives);
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
    const modal = document.querySelector(".mac-modal");
    if (
      modal &&
      !modal.contains(event.target) &&
      !event.target.closest("button")
    ) {
      addModal = false;
      archiveModal = false;
    }
  }

  // Add toggle state for auto-recording
  let autoRecordStates = new Map<string, boolean>();

  // Function to toggle auto-record state
  function toggleAutoRecord(room: RecorderInfo) {
    invoke("set_auto_start", {
      roomId: room.room_id,
      platform: room.platform,
      autoStart: !room.auto_start,
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
</script>

<div class="flex-1 p-6 overflow-auto">
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
            <Image
              src={room.room_info.room_cover}
              iclass={"w-full h-40 object-cover rounded-lg " +
                (room.live_status ? "" : "brightness-75")}
            />
            <div
              class={"absolute top-2 left-2 p-1.5 px-2 rounded-full text-white text-xs flex items-center justify-center " +
                (room.is_recording ? "bg-red-500" : "bg-gray-700/90")}
            >
              {#if room.auto_start}
                <AutoRecordIcon class="w-4 h-4 text-white" />
              {:else}
                <Activity class="w-4 h-4 text-white" />
              {/if}
              {#if room.is_recording}
                <span class="text-white ml-1">录制中</span>
              {/if}
            </div>
            {#if !room.live_status}
              <div
                class={"absolute bottom-2 right-2 p-1.5 px-2 rounded-full text-white text-xs flex items-center justify-center bg-gray-700"}
              >
                <span>直播未开始</span>
              </div>
            {:else}
              <div
                class={"absolute bottom-2 right-2 p-1.5 px-2 rounded-full text-white text-xs flex items-center justify-center bg-green-500"}
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
              {#if room.is_recording}
                <DropdownItem
                  on:click={() => {
                    invoke("force_stop", {
                      platform: room.platform,
                      roomId: room.room_id,
                    });
                  }}>暂停本次录制</DropdownItem
                >
              {/if}
              {#if !room.is_recording && room.live_status}
                <DropdownItem
                  on:click={() => {
                    invoke("force_start", {
                      platform: room.platform,
                      roomId: room.room_id,
                    });
                  }}>开始录制</DropdownItem
                >
              {/if}
              <button
                class="px-4 py-2 flex items-center justify-between hover:bg-gray-100 dark:hover:bg-gray-700 cursor-pointer"
                on:click={() => toggleAutoRecord(room)}
              >
                <span
                  class="text-sm text-gray-700 dark:text-gray-200 font-medium"
                  >自动开始录制</span
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
                  <Image
                    src={room.user_info.user_avatar}
                    iclass="w-8 h-8 rounded-full"
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
      class="mac-modal w-[320px] bg-white dark:bg-[#323234] rounded-xl shadow-xl overflow-hidden"
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
            class="w-24 px-4 py-2 text-sm font-medium text-gray-700 dark:text-gray-300 hover:bg-[#f5f5f7] dark:hover:bg-[#3a3a3c] rounded-lg transition-colors"
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
      class="mac-modal w-[400px] bg-white dark:bg-[#323234] rounded-xl shadow-xl overflow-hidden"
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
              class="px-4 py-2 text-sm font-medium text-gray-700 dark:text-gray-300 hover:bg-[#f5f5f7] dark:hover:bg-[#3a3a3c] rounded-lg transition-colors"
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
                invoke("add_recorder", {
                  roomId: Number(addRoom),
                  platform: selectedPlatform,
                })
                  .then(() => {
                    addModal = false;
                    addRoom = "";
                  })
                  .catch(async (e) => {
                    await message(e);
                  });
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
      class="mac-modal w-[900px] bg-white dark:bg-[#323234] rounded-xl shadow-xl overflow-hidden flex flex-col max-h-[80vh]"
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

      <div class="flex-1 overflow-auto">
        <div class="p-6">
          <div class="overflow-x-auto">
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
                {#each archives as archive}
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
                          <Image
                            src={archive.cover}
                            iclass="w-12 h-8 rounded object-cover"
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
                      <div
                        class="flex items-center space-x-2 opacity-0 group-hover:opacity-100 transition-opacity"
                      >
                        <button
                          class="p-1.5 rounded-lg hover:bg-blue-500/10 transition-colors"
                          title="编辑切片"
                          on:click={() => {
                            invoke("open_live", {
                              platform: archiveRoom.platform,
                              roomId: archiveRoom.room_id,
                              liveId: archive.live_id,
                            });
                          }}
                        >
                          <Scissors class="w-4 h-4 icon-primary" />
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
                                archives = await invoke("get_archives", {
                                  roomId: archiveRoom.room_id,
                                });
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
        </div>
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
