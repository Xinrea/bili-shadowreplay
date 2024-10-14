<script lang="ts">
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import {
    Button,
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
  import { ClapperboardPlaySolid } from "flowbite-svelte-icons";
  import type { Profile, VideoItem } from "./lib/interface";

  let use_titlebar = platform() == "windows";

  const appWindow = getCurrentWebviewWindow();
  const urlParams = new URLSearchParams(window.location.search);
  const port = urlParams.get("port");
  const room_id = parseInt(urlParams.get("room_id"));
  const ts = parseInt(urlParams.get("ts"));

  // get profile in local storage with a default value
  let profile: Profile = get_profile();

  function get_profile(): Profile {
    const profile_str = window.localStorage.getItem("profile-" + room_id);
    if (profile_str && profile_str.includes("videos")) {
      return JSON.parse(profile_str);
    }
    return default_profile();
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

  let accounts = [];
  let videos = [];

  let video = null;

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
      appWindow.setTitle(`[${room_id}][${ts}]${archive.title}`);
    },
  );

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
    let cover = generateCover();
    appWindow.setTitle(`[${room_id}][${ts}]${archive.title} - 切片生成中`);
    let video = (await invoke("clip_range", {
      roomId: room_id,
      cover,
      ts: ts,
      x: start,
      y: end,
    })) as VideoItem;
    appWindow.setTitle(`[${room_id}][${ts}]${archive.title} - 切片生成成功`);
    console.log("video file generatd:", video);
    await get_video_list();
    video_selected = video.id;
    loading = false;
  }

  async function do_post() {
    if (!video) {
      return;
    }
    appWindow.setTitle(`[${room_id}][${ts}]${archive.title} - 投稿上传中`);
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
        appWindow.setTitle(`[${room_id}][${ts}]${archive.title} - 投稿成功`);
        video_selected = 0;
        await get_video_list();
      })
      .catch((e) => {
        loading = false;
        appWindow.setTitle(`[${room_id}][${ts}]${archive.title} - 投稿失败`);
        alert(e);
      });
  }
</script>

<main>
  {#if use_titlebar}
    <TitleBar dark />
  {/if}
  <div class="flex flex-row">
    <div class="w-3/4">
      <Player bind:start bind:end {port} {room_id} {ts} />
      <Modal title="切片预览" bind:open={preview} autoclose>
        <!-- svelte-ignore a11y-media-has-caption -->
        <video src={convertFileSrc(video.file)} controls />
      </Modal>
    </div>
    <div
      class="w-1/4 h-screen p-6 pt-6 overflow-auto border-solid bg-gray-50 border-l-2 border-slate-200"
      class:titlebar={use_titlebar}
    >
      <div>
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <div
          class="w-full"
          hidden={!video}
          on:click={() => {
            preview = true;
          }}
        >
          <div id="capture" class="cover-wrap relative cursor-pointer">
            <div class="cover-text absolute p-2">{cover_text}</div>
            <img src={video ? video.cover : ""} alt="cover" />
          </div>
        </div>
        <div class="w-full flex flex-col justify-center">
          <Label>切片列表</Label>
          <Select
            items={videos}
            bind:value={video_selected}
            on:change={find_video}
            class="mb-2"
          />
          <Button size="sm" on:click={generate_clip} disabled={loading}>
            {#if loading}
              <Spinner class="me-3" size="4" />
            {:else}
              <ClapperboardPlaySolid />
            {/if}
            从选区生成新切片</Button
          >
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
        <Input size="sm" value="动画 - 综合" disabled />
        <Label class="mt-2">投稿账号</Label>
        <Select size="sm" items={accounts} bind:value={uid_selected} />
      </div>
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
</main>

<style>
  main {
    width: 100vw;
    height: 100vh;
  }
  .cover-wrap:hover {
    opacity: 0.8;
  }
  .cover-text {
    white-space: pre-wrap;
    font-size: 24px;
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
  .titlebar {
    @apply pt-12;
  }
</style>
