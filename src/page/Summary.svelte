<script lang="ts">
  import { get_static_url, invoke } from "../lib/invoker";
  import type { RecorderList, DiskInfo } from "../lib/interface";
  import type { RecordItem } from "../lib/db";
  const INTERVAL = 5000;
  import { scale } from "svelte/transition";
  import {
    CalendarCheck,
    Clock,
    Database,
    HardDrive,
    Play,
    RefreshCw,
    Trash2,
    Users,
    Video,
  } from "lucide-svelte";

  let summary: RecorderList = {
    count: 0,
    recorders: [],
  };

  let disk_info: DiskInfo = {
    disk: "",
    total: 0,
    free: 0,
  };

  let total = 0;
  let online = 0;
  let disk_usage = 0;
  let account_count = 0;
  let total_length = 0;
  let today_record_count = 0;
  let recent_records: RecordItem[] = [];
  let activeDropdown = null;
  let loading = false;
  let offset = 0;
  let hasMore = true;
  let hasNewRecords = false;
  const RECORDS_PER_PAGE = 5;

  async function update_summary() {
    summary = (await invoke("get_recorder_list")) as RecorderList;
    total = summary.count;
    online = summary.recorders.filter((r) => r.room_info.status).length;

    disk_usage = await get_archive_disk_usage();

    // get disk info
    disk_info = await invoke("get_disk_info");
    account_count = await get_account_count();

    // get total length
    total_length = await get_total_length();

    // get today record count
    today_record_count = await get_today_record_count();

    // check for new records
    if (recent_records.length > 0) {
      const latestRecords = (await invoke("get_recent_record", {
        roomId: "",
        offset: 0,
        limit: 1,
      })) as RecordItem[];

      if (
        latestRecords.length > 0 &&
        (!recent_records[0] ||
          latestRecords[0].live_id !== recent_records[0].live_id)
      ) {
        hasNewRecords = true;
      }
    } else {
      // Initial load
      await loadMoreRecords();
    }
  }

  async function loadMoreRecords() {
    if (loading || (!hasMore && !hasNewRecords)) return;

    loading = true;
    const newRecords = (await invoke("get_recent_record", {
      roomId: "",
      offset: hasNewRecords ? 0 : offset,
      limit: RECORDS_PER_PAGE,
    })) as RecordItem[];

    for (const record of newRecords) {
      record.cover = await get_static_url(
        "cache",
        `${record.platform}/${record.room_id}/${record.live_id}/cover.jpg`
      );
    }

    if (hasNewRecords) {
      recent_records = newRecords;
      offset = newRecords.length;
      hasNewRecords = false;
      hasMore = true;
    } else {
      if (newRecords.length < RECORDS_PER_PAGE) {
        hasMore = false;
      }
      recent_records = [...recent_records, ...newRecords];
      offset += newRecords.length;
    }

    loading = false;
  }

  function handleScroll(event) {
    const target = event.target;
    // If we're at the top and there are new records, load them
    if (target.scrollTop === 0 && hasNewRecords) {
      loadMoreRecords();
      return;
    }

    // Otherwise check if we need to load more old records
    const bottom =
      target.scrollHeight - target.scrollTop - target.clientHeight < 50;
    if (bottom && !hasNewRecords) {
      loadMoreRecords();
    }
  }

  update_summary();
  setInterval(update_summary, INTERVAL);

  async function get_archive_disk_usage() {
    const total_size = (await invoke("get_archive_disk_usage")) as number;
    return total_size;
  }

  async function get_total_length(): Promise<number> {
    return await invoke("get_total_length");
  }

  async function get_today_record_count(): Promise<number> {
    return await invoke("get_today_record_count");
  }

  async function get_account_count(): Promise<number> {
    return await invoke("get_account_count");
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

  function format_time(time: number) {
    time = Math.round(time);
    const hours = Math.floor(time / 3600);
    const minutes = Math.floor((time % 3600) / 60);
    const seconds = time % 60;
    // two digits
    return `${hours.toString().padStart(2, "0")}:${minutes
      .toString()
      .padStart(2, "0")}:${seconds.toString().padStart(2, "0")}`;
  }

  // format date to YYYY-MM-DD HH:MM:SS
  function format_date(date: string) {
    return new Date(date).toLocaleString();
  }

  function toggleDropdown(id) {
    if (activeDropdown === id) {
      activeDropdown = null;
    } else {
      activeDropdown = id;
    }
  }

  function handleClickOutside(event) {
    if (
      activeDropdown !== null &&
      !event.target.closest(".dropdown-container")
    ) {
      activeDropdown = null;
    }
  }

  async function deleteRecord(record: RecordItem) {
    try {
      await invoke("delete_archive", {
        platform: record.platform,
        roomId: record.room_id,
        liveId: record.live_id,
      });

      // Remove the record from the list
      recent_records = recent_records.filter(
        (r) => r.live_id !== record.live_id
      );

      // Update stats
      disk_usage -= record.size;
      total_length -= record.length;
      if (
        new Date(record.created_at).toDateString() === new Date().toDateString()
      ) {
        today_record_count--;
      }
    } catch (error) {
      alert(error);
    }
  }

  async function refreshRecords() {
    // Reset pagination
    offset = 0;
    hasMore = true;
    recent_records = [];
    // Load records from beginning
    await loadMoreRecords();
  }
</script>

<svelte:window on:click={handleClickOutside} />

<div
  class="flex-1 p-6 overflow-y-auto custom-scrollbar-light bg-gray-50 dark:bg-black"
  on:scroll={handleScroll}
>
  <div class="space-y-6">
    <!-- Header -->
    <div class="flex justify-between items-center">
      <h1 class="text-2xl font-semibold text-gray-900 dark:text-white">总览</h1>
    </div>

    <!-- Stats Grid -->
    <div class="grid grid-cols-3 gap-6">
      <!-- Cache Size -->
      <div
        class="p-6 rounded-xl bg-white dark:bg-[#3c3c3e] shadow-sm border border-gray-200 dark:border-gray-700 hover:border-blue-500 dark:hover:border-blue-400 transition-colors"
      >
        <div class="flex items-center space-x-3">
          <div class="p-3 rounded-lg bg-blue-500">
            <HardDrive class="w-6 h-6 icon-white" />
          </div>
          <div>
            <p class="text-sm text-gray-600 dark:text-gray-400">缓存占用</p>
            <p class="text-2xl font-semibold text-gray-900 dark:text-white">
              {format_size(disk_usage)}
            </p>
          </div>
        </div>
      </div>

      <div
        class="p-6 rounded-xl bg-white dark:bg-[#3c3c3e] shadow-sm border border-gray-200 dark:border-gray-700 hover:border-blue-500 dark:hover:border-blue-400 transition-colors"
      >
        <div class="flex items-center space-x-3">
          <div class="p-3 rounded-lg bg-orange-500">
            <Database class="w-6 h-6 icon-white" />
          </div>
          <div class="min-w-0 flex-1">
            <div class="flex items-baseline justify-between">
              <p class="text-sm text-gray-600 dark:text-gray-400">磁盘使用</p>
              <p class="text-xs text-gray-500 dark:text-gray-400">
                {format_size(disk_info.free)}剩余
              </p>
            </div>
            <p class="text-2xl font-semibold text-gray-900 dark:text-white">
              {format_size(disk_info.total - disk_info.free)}
            </p>
            <div
              class="w-full h-1 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden mt-1.5"
            >
              <div
                class="h-full bg-orange-500 rounded-full"
                style="width: {((disk_info.total - disk_info.free) /
                  disk_info.total) *
                  100}%"
              ></div>
            </div>
          </div>
        </div>
      </div>

      <!-- Active Rooms -->
      <div
        class="p-6 rounded-xl bg-white dark:bg-[#3c3c3e] shadow-sm border border-gray-200 dark:border-gray-700 hover:border-blue-500 dark:hover:border-blue-400 transition-colors"
      >
        <div class="flex items-center space-x-3">
          <div class="p-3 rounded-lg bg-green-500">
            <Video class="w-6 h-6 icon-white" />
          </div>
          <div>
            <p class="text-sm text-gray-600 dark:text-gray-400">直播间</p>
            <p class="text-2xl font-semibold text-gray-900 dark:text-white">
              {online} / {total}
            </p>
          </div>
        </div>
      </div>

      <!-- Connected Accounts -->
      <div
        class="p-6 rounded-xl bg-white dark:bg-[#3c3c3e] shadow-sm border border-gray-200 dark:border-gray-700 hover:border-blue-500 dark:hover:border-blue-400 transition-colors"
      >
        <div class="flex items-center space-x-3">
          <div class="p-3 rounded-lg bg-purple-500">
            <Users class="w-6 h-6 icon-white" />
          </div>
          <div>
            <p class="text-sm text-gray-600 dark:text-gray-400">账号</p>
            <p class="text-2xl font-semibold text-gray-900 dark:text-white">
              {account_count}
            </p>
          </div>
        </div>
      </div>

      <!-- Total Recording Time -->
      <div
        class="p-6 rounded-xl bg-white dark:bg-[#3c3c3e] shadow-sm border border-gray-200 dark:border-gray-700 hover:border-blue-500 dark:hover:border-blue-400 transition-colors"
      >
        <div class="flex items-center space-x-3">
          <div class="p-3 rounded-lg bg-indigo-500">
            <Clock class="w-6 h-6 icon-white" />
          </div>
          <div>
            <p class="text-sm text-gray-600 dark:text-gray-400">总缓存时长</p>
            <p class="text-2xl font-semibold text-gray-900 dark:text-white">
              {format_time(total_length)}
            </p>
          </div>
        </div>
      </div>

      <!-- Today's Recordings -->
      <div
        class="p-6 rounded-xl bg-white dark:bg-[#3c3c3e] shadow-sm border border-gray-200 dark:border-gray-700 hover:border-blue-500 dark:hover:border-blue-400 transition-colors"
      >
        <div class="flex items-center space-x-3">
          <div class="p-3 rounded-lg bg-pink-500">
            <CalendarCheck class="w-6 h-6 icon-white" />
          </div>
          <div>
            <p class="text-sm text-gray-600 dark:text-gray-400">
              今日缓存直播数
            </p>
            <p class="text-2xl font-semibold text-gray-900 dark:text-white">
              {today_record_count}
            </p>
          </div>
        </div>
      </div>
    </div>

    <!-- Recent Recordings -->
    <div class="space-y-4">
      <div class="flex justify-between items-center">
        <div class="flex items-center space-x-3">
          <h2 class="text-lg font-semibold text-gray-900 dark:text-white">
            最近的直播记录
          </h2>
          <button
            class="p-1.5 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-700/50 transition-colors text-gray-500 dark:text-gray-400"
            on:click={refreshRecords}
          >
            <RefreshCw class="w-5 h-5 dark:icon-white" />
          </button>
        </div>
        {#if hasNewRecords}
          <button
            class="px-3 py-1 text-sm text-blue-600 dark:text-blue-400 bg-blue-50 dark:bg-blue-500/10 rounded-full hover:bg-blue-100 dark:hover:bg-blue-500/20 transition-colors"
            on:click={loadMoreRecords}
          >
            记录有更新 • 点击刷新
          </button>
        {/if}
      </div>
      <div class="space-y-3">
        <!-- Recording Items -->
        {#each recent_records as record}
          <div
            class="p-4 rounded-lg bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 flex items-center justify-between hover:border-blue-500 dark:hover:border-blue-400 transition-colors"
          >
            <div class="flex items-center space-x-4">
              {#if record.cover}
                <img
                  src={record.cover}
                  class="w-32 h-18 rounded-lg object-cover"
                  alt="Gaming stream thumbnail"
                  on:error={(e) =>
                    console.error("Image error in template:", record.cover, e)}
                />
              {:else}
                <div
                  class="w-32 h-20 rounded-lg bg-gray-200 dark:bg-gray-700 flex items-center justify-center"
                >
                  <Video class="w-8 h-8 text-gray-400 dark:text-gray-500" />
                </div>
              {/if}
              <div>
                <h3 class="font-medium text-gray-900 dark:text-white">
                  {record.title}
                </h3>
                <p class="text-sm text-gray-600 dark:text-gray-400">
                  {record.platform} • {record.room_id} • {format_date(
                    record.created_at
                  )} • {format_size(record.size)}
                </p>
              </div>
            </div>
            <div class="flex items-center space-x-2">
              <button
                class="p-2 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-700"
                on:click={() => {
                  invoke("open_live", {
                    platform: record.platform,
                    roomId: record.room_id,
                    liveId: record.live_id,
                  });
                }}
              >
                <Play class="w-5 h-5 dark:icon-white" />
              </button>
              <div class="relative dropdown-container">
                <button
                  class="p-2 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-700 text-red-600 dark:text-red-400"
                  on:click|stopPropagation={() =>
                    toggleDropdown(record.live_id)}
                >
                  <Trash2 class="w-5 h-5 icon-danger" />
                </button>
                {#if activeDropdown === record.live_id}
                  <div
                    class="absolute right-0 mt-2 w-48 rounded-lg shadow-lg bg-white dark:bg-[#3c3c3e] border border-gray-200 dark:border-gray-700 backdrop-blur-xl bg-opacity-90 dark:bg-opacity-90 z-50"
                    style="transform-origin: top right;"
                    in:scale={{ duration: 100, start: 0.95 }}
                    out:scale={{ duration: 100, start: 0.95 }}
                  >
                    <div
                      class="px-4 py-3 border-b border-gray-200 dark:border-gray-700"
                    >
                      <h3
                        class="text-sm font-medium text-gray-900 dark:text-white"
                      >
                        确认删除
                      </h3>
                      <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">
                        此操作无法撤销
                      </p>
                    </div>
                    <div class="p-2 flex space-x-2">
                      <button
                        class="flex-1 px-3 py-1.5 text-sm text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700/50 rounded-md transition-colors"
                        on:click={() => {
                          activeDropdown = null;
                        }}
                      >
                        取消
                      </button>
                      <button
                        class="flex-1 px-3 py-1.5 text-sm text-white bg-red-600 hover:bg-red-700 rounded-md transition-colors"
                        on:click={() => {
                          deleteRecord(record);
                          activeDropdown = null;
                        }}
                      >
                        删除
                      </button>
                    </div>
                  </div>
                {/if}
              </div>
            </div>
          </div>
        {/each}

        {#if loading}
          <div class="flex justify-center py-4">
            <div
              class="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"
            ></div>
          </div>
        {/if}

        {#if !hasMore && recent_records.length > 0}
          <div class="text-center py-4 text-gray-500 dark:text-gray-400">
            没有更多记录了
          </div>
        {/if}
      </div>
    </div>
  </div>
</div>
