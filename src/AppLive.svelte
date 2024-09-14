<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import { Button, Input, Label, Spinner, Textarea } from "flowbite-svelte";
  import Player from "./lib/Player.svelte";
  import TitleBar from "./lib/TitleBar.svelte";
  import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

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

  async function do_post() {
    if (end == 0) {
      alert("请检查选区范围");
      return;
    }
    if (end - start < 5.0) {
      alert("选区过短:," + (end - start).toFixed(2));
      return;
    }
    appWindow.setTitle(`[${room_id}]${room_info.room_title} 切片中···`);
    const video_file = await invoke("clip_range", {
      roomId: parseInt(room_id),
      ts: ts,
      x: start,
      y: end,
    });
    appWindow.setTitle(`[${room_id}]${room_info.room_title} 完成`);
    console.log("video file generatd:", video_file);
    const cover = generateCover();
    appWindow.setTitle(`[${room_id}]${room_info.room_title} 上传中···`);
    loading = true;
    invoke("upload_procedure", {
      roomId: parseInt(room_id),
      file: video_file,
      cover: cover,
      profile: profile,
    })
      .then(() => {
        loading = false;
        appWindow.setTitle(`[${room_id}]${room_info.room_title} 投稿完成`);
      })
      .catch((e) => {
        loading = false;
        appWindow.setTitle(`[${room_id}]${room_info.room_title} 投稿失败`);
        alert(e);
      });
  }
</script>

<main>
  <TitleBar dark />
  <div class="flex flex-row">
    <div class="w-3/4">
      <Player bind:start bind:end {port} {room_id} {ts} />
    </div>
    <div
      class="w-1/4 h-screen border-solid bg-gray-50 border-l-2 border-slate-200"
    >
      <div class="p-6">
        <Label class="mt-6">标题</Label>
        <Input bind:value={profile.title} />
        <Label class="mt-2">描述</Label>
        <Textarea bind:value={profile.desc} />
        <Label class="mt-2">标签</Label>
        <Input bind:value={profile.tag} />
        <Label class="mt-2">动态</Label>
        <Textarea bind:value={profile.dynamic} />
        <Label class="mt-2">视频分区</Label>
        <Input value="动画 - 综合" disabled />
      </div>
      <div class="flex justify-center w-full">
        <Button on:click={do_post} disabled={loading}>
          {#if loading}
            <Spinner class="me-3" size="4" />
          {/if}
          投稿</Button
        >
      </div>
    </div>
  </div>
</main>

<style>
  main {
    width: 100vw;
    height: 100vh;
  }
</style>
