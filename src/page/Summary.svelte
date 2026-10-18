<script lang="ts">
  import { createStateRefresh, watchStateChanges } from "../lib/state-refresh";

  import { get_static_url, invoke } from "../lib/invoker";
  import type { RecorderList, DiskInfo } from "../lib/interface";
  import type { RecordItem } from "../lib/db";
  import { onMount } from "svelte";
  import PageTitlebar from "../lib/components/PageTitlebar.svelte";
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

  let disk_info: DiskInfo = $state({
    disk: "",
    total: 0,
    free: 0,
  });

  let total = $state(0);
  let online = $state(0);
  let disk_usage = $state(0);
  let account_count = $state(0);
  let total_length = $state(0);
  let today_record_count = $state(0);
  let recent_records: RecordItem[] = $state([]);
  let activeDropdown = $state(null);
  let loading = $state(false);
  let offset = 0;
  let hasMore = $state(true);
  let hasNewRecords = $state(false);
  const RECORDS_PER_PAGE = 5;

  const stateRefresh = createStateRefresh(update_summary);

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
        `${record.platform}/${record.room_id}/${record.live_id}/cover.jpg`,
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

  onMount(() => watchStateChanges(stateRefresh, ["recorders-changed", "records-changed"]));

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
        (r) => r.live_id !== record.live_id,
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

<svelte:window onclick={handleClickOutside} />

<PageTitlebar title="总览" />

<div
  class="flex-1 overflow-y-auto bg-canvas p-6 custom-scrollbar-light"
  onscroll={handleScroll}
>
  <div class="space-y-6">
    <!-- Stats Grid -->
    <div class="grid grid-cols-3 gap-4">
      <!-- Cache Size -->
      <div class="ui-card p-4 transition-colors hover:border-line-strong">
        <div class="flex items-center gap-3">
          <div
            class="flex h-10 w-10 shrink-0 items-center justify-center rounded-[9px] bg-accent-soft text-accent-fg"
          >
            <HardDrive class="h-5 w-5" />
          </div>
          <div class="min-w-0">
            <p class="text-[12px] text-fg-muted">缓存占用</p>
            <p class="text-[20px] font-semibold leading-tight text-fg tabular-nums">
              {format_size(disk_usage)}
            </p>
          </div>
        </div>
      </div>

      <div class="ui-card p-4 transition-colors hover:border-line-strong">
        <div class="flex items-center gap-3">
          <div
            class="flex h-10 w-10 shrink-0 items-center justify-center rounded-[9px] bg-warning-soft text-warning-fg"
          >
            <Database class="h-5 w-5" />
          </div>
          <div class="min-w-0 flex-1">
            <div class="flex items-baseline justify-between gap-2">
              <p class="text-[12px] text-fg-muted">磁盘使用</p>
              <p class="text-[11px] text-fg-faint tabular-nums">
                {format_size(disk_info.free)}剩余
              </p>
            </div>
            <p class="text-[20px] font-semibold leading-tight text-fg tabular-nums">
              {format_size(disk_info.total - disk_info.free)}
            </p>
            <div class="mt-1.5 h-1 w-full overflow-hidden rounded-full bg-sunken">
              <div
                class="h-full rounded-full bg-warning"
                style="width: {((disk_info.total - disk_info.free) /
                  disk_info.total) *
                  100}%"
              ></div>
            </div>
          </div>
        </div>
      </div>

      <!-- Active Rooms -->
      <div class="ui-card p-4 transition-colors hover:border-line-strong">
        <div class="flex items-center gap-3">
          <div
            class="flex h-10 w-10 shrink-0 items-center justify-center rounded-[9px] bg-success-soft text-success-fg"
          >
            <Video class="h-5 w-5" />
          </div>
          <div class="min-w-0">
            <p class="text-[12px] text-fg-muted">直播间</p>
            <p class="text-[20px] font-semibold leading-tight text-fg tabular-nums">
              {online} / {total}
            </p>
          </div>
        </div>
      </div>

      <!-- Connected Accounts -->
      <div class="ui-card p-4 transition-colors hover:border-line-strong">
        <div class="flex items-center gap-3">
          <div
            class="flex h-10 w-10 shrink-0 items-center justify-center rounded-[9px] bg-violet-500/15 text-violet-500"
          >
            <Users class="h-5 w-5" />
          </div>
          <div class="min-w-0">
            <p class="text-[12px] text-fg-muted">账号</p>
            <p class="text-[20px] font-semibold leading-tight text-fg tabular-nums">
              {account_count}
            </p>
          </div>
        </div>
      </div>

      <!-- Total Recording Time -->
      <div class="ui-card p-4 transition-colors hover:border-line-strong">
        <div class="flex items-center gap-3">
          <div
            class="flex h-10 w-10 shrink-0 items-center justify-center rounded-[9px] bg-indigo-500/15 text-indigo-500"
          >
            <Clock class="h-5 w-5" />
          </div>
          <div class="min-w-0">
            <p class="text-[12px] text-fg-muted">总缓存时长</p>
            <p class="text-[20px] font-semibold leading-tight text-fg tabular-nums">
              {format_time(total_length)}
            </p>
          </div>
        </div>
      </div>

      <!-- Today's Recordings -->
      <div class="ui-card p-4 transition-colors hover:border-line-strong">
        <div class="flex items-center gap-3">
          <div
            class="flex h-10 w-10 shrink-0 items-center justify-center rounded-[9px] bg-pink-500/15 text-pink-500"
          >
            <CalendarCheck class="h-5 w-5" />
          </div>
          <div class="min-w-0">
            <p class="text-[12px] text-fg-muted">
              今日缓存直播数
            </p>
            <p class="text-[20px] font-semibold leading-tight text-fg tabular-nums">
              {today_record_count}
            </p>
          </div>
        </div>
      </div>
    </div>

    <!-- Recent Recordings -->
    <div class="space-y-3">
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-2">
          <h2 class="ui-section-title">
            最近的直播记录
          </h2>
          <button
            class="ui-btn ui-btn-ghost ui-btn-sm ui-btn-icon"
            onclick={refreshRecords}
          >
            <RefreshCw class="h-3.5 w-3.5" />
          </button>
        </div>
        {#if hasNewRecords}
          <button
            class="inline-flex h-6 items-center rounded-full border border-accent/30 bg-accent-soft px-2.5 text-[11px] font-medium text-accent-fg transition-colors hover:border-accent/60"
            onclick={loadMoreRecords}
          >
            记录有更新 • 点击刷新
          </button>
        {/if}
      </div>
      <div class="space-y-2">
        <!-- Recording Items -->
        {#each recent_records as record}
          <div
            class="ui-card flex items-center justify-between gap-3 p-3 transition-colors hover:border-line-strong"
          >
            <div class="flex min-w-0 items-center gap-3">
              {#if record.cover}
                <img
                  src={record.cover}
                  class="h-[72px] w-32 shrink-0 rounded-[8px] border border-line object-cover"
                  alt="Gaming stream thumbnail"
                  onerror={(e) =>
                    console.error("Image error in template:", record.cover, e)}
                />
              {:else}
                <div
                  class="flex h-[72px] w-32 shrink-0 items-center justify-center rounded-[8px] bg-sunken"
                >
                  <Video class="h-7 w-7 text-fg-faint" />
                </div>
              {/if}
              <div class="min-w-0">
                <h3 class="text-[13px] font-medium text-fg">
                  {record.title}
                </h3>
                <p class="mt-1 text-[11px] text-fg-muted">
                  {record.platform} • {record.room_id} • {format_date(
                    record.created_at,
                  )} • {format_size(record.size)}
                </p>
              </div>
            </div>
            <div class="flex shrink-0 items-center gap-1">
              <button
                class="ui-btn ui-btn-ghost ui-btn-icon"
                onclick={() => {
                  invoke("open_live", {
                    platform: record.platform,
                    roomId: record.room_id,
                    liveId: record.live_id,
                  });
                }}
              >
                <Play class="h-4 w-4" />
              </button>
              <div class="relative dropdown-container">
                <button
                  class="ui-btn ui-btn-ghost ui-btn-icon"
                  onclick={(event) => {
                    event.stopPropagation();
                    toggleDropdown(record.live_id);
                  }}
                >
                  <Trash2 class="h-4 w-4 icon-danger" />
                </button>
                {#if activeDropdown === record.live_id}
                  <div
                    class="ui-menu absolute right-0 z-50 mt-1 w-48"
                    style="transform-origin: top right;"
                    in:scale={{ duration: 100, start: 0.95 }}
                    out:scale={{ duration: 100, start: 0.95 }}
                  >
                    <div class="border-b border-line px-2 pb-2 pt-1.5">
                      <h3 class="text-[12px] font-medium text-fg">
                        确认删除
                      </h3>
                      <p class="ui-hint mt-0.5">
                        此操作无法撤销
                      </p>
                    </div>
                    <div class="flex gap-1.5 p-1 pt-1.5">
                      <button
                        class="ui-btn ui-btn-sm flex-1"
                        onclick={() => {
                          activeDropdown = null;
                        }}
                      >
                        取消
                      </button>
                      <button
                        class="ui-btn ui-btn-sm ui-btn-danger flex-1"
                        onclick={() => {
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
              class="h-6 w-6 animate-spin rounded-full border-b-2 border-accent"
            ></div>
          </div>
        {/if}

        {#if !hasMore && recent_records.length > 0}
          <div class="py-4 text-center text-[12px] text-fg-faint">
            没有更多记录了
          </div>
        {/if}
      </div>
    </div>
  </div>
</div>
