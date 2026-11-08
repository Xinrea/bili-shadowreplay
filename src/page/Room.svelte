<script lang="ts">
  import { createStateRefresh, watchStateChanges } from "../lib/state-refresh";
  import { liveRoomUrl } from "../lib/platform";
  import { invoke, open, onOpenUrl, get } from "../lib/invoker";
  import { fade, scale } from "svelte/transition";
  import { Dropdown, DropdownItem } from "flowbite-svelte";
  import type { Config, RecorderList, RecorderInfo, RecorderRow, StorageConfig } from "../lib/interface";
  import {
    Ellipsis,
    Play,
    Plus,
    Search,
    Trash2,
    X,
    History,
    PlayIcon,
    Globe,
    Radio,
  } from "lucide-svelte";
  import BilibiliIcon from "../lib/components/BilibiliIcon.svelte";
  import DouyinIcon from "../lib/components/DouyinIcon.svelte";
  import DouyuIcon from "../lib/components/DouyuIcon.svelte";
  import KuaishouIcon from "../lib/components/KuaishouIcon.svelte";
  import HuyaIcon from "../lib/components/HuyaIcon.svelte";
  import TikTokIcon from "../lib/components/TikTokIcon.svelte";
  import TwitchIcon from "../lib/components/TwitchIcon.svelte";
  import YouTubeIcon from "../lib/components/YouTubeIcon.svelte";
  import PlatformIconTabs from "../lib/components/PlatformIconTabs.svelte";
  import AutoRecordIcon from "../lib/components/AutoRecordIcon.svelte";
  import PageTitlebar from "../lib/components/PageTitlebar.svelte";
  import DeleteConfirmActions from "../lib/components/DeleteConfirmActions.svelte";
  import RoomStatusModal from "../lib/components/RoomStatusModal.svelte";
  import RoomArchiveModal from "../lib/components/RoomArchiveModal.svelte";
  import StorageBackendSelect from "../lib/components/StorageBackendSelect.svelte";
  import { storageBackend } from "../lib/recording-storage";
  import { onMount } from "svelte";
  import { registerCommands } from "../lib/stores/commands";

  interface Props {
    room_count?: number;
  }

  let { room_count = $bindable(0) }: Props = $props();
  let room_active = $state(0);
  let room_inactive = $state(0);

  let summary: RecorderList = $state({
    count: 0,
    recorders: [],
  });

  let searchQuery = $state("");
  let filteredRecorders = $derived(summary.recorders.filter((room) => {
    const query = searchQuery.toLowerCase();
    return (
      room.room_info.room_title.toLowerCase().includes(query) ||
      room.user_info.user_name.toLowerCase().includes(query) ||
      room.room_info.room_id.toString().includes(query)
    );
  }));

  function default_avatar(platform: string) {
    const avatarMap = {
      bilibili: "/imgs/bilibili_avatar.png",
      douyin: "/imgs/douyin.png",
      douyu: "/imgs/douyu.svg",
      huya: "/imgs/huya_avatar.png",
      kuaishou: "/imgs/kuaishou.svg",
      tiktok: "/imgs/tiktok.png",
      twitch: "/imgs/twitch.svg",
      youtube: "/imgs/youtube.svg",
      rtmp: "/imgs/rtmp.svg",
    };
    return avatarMap[platform] || "/imgs/huya_avatar.png";
  }

  function default_cover(platform: string) {
    const coverMap = {
      bilibili: "/imgs/bilibili.png",
      douyin: "/imgs/douyin.png",
      douyu: "/imgs/douyu.svg",
      huya: "/imgs/huya.png",
      kuaishou: "/imgs/kuaishou.svg",
      tiktok: "/imgs/tiktok.png",
      twitch: "/imgs/twitch.svg",
      youtube: "/imgs/youtube.svg",
      rtmp: "/imgs/rtmp.svg",
    };
    return coverMap[platform] || "/imgs/huya.png";
  }

  let avatar_cache: Map<string, string> = new Map();
  async function get_avatar_url(user_id: string, url: string) {
    if (avatar_cache.has(user_id)) {
      return avatar_cache.get(user_id);
    }
    console.log("get avatar url:", url);
    const response = await get(url);
    const blob = await response.blob();
    const avatar_url = URL.createObjectURL(blob);
    avatar_cache.set(user_id, avatar_url);
    return avatar_url;
  }

  let image_cache: Map<string, string> = new Map();
  async function get_image_url(url: string) {
    if (image_cache.has(url)) {
      return image_cache.get(url);
    }
    console.log("get image url:", url);
    const response = await get(url);
    const blob = await response.blob();
    const cover_url = URL.createObjectURL(blob);
    image_cache.set(url, cover_url);
    return cover_url;
  }

  const stateRefresh = createStateRefresh(update_summary);

  async function update_summary() {
    let new_summary = (await invoke("get_recorder_list")) as RecorderList;
    room_count = new_summary.count;
    room_active = new_summary.recorders.filter(
      (room) => room.room_info.status,
    ).length;
    room_inactive = new_summary.recorders.filter(
      (room) => !room.room_info.status,
    ).length;

    // sort new_summary.recorders by live_status
    new_summary.recorders.sort((a, b) => {
      if (a.room_info.status && !b.room_info.status) return -1;
      if (!a.room_info.status && b.room_info.status) return 1;
      return 0;
    });

    // process room cover
    for (const room of new_summary.recorders) {
      if (room.user_info.user_avatar != "") {
        room.user_info.user_avatar = await get_avatar_url(
          room.user_info.user_id,
          room.user_info.user_avatar,
        );
      } else {
        room.user_info.user_avatar = default_avatar(room.room_info.platform);
      }

      if (room.room_info.room_cover != "") {
        room.room_info.room_cover = await get_image_url(
          room.room_info.room_cover,
        );
      } else if (room.user_info.user_avatar != "") {
        room.room_info.room_cover = room.user_info.user_avatar;
      } else {
        room.room_info.room_cover = default_cover(room.room_info.platform);
      }
    }

    summary = new_summary;
  }
  onMount(() => watchStateChanges(stateRefresh, ["recorders-changed"]));

  let searchInput: HTMLInputElement | null = $state(null);

  onMount(() =>
    registerCommands([
      {
        id: "room:add",
        group: "action",
        title: "添加直播间",
        detail: "直播间",
        icon: Plus,
        keywords: ["add room", "new room", "record"],
        page: "直播间",
        run: () => (addModal = true),
      },
      {
        id: "room:search",
        group: "action",
        title: "搜索直播间",
        detail: "直播间",
        icon: Search,
        keywords: ["search room", "find", "filter"],
        page: "直播间",
        run: () => searchInput?.focus({ preventScroll: true }),
      },
    ]),
  );

  // modals
  let statusRoom: RecorderInfo | null = $state(null);
  let roomMenuTrigger: HTMLButtonElement | null = null;

  function closeRoomStatus() {
    statusRoom = null;
    queueMicrotask(() => roomMenuTrigger?.focus({ preventScroll: true }));
  }

  let deleteModal = $state(false);
  let deletingRoom = $state(false);
  let deleteRoom = $state(null);

  let addModal = $state(false);
  let addRoom = $state("");
  let selectedPlatform = $state("bilibili");
  let creatingRoom = $state(false);
  let addStorage: StorageConfig | null = $state(null);
  let addStorageId = $state<string | null>(null);
  let storageLoading = $state(false);
  let storageError = $state("");
  let addError = $state("");
  let storageLoadRequest = 0;

  async function loadAddStorage() {
    const request = ++storageLoadRequest;
    const isCurrent = () => request === storageLoadRequest && addModal;
    storageLoading = true;
    storageError = "";
    try {
      const config = await invoke<Config>("get_config");
      if (isCurrent()) addStorage = config.storage;
    } catch (error) {
      if (isCurrent()) storageError = `录制存储加载失败：${String(error)}`;
    } finally {
      if (isCurrent()) storageLoading = false;
    }
  }

  $effect(() => {
    if (!addModal) return;
    addStorage = null;
    addStorageId = null;
    addError = "";
    void loadAddStorage();
    return () => { ++storageLoadRequest; };
  });

  let archiveRoom: RecorderInfo | null = $state(null);
  const currentArchiveRoom = $derived(archiveRoom && (summary.recorders.find((room) =>
    room.room_info.platform === archiveRoom.room_info.platform &&
    room.room_info.room_id === archiveRoom.room_info.room_id) ?? archiveRoom));

  function handleModalClickOutside(event) {
    if (statusRoom || archiveRoom) return;
    // 检查点击是否在任何modal内部
    const clickedElement = event.target;

    // 检查是否点击了按钮，如果是则不关闭modal
    if (clickedElement.closest("button")) {
      return;
    }

    // 添加直播间弹窗
    if (addModal) {
      if (creatingRoom) return;
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

    // 删除直播间弹窗
    if (deleteModal) {
      if (deletingRoom) return;
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
      roomId: room.room_info.room_id,
      platform: room.room_info.platform,
      enabled: !room.enabled,
    });
  }

  function openUserUrl(room: RecorderInfo) {
    if (room.room_info.platform === "bilibili") {
      open("https://space.bilibili.com/" + room.user_info.user_id);
    } else if (room.room_info.platform === "douyin") {
      console.log(room.user_info);
      open("https://www.douyin.com/user/" + room.user_info.user_id);
    } else if (room.room_info.platform === "douyu") {
      openLiveUrl(room);
    } else if (room.room_info.platform === "kuaishou") {
      if (room.user_info.user_id) {
        open("https://www.kuaishou.com/profile/" + room.user_info.user_id);
      } else {
        openLiveUrl(room);
      }
    } else if (room.room_info.platform === "tiktok") {
      if (room.user_info.user_id) {
        const handle = room.user_info.user_id.startsWith("@")
          ? room.user_info.user_id
          : `@${room.user_info.user_id}`;
        open(`https://www.tiktok.com/${handle}`);
      } else {
        openLiveUrl(room);
      }
    } else if (room.room_info.platform === "twitch") {
      openLiveUrl(room);
    } else if (room.room_info.platform === "youtube") {
      if (room.user_info.user_id) {
        open(`https://www.youtube.com/channel/${room.user_info.user_id}`);
      } else {
        openLiveUrl(room);
      }
    }
  }

  function openLiveUrl(room: RecorderInfo) {
    const url = liveRoomUrl(room.room_info.platform, room.room_info.room_id);
    if (url) open(url);
  }

  function addNewRecorder(room_id: string, platform: string) {
    if (creatingRoom || storageLoading || !addStorage || storageError) return;
    addError = "";
    const storageId = addStorageId === null ? addStorage.default_backend : addStorageId;
    if (storageBackend(addStorage, storageId)?.type !== "local") {
      addError = "请选择已配置的本地录制存储。不会自动改用其他存储。";
      return;
    }
    const room_input = room_id.trim();
    const douyu_room_id =
      platform === "douyu"
        ? new URLSearchParams(room_input.split("?")[1]?.split("#")[0] || "").get("rid")
        : null;
    room_id = platform === "youtube" ? room_input : room_input.split(/[?#]/)[0];
    if (douyu_room_id && /^\d+$/.test(douyu_room_id)) {
      room_id = douyu_room_id;
    }
    switch (platform) {
      case "bilibili":
        // room_id might be a link, extract the room_id from the link
        // example: https://live.bilibili.com/1234567890
        if (room_id.includes("https://") || room_id.includes("bsr://")) {
          room_id = room_id.split("/").pop();
        }
        break;
      case "douyin":
        // room_id might be a link, extract the room_id from the link
        // example: https://live.douyin.com/1234567890
        if (room_id.includes("https://") || room_id.includes("bsr://")) {
          room_id = room_id.split("/").pop();
        }
        break;
      case "douyu":
        // Douyu accepts numeric room IDs and vanity paths.
        // Keep only the path segment so the backend can resolve a vanity path.
        if (room_id.includes("://")) {
          room_id = room_id.split(/[?#]/)[0].replace(/\/+$/, "").split("/").pop() || "";
        }
        break;
      case "huya":
        // room_id might be a link, extract the room_id from the link
        // example: https://www.huya.com/1234567890
        if (room_id.includes("https://") || room_id.includes("bsr://")) {
          room_id = room_id.split("/").pop();
        }
        break;
      case "kuaishou":
        // room_id might be a link, extract the room_id from the link
        // example: https://live.kuaishou.com/u/1234567890
        if (room_id.includes("https://") || room_id.includes("bsr://")) {
          room_id = room_id.split("/").pop();
        }
        break;
      case "tiktok":
        // room_id might be a link, extract the room_id from the link
        // example: https://www.tiktok.com/@1234567890/live -> @1234567890
        if (room_id.includes("https://") || room_id.includes("bsr://")) {
          room_id = "@" + room_id.split("@").pop().split("/")[0];
        }
        break;
      case "twitch":
        // Twitch accepts a channel login or https://www.twitch.tv/<login>.
        if (room_id.includes("https://") || room_id.includes("bsr://")) {
          room_id = room_id
            .replace(/^(?:https?:|bsr:)\/\/(?:(?:www|m)\.)?twitch\.tv\//, "")
            .split("/")[0];
        }
        room_id = room_id.replace(/^@/, "");
        break;
      case "youtube":
        // Keep the URL intact. The recorder resolves a channel to its current
        // live video on each poll, while a watch URL pins one broadcast.
        if (room_id.startsWith("bsr://")) {
          room_id = room_id.replace("bsr://", "https://");
        }
        break;
    }

    creatingRoom = true;
    invoke<RecorderRow>("add_recorder", {
      roomId: platform === "rtmp" ? "" : room_id,
      platform: platform,
      extra: platform === "rtmp" ? JSON.stringify({ name: room_input }) : "",
      storageBackendId: addStorageId,
    })
      .then(() => {
        addModal = false;
        addRoom = "";
      })
      .catch((e) => {
        addError = `添加直播间失败：${String(e)}`;
      })
      .finally(() => { creatingRoom = false; });
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

        if (url.startsWith("bsr://live.bilibili.com/")) {
          platform = "bilibili";
        }

        if (url.startsWith("bsr://live.douyin.com/")) {
          platform = "douyin";
        }

        if (url.startsWith("bsr://www.douyu.com/") || url.startsWith("bsr://live.douyu.com/")) {
          platform = "douyu";
        }

        if (url.startsWith("bsr://live.kuaishou.com/")) {
          platform = "kuaishou";
        }

        if (url.startsWith("bsr://live.tiktok.com/")) {
          platform = "tiktok";
        }

        if (
          url.startsWith("bsr://www.twitch.tv/") ||
          url.startsWith("bsr://m.twitch.tv/") ||
          url.startsWith("bsr://twitch.tv/")
        ) {
          platform = "twitch";
        }

        if (
          url.startsWith("bsr://www.youtube.com/") ||
          url.startsWith("bsr://youtube.com/") ||
          url.startsWith("bsr://youtu.be/")
        ) {
          platform = "youtube";
        }

        if (url && platform) {
          addModal = true;
          addRoom = url;
          selectedPlatform = platform;
        }
      }
    });
  });
</script>

<PageTitlebar title="直播间">
  {#snippet subtitle()}
    <span
      class="flex items-center gap-1.5 whitespace-nowrap text-[12px] text-fg-muted"
    >
      <span class="h-1.5 w-1.5 rounded-full bg-success"></span>
      {room_active} 直播中
      <span class="text-fg-faint">·</span>
      {room_inactive} 未直播
    </span>
  {/snippet}
  <div class="relative">
    <Search
      class="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-fg-faint"
    />
    <input
      bind:this={searchInput}
      type="text"
      bind:value={searchQuery}
      placeholder="搜索直播间..."
      class="ui-input h-7 w-44 rounded-[7px] pl-7 pr-2"
    />
  </div>
  <button
    class="ui-btn ui-btn-primary h-7 gap-1 rounded-[7px] px-2.5"
    onclick={() => {
      addModal = true;
    }}
  >
    <Plus class="h-3.5 w-3.5" />
    添加直播间
  </button>
</PageTitlebar>

<div class="custom-scrollbar-light flex-1 overflow-auto bg-canvas p-5">
  <div class="space-y-6">
    <!-- Room Grid -->
    <div class="grid grid-cols-3 gap-4">
      <!-- Active Room Card -->
      {#each filteredRecorders as room (`${room.room_info.platform}:${room.room_info.room_id}`)}
        <div class="ui-card p-3 transition-colors hover:border-line-strong">
          <div class="relative">
            <img
              src={room.room_info.room_cover}
              alt="cover"
              class={"h-40 w-full rounded-[8px] bg-sunken object-cover " +
                (room.room_info.status ? "" : "brightness-75")}
            />
            <!-- Room ID watermark -->
            <div
              class="absolute bottom-2 left-2 flex h-6 max-w-[55%] items-center rounded-[6px] bg-black/50 px-1.5 backdrop-blur-sm"
            >
              <span class="truncate font-mono text-[11px] text-white/85"
                >{room.room_info.platform.toUpperCase()}#{room.room_info
                  .room_id}</span
              >
            </div>
            {#if room.enabled || room.room_info.platform === "rtmp"}
              <div
                class={"absolute left-2 top-2 flex h-6 items-center justify-center gap-1 rounded-[6px] px-1.5 text-[11px] font-medium text-white " +
                  (room.recording ? "bg-danger" : "bg-black/50 backdrop-blur-sm")}
              >
                <AutoRecordIcon class="h-4 w-4 text-white" />
                {#if room.recording}
                  <span class="text-white">{room.room_info.platform === "rtmp" ? "正在录制" : "录制中"}</span>
                {:else if room.room_info.platform === "rtmp"}
                  <span class="text-white">未录制</span>
                {/if}
              </div>
            {/if}
            {#if !room.room_info.status}
              <div
                class="absolute bottom-2 right-2 flex h-6 items-center justify-center rounded-[6px] bg-black/50 px-2 text-[11px] text-white/80 backdrop-blur-sm"
              >
                <span>{room.room_info.platform === "rtmp" ? "未开播" : "直播未开始"}</span>
              </div>
            {:else}
              <div
                class="absolute bottom-2 right-2 flex h-6 items-center justify-center gap-1.5 rounded-[6px] bg-black/50 px-2 text-[11px] font-medium text-white backdrop-blur-sm"
              >
                <span class="h-1.5 w-1.5 rounded-full bg-success"></span>
                <span>{room.room_info.platform === "rtmp" ? "已开播" : "直播进行中"}</span>
              </div>
            {/if}
            <button
              class="absolute right-2 top-2 flex h-7 w-7 items-center justify-center rounded-[7px] bg-black/50 text-white backdrop-blur-sm transition-colors hover:bg-black/70"
              aria-label={`管理直播间 ${room.user_info.user_name || room.room_info.room_id}`}
              onfocus={(event) => (roomMenuTrigger = event.currentTarget)}
              onpointerdown={(event) => (roomMenuTrigger = event.currentTarget)}
            >
              <Ellipsis class="h-4 w-4" />
            </button>
            <Dropdown
              class="min-w-[176px] whitespace-nowrap p-1"
              classContainer="rounded-[10px] border border-line dark:border-line bg-surface dark:bg-surface text-fg dark:text-fg divide-line dark:divide-line shadow-xl"
            >
              <button
                class="ui-menu-item cursor-pointer justify-between gap-4"
                onclick={() => toggleEnabled(room)}
              >
                <span class="font-medium">启用</span>
                <label class="ui-switch">
                  <input type="checkbox" checked={room.enabled} />
                  <span></span>
                </label>
              </button>
              <DropdownItem
                defaultClass="ui-menu-item"
                onclick={() => (statusRoom = room)}>状态</DropdownItem
              >
              {#if room.room_info.platform !== "rtmp"}
                <DropdownItem defaultClass="ui-menu-item" onclick={() => openLiveUrl(room)}>打开网页直播间</DropdownItem>
              {/if}
              <DropdownItem
                defaultClass="ui-menu-item danger"
                onclick={() => {
                  deleteRoom = room;
                  deleteModal = true;
                }}>移除</DropdownItem
              >
            </Dropdown>
          </div>
          <div class="mt-3 space-y-2">
            <div class="flex items-start justify-between">
              <div>
                <div class="flex items-center space-x-2">
                  <span class="shrink-0">
                    {#if room.room_info.platform === "bilibili"}
                      <BilibiliIcon class="w-4 h-4" />
                    {:else if room.room_info.platform === "douyin"}
                      <DouyinIcon class="w-4 h-4" />
                    {:else if room.room_info.platform === "douyu"}
                      <DouyuIcon class="w-4 h-4" />
                    {:else if room.room_info.platform === "kuaishou"}
                      <KuaishouIcon class="w-4 h-4" />
                    {:else if room.room_info.platform === "huya"}
                      <HuyaIcon class="w-4 h-4" />
                    {:else if room.room_info.platform === "tiktok"}
                      <TikTokIcon class="w-5 h-5" />
                    {:else if room.room_info.platform === "twitch"}
                      <TwitchIcon class="w-5 h-5 text-purple-600" />
                    {:else if room.room_info.platform === "youtube"}
                      <YouTubeIcon class="w-4 h-4" />
                    {:else if room.room_info.platform === "rtmp"}
                      <Radio class="w-4 h-4" />
                    {:else}
                      <Globe class="h-4 w-4 text-fg-faint" />
                    {/if}
                  </span>
                  <h3 class="text-[13px] font-medium text-fg">
                    {room.room_info.room_title}
                  </h3>
                </div>
              </div>
            </div>
            <div class="flex items-center justify-between">
              <div
                class="flex items-center space-x-2 text-[12px] text-fg-secondary"
              >
                <button
                  class="-ml-1 flex items-center space-x-2 rounded-[8px] p-1 pr-2 transition-colors hover:bg-control-hover"
                  onclick={() => {
                    openUserUrl(room);
                  }}
                >
                  <img
                    src={room.user_info.user_avatar}
                    alt="avatar"
                    class="h-7 w-7 rounded-full"
                  />
                  <span>{room.user_info.user_name}</span>
                </button>
              </div>
              <div class="flex items-center space-x-1">
                {#if room.recording}
                  <button
                    class="ui-btn ui-btn-ghost ui-btn-icon"
                    onclick={() => {
                      invoke("open_live", {
                        platform: room.room_info.platform,
                        roomId: room.room_info.room_id,
                        liveId: room.live_id,
                      });
                    }}
                  >
                    <Play class="h-4 w-4" />
                  </button>
                {/if}
                <button
                  class="ui-btn ui-btn-ghost ui-btn-icon"
                  title="录播历史"
                  aria-label={`查看 ${room.user_info.user_name || room.room_info.room_id} 的录播历史`}
                  onclick={() => { archiveRoom = room; }}
                >
                  <History class="h-4 w-4" />
                </button>
              </div>
            </div>
          </div>
        </div>
      {/each}

      <!-- Add Room Card -->
      <button
        class="flex flex-col items-center justify-center space-y-2 rounded-[10px] border border-dashed border-line-strong p-4 transition-colors hover:border-accent/60 hover:bg-surface/60"
        onclick={() => {
          addModal = true;
        }}
      >
        <div
          class="flex h-10 w-10 items-center justify-center rounded-full bg-accent-soft"
        >
          <Plus class="h-5 w-5 text-accent-fg" />
        </div>
        <span class="text-[13px] font-medium text-accent-fg">添加新直播间</span>
        <span class="text-[11px] text-fg-muted"
          >配置一个新直播间以及其相关设置</span
        >
      </button>
    </div>
  </div>
</div>
{#if deleteModal}
  <div class="ui-modal-backdrop" transition:fade={{ duration: 200 }}>
    <div
      class="ui-modal delete-modal w-[320px]"
      transition:scale={{ duration: 150, start: 0.95 }}
    >
      <div class="space-y-4 p-5">
        <div class="space-y-1.5 text-center">
          <h3 class="text-[14px] font-semibold text-fg">移除直播间</h3>
          <p class="text-[12px] text-fg-muted">
            此操作将移除所有相关的录制记录
          </p>
        </div>
        <DeleteConfirmActions
          bind:busy={deletingRoom}
          confirmLabel="移除"
          oncancel={() => (deleteModal = false)}
          onconfirm={async () => {
            await invoke("remove_recorder", {
              roomId: deleteRoom.room_info.room_id,
              platform: deleteRoom.room_info.platform,
            });
            deleteModal = false;
          }}
        />
      </div>
    </div>
  </div>
{/if}

{#if addModal}
  <div class="ui-modal-backdrop" transition:fade={{ duration: 200 }}>
    <div
      class="ui-modal add-modal w-[400px]"
      transition:scale={{ duration: 150, start: 0.95 }}
    >
      <!-- Header -->
      <div class="ui-modal-header">
        <h2>添加直播间</h2>
      </div>

      <div class="ui-modal-body space-y-4">
        <PlatformIconTabs
          bind:value={selectedPlatform}
          platforms={[
            { id: "bilibili", label: "哔哩哔哩" },
            { id: "douyin", label: "抖音" },
            { id: "douyu", label: "斗鱼" },
            { id: "huya", label: "虎牙" },
            { id: "kuaishou", label: "快手" },
            { id: "tiktok", label: "TikTok" },
            { id: "twitch", label: "Twitch" },
            { id: "youtube", label: "YouTube" },
            { id: "rtmp", label: "RTMP 推流" },
          ]}
        />

        <div class="space-y-1.5">
          <label for="room_id" class="ui-label block">
            {selectedPlatform === "rtmp" ? "房间名称" : "直播间 ID 或链接"}
          </label>
          <input
            id="room_id"
            type="text"
            bind:value={addRoom}
            class="ui-input w-full"
            placeholder={selectedPlatform === "rtmp" ? "例如：我的 OBS" : "请输入直播间 ID 或链接"}
          />
        </div>
        {#if selectedPlatform === "rtmp"}
          <p class="text-[12px] text-fg-muted">端口和推流密钥自动生成，创建后可在房间菜单的「状态」中查看。</p>
        {/if}
        <div class="space-y-1.5">
          <label for="add-room-storage" class="ui-label block">后续录制的存储偏好</label>
          {#if storageLoading}
            <p class="ui-hint" role="status">正在加载录制存储…</p>
          {:else if addStorage}
            <StorageBackendSelect storage={addStorage} id="add-room-storage" bind:value={addStorageId} disabled={creatingRoom} />
            <p class="ui-hint">为后续新录制跟随应用默认或指定存储；已有录播仍绑定各自原存储。</p>
          {/if}
          {#if storageError}
            <p class="text-[12px] text-danger-fg whitespace-pre-wrap" role="alert">{storageError}</p>
            <button class="ui-btn ui-btn-sm" disabled={storageLoading} onclick={() => loadAddStorage()}>重试</button>
          {/if}
        </div>
        {#if addError}<p class="text-[12px] text-danger-fg whitespace-pre-wrap" role="alert">{addError}</p>{/if}
      </div>

      <div class="ui-modal-footer">
        <button
          class="ui-btn"
          disabled={creatingRoom}
          onclick={() => {
            addModal = false;
          }}
        >
          取消
        </button>
        <button
          class="ui-btn ui-btn-primary"
          disabled={creatingRoom || storageLoading || !addStorage || !!storageError || !addRoom.trim()}
          onclick={() => {
            addNewRecorder(addRoom, selectedPlatform);
          }}
        >
          {creatingRoom ? "添加中…" : "添加"}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if currentArchiveRoom}
  {#key currentArchiveRoom.room_info.platform + ":" + currentArchiveRoom.room_info.room_id}
    <RoomArchiveModal room={currentArchiveRoom} onclose={() => { archiveRoom = null; }} />
  {/key}
{/if}

{#if statusRoom}
  <RoomStatusModal room={statusRoom} onclose={closeRoomStatus} />
{/if}

<svelte:window onmousedown={handleModalClickOutside} />

<style>
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
