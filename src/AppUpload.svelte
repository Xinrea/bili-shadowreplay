<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api";
  import { Button, Input, Label, Spinner } from "flowbite-svelte";
  let data = {
    room_id: 0,
    file: "",
    cover: "",
    profile: {
      title: "",
      desc: "",
      tag: "",
      dynamic: "",
    },
  };

  let loading = false;

  listen("init", (e) => {
    //@ts-ignore
    data = e.payload;
  });

  async function do_post() {
    console.log(data);
    loading = true;
    invoke("upload_procedure", {
      roomId: data.room_id,
      file: data.file,
      cover: data.cover,
      profile: data.profile,
    })
      .then(() => {
        loading = false;
        // clear
        data.cover = "";
      })
      .catch((e) => {
        loading = false;
        alert(e);
      });
  }
</script>

<main>
  {#if data.cover}
    <div class="p-4">
      <img src={data.cover} alt="" />
      <Label class="mt-6">标题</Label>
      <Input bind:value={data.profile.title} />
      <Label class="mt-2">描述</Label>
      <Input bind:value={data.profile.desc} />
      <Label class="mt-2">标签</Label>
      <Input bind:value={data.profile.tag} />
      <Label class="mt-2">动态</Label>
      <Input bind:value={data.profile.dynamic} />
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
  {:else}
    <div class="w-full h-full justify-center flex items-center text-xl">
      暂无投稿数据生成
    </div>
  {/if}
</main>

<style>
  main {
    width: 100vw;
    height: 100vh;
  }
</style>
