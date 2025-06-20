<script lang="ts">
  import {
    invoke,
    set_title,
    TAURI_ENV,
    convertFileSrc,
    listen,
    log,
  } from "./lib/invoker";
  import Player from "./lib/Player.svelte";
  import type { AccountInfo, RecordItem } from "./lib/db";
  import { ChevronRight, ChevronLeft, Play, Pen } from "lucide-svelte";
  import {
    type Profile,
    type VideoItem,
    type Config,
    type Marker,
    type ProgressUpdate,
    type ProgressFinished,
    clipRange,
    generateEventId,
  } from "./lib/interface";
  import TypeSelect from "./lib/TypeSelect.svelte";
  import MarkerPanel from "./lib/MarkerPanel.svelte";
  import CoverEditor from "./lib/CoverEditor.svelte";
  import VideoPreview from "./lib/VideoPreview.svelte";
  import { onDestroy, onMount } from "svelte";

  const urlParams = new URLSearchParams(window.location.search);
  const room_id = parseInt(urlParams.get("room_id"));
  const platform = urlParams.get("platform");
  const live_id = urlParams.get("live_id");
  const focus_start = parseInt(urlParams.get("start") || "0");
  const focus_end = parseInt(urlParams.get("end") || "0");

  log.info("AppLive loaded", room_id, platform, live_id);

  // get profile in local storage with a default value
  let profile: Profile = get_profile();
  let config: Config = null;

  invoke("get_config").then((c) => {
    config = c as Config;
    console.log(config);
  });

  function get_profile(): Profile {
    const profile_str = window.localStorage.getItem("profile-" + room_id);
    if (profile_str && profile_str.includes("videos")) {
      return JSON.parse(profile_str);
    }
    return default_profile();
  }

  $: {
    window.localStorage.setItem("profile-" + room_id, JSON.stringify(profile));
  }

  function default_profile(): Profile {
    return {
      videos: [],
      cover: "",
      cover43: null,
      title: "",
      copyright: 1,
      tid: 27,
      tag: "",
      desc_format_id: 9999,
      desc: "",
      recreate: -1,
      dynamic: "",
      interactive: 0,
      act_reserve_create: 0,
      no_disturbance: 0,
      no_reprint: 0,
      subtitle: {
        open: 0,
        lan: "",
      },
      dolby: 0,
      lossless_music: 0,
      up_selection_reply: false,
      up_close_danmu: false,
      up_close_reply: false,
      web_os: 0,
    };
  }

  let current_clip_event_id = null;
  let current_post_event_id = null;
  let danmu_enabled = false;

  const update_listener = listen<ProgressUpdate>(`progress-update`, (e) => {
    console.log("progress-update event", e.payload.id);
    let event_id = e.payload.id;
    if (event_id === current_clip_event_id) {
      update_clip_prompt(e.payload.content);
    } else if (event_id === current_post_event_id) {
      update_post_prompt(e.payload.content);
    }
  });
  const finished_listener = listen<ProgressFinished>(
    `progress-finished`,
    (e) => {
      console.log("progress-finished event", e.payload.id);
      let event_id = e.payload.id;
      if (event_id === current_clip_event_id) {
        console.log("clip event finished", event_id);
        update_clip_prompt(`生成切片`);
        if (!e.payload.success) {
          alert("请检查 ffmpeg 是否配置正确：" + e.payload.message);
        }
        current_clip_event_id = null;
      } else if (event_id === current_post_event_id) {
        update_post_prompt(`投稿`);
        if (!e.payload.success) {
          alert(e.payload.message);
        }
        current_post_event_id = null;
      }
    }
  );

  onDestroy(() => {
    update_listener?.then((fn) => fn());
    finished_listener?.then((fn) => fn());
  });

  let archive: RecordItem = null;

  let start = 0.0;
  let end = 0.0;
  let global_offset = 0;

  // load start and end from localStorage
  if (localStorage.getItem(`${live_id}_start`)) {
    start = parseFloat(localStorage.getItem(`${live_id}_start`)) - focus_start;
  }
  if (localStorage.getItem(`${live_id}_end`)) {
    end = parseFloat(localStorage.getItem(`${live_id}_end`)) - focus_start;
  }

  console.log("Loaded start and end", start, end);

  function generateCover() {
    const video = document.getElementById("video") as HTMLVideoElement;
    var w = video.videoWidth;
    var h = video.videoHeight;
    var canvas = document.createElement("canvas");
    canvas.width = 1280;
    canvas.height = 720;
    var context = canvas.getContext("2d");
    context.drawImage(video, 0, 0, w, h, 0, 0, 1280, 720);
    return canvas.toDataURL();
  }

  let preview = false;
  let show_cover_editor = false;
  let show_clip_confirm = false;
  let text_style = {
    position: { x: 8, y: 8 },
    fontSize: 24,
    color: "#FF7F00",
  };
  let uid_selected = 0;
  let video_selected = 0;
  let accounts = [];
  let videos = [];

  let selected_video = null;

  let video: HTMLVideoElement;

  function pauseVideo() {
    if (video) {
      video.pause();
    }
  }

  // Initialize video element when component is mounted
  onMount(() => {
    video = document.getElementById("video") as HTMLVideoElement;
  });

  invoke("get_accounts").then((account_info: AccountInfo) => {
    accounts = account_info.accounts.map((a) => {
      return {
        value: a.uid,
        name: a.name,
        platform: a.platform,
      };
    });
    accounts = accounts.filter((a) => a.platform === "bilibili");
  });

  get_video_list();

  invoke("get_archive", { roomId: room_id, liveId: live_id }).then(
    (a: RecordItem) => {
      console.log(a);
      archive = a;
      set_title(`[${room_id}]${archive.title}`);
    }
  );

  function update_clip_prompt(str: string) {
    // update button text
    const span = document.getElementById("generate-clip-prompt");
    if (span) {
      span.textContent = str;
    }
  }

  function update_post_prompt(str: string) {
    const span = document.getElementById("post-prompt");
    if (span) {
      span.textContent = str;
    }
  }

  async function get_video_list() {
    videos = (
      (await invoke("get_videos", { roomId: room_id })) as VideoItem[]
    ).map((v) => {
      return {
        id: v.id,
        value: v.id,
        name: v.file,
        file: convertFileSrc(config.output + "/" + v.file),
        cover: v.cover,
      };
    });
  }

  function find_video(e) {
    if (!e.target) {
      selected_video = null;
      return;
    }
    const id = parseInt(e.target.value);
    selected_video = videos.find((v) => {
      return v.value == id;
    });
    console.log("video selected", videos, selected_video, e, id);
  }

  async function generate_clip() {
    if (end == 0) {
      alert("请检查选区范围");
      return;
    }
    if (end - start < 5.0) {
      alert("选区过短:," + (end - start).toFixed(2));
      return;
    }

    show_clip_confirm = true;
  }

  async function confirm_generate_clip() {
    show_clip_confirm = false;
    let new_cover = generateCover();
    update_clip_prompt(`切片生成中`);
    let event_id = generateEventId();
    current_clip_event_id = event_id;
    let new_video = await clipRange(event_id, {
      title: archive.title,
      room_id: room_id,
      platform: platform,
      cover: new_cover,
      live_id: live_id,
      x: Math.floor(focus_start + start),
      y: Math.floor(focus_start + end),
      danmu: danmu_enabled,
      offset: global_offset,
      local_offset:
        parseInt(localStorage.getItem(`local_offset:${live_id}`) || "0", 10) ||
        0,
    });
    console.log("video file generatd:", new_video);
    await get_video_list();
    video_selected = new_video.id;
    selected_video = videos.find((v) => {
      return v.value == new_video.id;
    });
    if (selected_video) {
      selected_video.cover = new_video.cover;
    }
  }

  async function do_post() {
    if (!selected_video) {
      return;
    }

    let event_id = generateEventId();
    current_post_event_id = event_id;

    update_post_prompt(`投稿上传中`);
    // update profile in local storage
    window.localStorage.setItem("profile-" + room_id, JSON.stringify(profile));
    invoke("upload_procedure", {
      uid: uid_selected,
      eventId: event_id,
      roomId: room_id,
      videoId: video_selected,
      cover: selected_video.cover,
      profile: profile,
    }).then(async () => {
      video_selected = 0;
      await get_video_list();
    });
  }

  async function cancel_clip() {
    if (!current_clip_event_id) {
      return;
    }
    invoke("cancel", { eventId: current_clip_event_id });
  }

  async function cancel_post() {
    if (!current_post_event_id) {
      return;
    }
    invoke("cancel", { eventId: current_post_event_id });
  }

  async function delete_video() {
    if (!selected_video) {
      return;
    }
    await invoke("delete_video", { id: video_selected });
    video_selected = 0;
    selected_video = null;
    await get_video_list();
  }
  let player;
  let lpanel_collapsed = true;
  let rpanel_collapsed = false;
  let markers: Marker[] = [];
  // load markers from local storage
  markers = JSON.parse(
    window.localStorage.getItem(`markers:${room_id}:${live_id}`) || "[]"
  );
  $: {
    // makers changed, save to local storage
    window.localStorage.setItem(
      `markers:${room_id}:${live_id}`,
      JSON.stringify(markers)
    );
  }

  async function save_video() {
    if (!selected_video) {
      return;
    }
    // download video
    const video_url = selected_video.file;
    const video_name = selected_video.name;
    const a = document.createElement("a");
    a.href = video_url;
    a.download = video_name;
    a.click();
  }
</script>

<main>
  <div class="flex flex-row overflow-hidden">
    <div
      class="flex relative h-screen border-solid bg-gray-950 border-r-2 border-gray-800 z-[501] transition-all duration-300 ease-in-out"
      class:w-[200px]={!lpanel_collapsed}
      class:w-0={lpanel_collapsed}
    >
      <div class="relative flex w-full overflow-hidden">
        <div
          class="w-[200px] transition-all duration-300 overflow-hidden flex-shrink-0"
          style="margin-left: {lpanel_collapsed ? '-200px' : '0'};"
        >
          <div class="w-full whitespace-nowrap">
            <MarkerPanel
              {archive}
              bind:markers
              on:markerClick={(e) => {
                player.seek(e.detail.offset);
              }}
            />
          </div>
        </div>
      </div>
      <button
        class="collapse-btn lp transition-transform duration-300 absolute"
        on:click={() => {
          lpanel_collapsed = !lpanel_collapsed;
        }}
      >
        {#if lpanel_collapsed}
          <ChevronRight class="text-white" size={20} />
        {:else}
          <ChevronLeft class="text-white" size={20} />
        {/if}
      </button>
    </div>
    <div class="overflow-hidden h-screen w-full relative">
      <Player
        bind:start
        bind:end
        bind:global_offset
        bind:this={player}
        {focus_start}
        {focus_end}
        {platform}
        {room_id}
        {live_id}
        {markers}
        on:markerAdd={(e) => {
          markers.push({
            offset: e.detail.offset,
            realtime: e.detail.realtime,
            content: "[空标记点]",
          });
          markers = markers.sort((a, b) => a.offset - b.offset);
        }}
      />
      <VideoPreview
        bind:show={preview}
        video={selected_video}
        roomId={room_id}
        {videos}
        onVideoChange={(video) => {
          selected_video = video;
        }}
        onClose={() => {
          preview = false;
        }}
        onVideoListUpdate={get_video_list}
      />
    </div>
    <div
      class="flex relative h-screen border-solid bg-gray-950 border-l-2 border-gray-800 text-white transition-all duration-300 ease-in-out"
      class:w-[400px]={!rpanel_collapsed}
      class:w-0={rpanel_collapsed}
    >
      <button
        class="collapse-btn rp transition-transform duration-300"
        class:translate-x-[-20px]={!rpanel_collapsed}
        class:translate-x-0={rpanel_collapsed}
        on:click={() => {
          rpanel_collapsed = !rpanel_collapsed;
        }}
      >
        {#if rpanel_collapsed}
          <ChevronLeft class="text-white" size={20} />
        {:else}
          <ChevronRight class="text-white" size={20} />
        {/if}
      </button>
      <div
        id="post-panel"
        class="h-screen bg-[#1c1c1e] text-white w-[400px] flex flex-col transition-opacity duration-300"
        class:opacity-0={rpanel_collapsed}
        class:opacity-100={!rpanel_collapsed}
        class:invisible={rpanel_collapsed}
      >
        <!-- 顶部标题栏 -->
        <div
          class="flex-none sticky top-0 z-10 backdrop-blur-xl bg-[#1c1c1e]/80 px-6 py-4 border-b border-gray-800/50"
        >
          <h2 class="text-lg font-medium">视频投稿</h2>
        </div>

        <!-- 内容区域 -->
        <div class="flex-1 overflow-y-auto">
          <div class="px-6 py-4 space-y-8">
            <!-- 切片操作区 -->
            <section class="space-y-3">
              <div class="flex items-center justify-between">
                <h3 class="text-sm font-medium text-gray-300">切片列表</h3>
                <div class="flex space-x-2">
                  <button
                    on:click={generate_clip}
                    disabled={current_clip_event_id != null}
                    class="px-4 py-1.5 bg-[#0A84FF] text-white text-sm rounded-lg
                           transition-all duration-200 hover:bg-[#0A84FF]/90
                           disabled:opacity-50 disabled:cursor-not-allowed
                           flex items-center space-x-2"
                  >
                    {#if current_clip_event_id != null}
                      <div
                        class="w-4 h-4 border-2 border-current border-t-transparent rounded-full animate-spin"
                      />
                    {/if}
                    <span id="generate-clip-prompt">生成切片</span>
                  </button>
                  {#if current_clip_event_id != null}
                    <button
                      on:click={cancel_clip}
                      class="px-4 py-1.5 text-red-500 text-sm rounded-lg
                             transition-all duration-200 hover:bg-red-500/10
                             disabled:opacity-50 disabled:cursor-not-allowed"
                    >
                      取消
                    </button>
                  {/if}
                  {#if selected_video}
                    <button
                      on:click={delete_video}
                      class="px-4 py-1.5 text-red-500 text-sm rounded-lg
                             transition-all duration-200 hover:bg-red-500/10
                             disabled:opacity-50 disabled:cursor-not-allowed"
                    >
                      删除
                    </button>
                  {/if}
                </div>
              </div>

              <div class="flex flex-row items-center justify-between">
                <select
                  bind:value={video_selected}
                  on:change={find_video}
                  class="w-full px-3 py-2 bg-[#2c2c2e] text-white rounded-lg
                       border border-gray-800/50 focus:border-[#0A84FF]
                       transition duration-200 outline-none appearance-none
                       hover:border-gray-700/50"
                >
                  <option value={0}>选择切片</option>
                  {#each videos as video}
                    <option value={video.value}>{video.name}</option>
                  {/each}
                </select>
                {#if !TAURI_ENV && selected_video}
                  <button
                    on:click={save_video}
                    class="w-24 ml-2 px-3 py-2 bg-[#0A84FF] text-white rounded-lg
                     transition-all duration-200 hover:bg-[#0A84FF]/90
                     disabled:opacity-50 disabled:cursor-not-allowed"
                  >
                    下载
                  </button>
                {/if}
              </div>
            </section>
            <!-- 封面预览 -->
            {#if selected_video && selected_video.id != -1}
              <section>
                <div class="group">
                  <div
                    class="text-sm text-gray-400 mb-2 flex items-center justify-between"
                  >
                    <span>视频封面</span>
                    <button
                      class="text-[#0A84FF] hover:text-[#0A84FF]/80 transition-colors duration-200 flex items-center space-x-1"
                      on:click={() => (show_cover_editor = true)}
                    >
                      <Pen class="w-4 h-4" />
                      <span class="text-xs">创建新封面</span>
                    </button>
                  </div>
                  <!-- svelte-ignore a11y-click-events-have-key-events -->
                  <div
                    id="capture"
                    class="relative rounded-xl overflow-hidden bg-black/20 border border-gray-800/50 cursor-pointer group"
                    on:click={() => {
                      pauseVideo();
                      preview = true;
                    }}
                  >
                    <div
                      class="absolute inset-0 bg-black/40 opacity-0 group-hover:opacity-100
                              transition duration-200 flex items-center justify-center backdrop-blur-[2px]"
                    >
                      <div
                        class="bg-white/10 backdrop-blur p-3 rounded-full opacity-0 group-hover:opacity-50"
                      >
                        <Play class="w-6 h-6 text-white" />
                      </div>
                    </div>
                    <img
                      src={selected_video.cover}
                      alt="视频封面"
                      class="w-full"
                    />
                  </div>
                </div>
              </section>
            {/if}

            <!-- 表单区域 -->
            <section class="space-y-8">
              <!-- 基本信息 -->
              <div class="space-y-4">
                <h3 class="text-sm font-medium text-gray-400">基本信息</h3>
                <!-- 标题 -->
                <div class="space-y-2">
                  <label
                    for="title"
                    class="block text-sm font-medium text-gray-300">标题</label
                  >
                  <input
                    id="title"
                    type="text"
                    bind:value={profile.title}
                    placeholder="输入视频标题"
                    class="w-full px-3 py-2 bg-[#2c2c2e] text-white rounded-lg
                           border border-gray-800/50 focus:border-[#0A84FF]
                           transition duration-200 outline-none
                           hover:border-gray-700/50"
                  />
                </div>

                <!-- 视频分区 -->
                <div class="space-y-2">
                  <label
                    for="tid"
                    class="block text-sm font-medium text-gray-300"
                    >视频分区</label
                  >
                  <div class="w-full" id="tid">
                    <TypeSelect bind:value={profile.tid} />
                  </div>
                </div>

                <!-- 投稿账号 -->
                <div id="uid" class="space-y-2">
                  <label
                    for="uid"
                    class="block text-sm font-medium text-gray-300"
                    >投稿账号</label
                  >
                  <select
                    bind:value={uid_selected}
                    class="w-full px-3 py-2 bg-[#2c2c2e] text-white rounded-lg
                           border border-gray-800/50 focus:border-[#0A84FF]
                           transition duration-200 outline-none appearance-none
                           hover:border-gray-700/50"
                  >
                    {#each accounts as account}
                      <option value={account.value}>{account.name}</option>
                    {/each}
                  </select>
                </div>
              </div>

              <!-- 详细信息 -->
              <div class="space-y-4">
                <h3 class="text-sm font-medium text-gray-400">详细信息</h3>
                <!-- 描述 -->
                <div class="space-y-2">
                  <label
                    for="desc"
                    class="block text-sm font-medium text-gray-300">描述</label
                  >
                  <textarea
                    id="desc"
                    bind:value={profile.desc}
                    placeholder="输入视频描述"
                    class="w-full px-3 py-2 bg-[#2c2c2e] text-white rounded-lg
                           border border-gray-800/50 focus:border-[#0A84FF]
                           transition duration-200 outline-none resize-none h-32
                           hover:border-gray-700/50"
                  />
                </div>

                <!-- 标签 -->
                <div class="space-y-2">
                  <label
                    for="tag"
                    class="block text-sm font-medium text-gray-300">标签</label
                  >
                  <input
                    id="tag"
                    type="text"
                    bind:value={profile.tag}
                    placeholder="输入视频标签，用逗号分隔"
                    class="w-full px-3 py-2 bg-[#2c2c2e] text-white rounded-lg
                           border border-gray-800/50 focus:border-[#0A84FF]
                           transition duration-200 outline-none
                           hover:border-gray-700/50"
                  />
                </div>

                <!-- 动态 -->
                <div class="space-y-2">
                  <label
                    for="dynamic"
                    class="block text-sm font-medium text-gray-300">动态</label
                  >
                  <textarea
                    id="dynamic"
                    bind:value={profile.dynamic}
                    placeholder="输入动态内容"
                    class="w-full px-3 py-2 bg-[#2c2c2e] text-white rounded-lg
                           border border-gray-800/50 focus:border-[#0A84FF]
                           transition duration-200 outline-none resize-none h-32
                           hover:border-gray-700/50"
                  />
                </div>
              </div>
            </section>

            <!-- 投稿按钮 -->
            {#if selected_video}
              <div class="h-10" />
            {/if}
          </div>
        </div>

        <!-- 底部按钮 -->
        {#if selected_video}
          <div
            class="flex-none sticky bottom-0 px-6 py-4 bg-gradient-to-t from-[#1c1c1e] via-[#1c1c1e] to-transparent"
          >
            <div class="flex gap-3">
              <button
                on:click={do_post}
                disabled={current_post_event_id != null}
                class="flex-1 px-4 py-2.5 bg-[#0A84FF] text-white rounded-lg
                       transition-all duration-200 hover:bg-[#0A84FF]/90
                       disabled:opacity-50 disabled:cursor-not-allowed
                       flex items-center justify-center space-x-2"
              >
                {#if current_post_event_id != null}
                  <div
                    class="w-4 h-4 border-2 border-current border-t-transparent rounded-full animate-spin"
                  />
                {/if}
                <span id="post-prompt">投稿</span>
              </button>
              {#if current_post_event_id != null}
                <button
                  on:click={() => cancel_post()}
                  class="w-24 px-3 py-2 bg-red-500 text-white rounded-lg
                         transition-all duration-200 hover:bg-red-500/90
                         flex items-center justify-center"
                >
                  取消
                </button>
              {/if}
            </div>
          </div>
        {/if}
      </div>
    </div>
  </div>
</main>

<!-- Clip Confirmation Dialog -->
{#if show_clip_confirm}
  <div
    class="fixed inset-0 bg-gray-900/50 backdrop-blur-sm flex items-center justify-center z-50"
  >
    <div class="bg-[#1c1c1e] rounded-lg p-6 max-w-md w-full mx-4">
      <h3 class="text-lg font-medium text-white mb-4">确认生成切片</h3>
      <div class="space-y-4">
        <div class="text-sm text-gray-300">
          <p>切片时长: {(end - start).toFixed(2)} 秒</p>
        </div>
        <div class="flex items-center space-x-2">
          <input
            type="checkbox"
            id="confirm-danmu-checkbox"
            bind:checked={danmu_enabled}
            class="w-4 h-4 text-[#0A84FF] bg-[#2c2c2e] border-gray-800 rounded focus:ring-[#0A84FF] focus:ring-offset-[#1c1c1e]"
          />
          <label for="confirm-danmu-checkbox" class="text-sm text-gray-300"
            >压制弹幕</label
          >
        </div>
        <div class="flex justify-end space-x-3">
          <button
            on:click={() => (show_clip_confirm = false)}
            class="px-4 py-2 text-gray-300 hover:text-white transition-colors duration-200"
          >
            取消
          </button>
          <button
            on:click={confirm_generate_clip}
            class="px-4 py-2 bg-[#0A84FF] text-white rounded-lg hover:bg-[#0A84FF]/90 transition-colors duration-200"
          >
            确认生成
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

<CoverEditor
  bind:show={show_cover_editor}
  video={selected_video}
  on:coverUpdate={(event) => {
    selected_video = {
      ...selected_video,
      cover: event.detail.cover,
    };
  }}
/>

<style>
  main {
    width: 100vw;
    height: 100vh;
  }

  .collapse-btn {
    position: absolute;
    z-index: 50;
    top: 50%;
    width: 20px;
    height: 40px;
  }
  .collapse-btn.rp {
    left: -20px;
    border-radius: 4px 0 0 4px;
    border: 2px solid rgb(31 41 55 / var(--tw-border-opacity));
    border-right: none;
    background-color: rgb(3 7 18 / var(--tw-bg-opacity));
    transform: translateY(-50%);
  }
  .collapse-btn.lp {
    right: -20px;
    border-radius: 0 4px 4px 0;
    border: 2px solid rgb(31 41 55 / var(--tw-border-opacity));
    border-left: none;
    background-color: rgb(3 7 18 / var(--tw-bg-opacity));
    transform: translateY(-50%);
  }
</style>
