<script lang="ts">
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import {
    Button,
    ButtonGroup,
    Input,
    Label,
    Spinner,
    Textarea,
    Modal,
    Select,
    Hr,
  } from "flowbite-svelte";
  import Player from "./lib/Player.svelte";
  import TitleBar from "./lib/TitleBar.svelte";
  import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
  import html2canvas from "html2canvas";
  import type { AccountInfo, RecordItem } from "./lib/db";
  import { platform } from "@tauri-apps/plugin-os";
  import {
    AngleRightOutline,
    AngleLeftOutline,
    ClapperboardPlaySolid,
    PlayOutline,
  } from "flowbite-svelte-icons";
  import type { Profile, VideoItem, Config, Marker } from "./lib/interface";
  import { onMount } from "svelte";
  import TypeSelect from "./lib/TypeSelect.svelte";
  import MarkerPanel from "./lib/MarkerPanel.svelte";

  let use_titlebar = platform() == "windows";

  const appWindow = getCurrentWebviewWindow();
  const urlParams = new URLSearchParams(window.location.search);
  const port = urlParams.get("port");
  const room_id = parseInt(urlParams.get("room_id"));
  const ts = parseInt(urlParams.get("ts"));

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

  let archive: RecordItem = null;

  let loading = false;
  let start = 0.0;
  let end = 0.0;

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

  let cover_text = "";
  let preview = false;
  let uid_selected = 0;
  let video_selected = 0;
  $: video_src = video ? convertFileSrc(config.output + "/" + video.name) : "";

  let accounts = [];
  let videos = [];

  let video = null;
  let cover = "";

  invoke("get_accounts").then((account_info: AccountInfo) => {
    accounts = account_info.accounts.map((a) => {
      return {
        value: a.uid,
        name: a.name,
      };
    });
    console.log(accounts);
  });

  get_video_list();

  invoke("get_archive", { roomId: room_id, liveId: ts }).then(
    (a: RecordItem) => {
      console.log(a);
      archive = a;
      appWindow.setTitle(`[${room_id}][${format_ts(ts)}]${archive.title}`);
    },
  );

  function update_title(str: string) {
    appWindow.setTitle(
      `[${room_id}][${format_ts(ts)}]${archive.title} - ${str}`,
    );
  }

  function format_ts(ts: number) {
    const date = new Date(ts * 1000);
    return date.toLocaleString();
  }

  async function get_video_list() {
    videos = (
      (await invoke("get_videos", { roomId: room_id })) as VideoItem[]
    ).map((v) => {
      return {
        value: v.id,
        name: v.file,
        cover: v.cover,
      };
    });
    console.log(videos, video_selected);
  }

  function find_video(e) {
    const id = parseInt(e.target.value);
    video = videos.find((v) => {
      return v.value == id;
    });
    cover = video.cover;
    console.log("video selected", videos, video, e, id);
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
    loading = true;
    let new_cover = generateCover();
    update_title(`切片生成中`);
    try {
      let new_video = (await invoke("clip_range", {
        roomId: room_id,
        cover: new_cover,
        ts: ts,
        x: start,
        y: end,
      })) as VideoItem;
      update_title(`切片生成成功`);
      console.log("video file generatd:", video);
      await get_video_list();
      video_selected = new_video.id;
      video = videos.find((v) => {
        return v.value == new_video.id;
      });
      cover = new_video.cover;
      loading = false;
    } catch (e) {
      alert("Err generating clip: " + e);
    }
  }

  async function do_post() {
    if (!video) {
      return;
    }
    update_title(`投稿上传中`);
    loading = true;
    // render cover with text
    const ecapture = document.getElementById("capture");
    const render_canvas = await html2canvas(ecapture, {
      scale: 720 / ecapture.clientHeight,
    });
    const rendered_cover = render_canvas.toDataURL();
    // update profile in local storage
    window.localStorage.setItem("profile-" + room_id, JSON.stringify(profile));
    invoke("upload_procedure", {
      uid: uid_selected,
      roomId: room_id,
      videoId: video_selected,
      cover: rendered_cover,
      profile: profile,
    })
      .then(async () => {
        loading = false;
        update_title(`投稿成功`);
        video_selected = 0;
        await get_video_list();
      })
      .catch((e) => {
        loading = false;
        update_title(`投稿失败`);
        alert(e);
      });
  }

  async function delete_video() {
    if (!video) {
      return;
    }
    loading = true;
    update_title(`删除中`);
    await invoke("delete_video", { id: video_selected });
    update_title(`删除成功`);
    loading = false;
    video_selected = 0;
    video = null;
    cover = "";
    await get_video_list();
  }
  let player;
  let lpanel_collapsed = true;
  let rpanel_collapsed = false;
  let markers: Marker[] = [];
  // load markers from local storage
  markers = JSON.parse(
    window.localStorage.getItem(`markers:${room_id}:${ts}`) || "[]",
  );
  $: {
    // makers changed, save to local storage
    window.localStorage.setItem(
      `markers:${room_id}:${ts}`,
      JSON.stringify(markers),
    );
  }

  // when window resize, update post panel height
  onMount(() => {
    let post_panel = document.getElementById("post-panel");
    if (post_panel && !rpanel_collapsed) {
      post_panel.style.height = `calc(100vh - 35px)`;
    }
    window.addEventListener("resize", () => {
      if (post_panel && !rpanel_collapsed) {
        post_panel.style.height = `calc(100vh - 35px)`;
      }
    });
  });
</script>

<main>
  {#if use_titlebar}
    <TitleBar dark />
  {/if}
  <div class="flex flex-row overflow-hidden">
    <div
      class="flex relative h-screen border-solid bg-gray-950 border-r-2 border-gray-800 z-[39]"
      class:w14={!lpanel_collapsed}
    >
      <div class="w-full" hidden={lpanel_collapsed}>
        <MarkerPanel
          {archive}
          bind:markers
          on:markerClick={(e) => {
            player.seek(e.detail.offset);
          }}
        />
      </div>
      <button
        class="collapse-btn lp"
        on:click={() => {
          lpanel_collapsed = !lpanel_collapsed;
        }}
      >
        {#if lpanel_collapsed}
          <AngleRightOutline color="white" />
        {:else}
          <AngleLeftOutline color="white" />
        {/if}
      </button>
    </div>
    <div class="overflow-hidden h-screen w-full relative">
      <Player
        bind:start
        bind:end
        bind:this={player}
        {port}
        {room_id}
        {ts}
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
      }} />
      <Modal title="预览" bind:open={preview} autoclose>
        <!-- svelte-ignore a11y-media-has-caption -->
        <video src={video_src} controls />
      </Modal>
    </div>
    <div
      class="flex relative h-screen border-solid bg-gray-950 border-l-2 border-gray-800 text-white"
      class:w14={!rpanel_collapsed}
    >
      <button
        class="collapse-btn rp"
        on:click={() => {
          rpanel_collapsed = !rpanel_collapsed;
        }}
      >
        {#if rpanel_collapsed}
          <AngleLeftOutline color="white" />
        {:else}
          <AngleRightOutline color="white" />
        {/if}
      </button>
      <div
        id="post-panel"
        class="mt-6 overflow-y-auto overflow-x-hidden p-4 py-6 w-full text-white"
        class:titlebar={use_titlebar}
        hidden={rpanel_collapsed}
      >
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        {#if video}
          <div
            class="w-full mb-2"
            on:click={() => {
              preview = true;
            }}
          >
            <div id="capture" class="cover-wrap relative cursor-pointer">
              <div
                class="cover-text absolute py-1 px-8"
                class:play-icon={false}
              >
                {cover_text}
              </div>
              <div class="play-icon opacity-0">
                <PlayOutline class="w-full h-full absolute" color="white" />
              </div>
              <img src={cover} alt="cover" />
            </div>
          </div>
        {/if}
        <div class="w-full flex flex-col justify-center">
          <Label>切片列表</Label>
          <Select
            items={videos}
            bind:value={video_selected}
            on:change={find_video}
            class="mb-2"
          />
          <ButtonGroup>
            <Button
              on:click={generate_clip}
              disabled={loading}
              color="primary"
              class="w-3/4"
            >
              {#if loading}
                <Spinner class="me-3" size="4" />
              {:else}
                <ClapperboardPlaySolid />
              {/if}
              从选区生成新切片</Button
            >
            <Button
              color="red"
              class="w-1/4"
              disabled={!loading && !video}
              on:click={delete_video}>删除</Button
            >
          </ButtonGroup>
        </div>
        <Hr />
        <Label class="mt-4">标题</Label>
        <Input size="sm" bind:value={profile.title} />
        <Label class="mt-2">封面文本</Label>
        <Textarea bind:value={cover_text} />
        <Label class="mt-2">描述</Label>
        <Textarea bind:value={profile.desc} />
        <Label class="mt-2">标签</Label>
        <Input size="sm" bind:value={profile.tag} />
        <Label class="mt-2">动态</Label>
        <Textarea bind:value={profile.dynamic} />
        <Label class="mt-2">视频分区</Label>
        <TypeSelect bind:value={profile.tid} />
        <Label class="mt-2">投稿账号</Label>
        <Select size="sm" items={accounts} bind:value={uid_selected} />
        {#if video}
          <div class="flex mt-4 justify-center w-full">
            <Button on:click={do_post} disabled={loading}>
              {#if loading}
                <Spinner class="me-3" size="4" />
              {/if}
              投稿
            </Button>
          </div>
        {/if}
      </div>
    </div>
  </div>
</main>

<style>
  main {
    width: 100vw;
    height: 100vh;
  }
  .cover-wrap:hover {
    opacity: 0.8;
  }
  .cover-wrap:hover .play-icon {
    opacity: 0.5;
  }
  .cover-text {
    white-space: pre-wrap;
    font-size: 24px;
    line-height: 1.3;
    font-weight: bold;
    color: rgb(255, 127, 0);
    text-shadow:
      -1px -1px 0 rgba(255, 255, 255, 1),
      1px -1px 0 rgba(255, 255, 255, 1),
      -1px 1px 0 rgba(255, 255, 255, 1),
      1px 1px 0 rgba(255, 255, 255, 1),
      -2px -2px 0 rgba(255, 255, 255, 0.5),
      2px -2px 0 rgba(255, 255, 255, 0.5),
      -2px 2px 0 rgba(255, 255, 255, 0.5),
      2px 2px 0 rgba(255, 255, 255, 0.5); /* 创建细腻的白色描边效果 */
  }
  .w14 {
    @apply w-1/4;
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
  }
  .collapse-btn.lp {
    right: -20px;
    border-radius: 0 4px 4px 0;
    border: 2px solid rgb(31 41 55 / var(--tw-border-opacity));
    border-left: none;
    background-color: rgb(3 7 18 / var(--tw-bg-opacity));
  }
</style>
