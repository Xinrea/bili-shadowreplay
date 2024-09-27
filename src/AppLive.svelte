<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
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
  import type { AccountInfo } from "./lib/db";
  import { platform } from "@tauri-apps/plugin-os";
  import { ClapperboardPlaySolid } from "flowbite-svelte-icons";

  let use_titlebar = platform() == "windows";

  const appWindow = getCurrentWebviewWindow();
  const urlParams = new URLSearchParams(window.location.search);
  const port = urlParams.get("port");
  const room_id = urlParams.get("room_id");
  const ts = parseInt(urlParams.get("ts"));

  let profile = {
    title: "",
    desc: "",
    tag: "",
    dynamic: "",
  };

  let room_info;

  let loading = false;
  let start = 0.0;
  let end = 0.0;

  invoke("get_profile", { roomId: parseInt(room_id) }).then((p) => {
    //@ts-ignore
    profile = p;
    console.log(profile);
  });

  invoke("get_room_info", { roomId: parseInt(room_id) }).then((d) => {
    room_info = d;
    console.log(room_info);
    appWindow.setTitle(`[${room_id}]${room_info.room_title}`);
  });

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

  let video_file = null;
  let cover = null;
  let cover_text = "";
  let preview = false;
  let uid_selected = 0;

  let accounts = [];

  invoke("get_accounts").then((account_info: AccountInfo) => {
    accounts = account_info.accounts.map((a) => {
      return {
        value: a.uid,
        name: a.name,
      };
    });
  });

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
    cover = generateCover();
    video_file = await invoke("clip_range", {
      roomId: parseInt(room_id),
      ts: ts,
      x: start,
      y: end,
    });
    console.log("video file generatd:", video_file);
    loading = false;
  }

  async function do_post() {
    if (!video_file) {
      return;
    }
    appWindow.setTitle(`[${room_id}]${room_info.room_title} 上传中···`);
    loading = true;
    // render cover with text
    const ecapture = document.getElementById("capture");
    const render_canvas = await html2canvas(ecapture, {
      scale: 720 / ecapture.clientHeight,
    });
    const rendered_cover = render_canvas.toDataURL();
    invoke("upload_procedure", {
      uid: uid_selected,
      roomId: parseInt(room_id),
      file: video_file,
      cover: rendered_cover,
      profile: profile,
    })
      .then(() => {
        loading = false;
        appWindow.setTitle(`[${room_id}]${room_info.room_title} 投稿完成`);
        video_file = null;
        cover = null;
      })
      .catch((e) => {
        loading = false;
        appWindow.setTitle(`[${room_id}]${room_info.room_title} 投稿失败`);
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
        <video src={convertFileSrc(video_file)} controls />
      </Modal>
    </div>
    <div
      class="w-1/4 h-screen p-6 pt-12 overflow-auto border-solid bg-gray-50 border-l-2 border-slate-200"
    >
      <div>
        <!-- svelte-ignore a11y-click-events-have-key-events -->
        <div
          class="w-full"
          hidden={!video_file}
          on:click={() => {
            preview = true;
          }}
        >
          <div id="capture" class="cover-wrap relative cursor-pointer">
            <img src={cover} alt="cover" />
            <div
              class="absolute top-0 left-0 w-full h-full border-none bg-transparent resize-none text-amber-500 text-3xl font-bold px-8 py-2 drop-shadow cover-text"
            >
              {cover_text}
            </div>
          </div>
          {video_file}
        </div>
        {#if !video_file}
          <div class="w-full flex justify-center">
            <Button size="sm" on:click={generate_clip} disabled={loading}>
              {#if loading}
                <Spinner class="me-3" size="4" />
              {:else}
                <ClapperboardPlaySolid />
              {/if}
              从选区生成切片</Button
            >
          </div>
        {/if}
        <Hr />
        <Label class="mt-4">标题</Label>
        <Input bind:value={profile.title} />
        <Label class="mt-2">封面文本</Label>
        <Textarea bind:value={cover_text} />
        <Label class="mt-2">描述</Label>
        <Textarea bind:value={profile.desc} />
        <Label class="mt-2">标签</Label>
        <Input bind:value={profile.tag} />
        <Label class="mt-2">动态</Label>
        <Textarea bind:value={profile.dynamic} />
        <Label class="mt-2">视频分区</Label>
        <Input value="动画 - 综合" disabled />
        <Label class="mt-2">投稿账号</Label>
        <Select items={accounts} bind:value={uid_selected} />
      </div>
      {#if video_file}
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
</style>
