<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import {
    Button,
    ButtonGroup,
    Toggle,
    Input,
    Label,
    Card,
  } from "flowbite-svelte";

  import type { Config } from "./interface";

  let setting_model: Config = {
    cache: "",
    output: "",
    primary_uid: 0,
    live_start_notify: true,
    live_end_notify: true,
    clip_notify: true,
    post_notify: true,
  };

  async function get_config() {
    let config: Config = await invoke("get_config");
    setting_model = config;
    console.log(config);
  }

  async function browse_folder() {
    const selected = await open({ directory: true });
    return Array.isArray(selected) ? selected[0] : selected;
  }

  async function update_notify() {
    await invoke("update_notify", {
      liveStartNotify: setting_model.live_start_notify,
      liveEndNotify: setting_model.live_end_notify,
      clipNotify: setting_model.clip_notify,
      postNotify: setting_model.post_notify,
    });
  }

  get_config();
</script>

<div class="p-8 pt-12">
  <Card>
    <h5
      class="mb-2 text-2xl font-bold tracking-tight text-gray-900 dark:text-white"
    >
      通知设置
    </h5>
    <Toggle
      class="mb-2"
      bind:checked={setting_model.live_start_notify}
      on:change={update_notify}>开播通知</Toggle
    >
    <Toggle
      class="mb-2"
      bind:checked={setting_model.live_end_notify}
      on:change={update_notify}>下播通知</Toggle
    >
    <Toggle
      class="mb-2"
      bind:checked={setting_model.clip_notify}
      on:change={update_notify}>切片完成通知</Toggle
    >
    <Toggle
      class="mb-2"
      bind:checked={setting_model.post_notify}
      on:change={update_notify}>投稿完成通知</Toggle
    >
  </Card>
  <Card size="xl" class="mt-4">
    <h5
      class="mb-2 text-2xl font-bold tracking-tight text-gray-900 dark:text-white"
    >
      目录设置
    </h5>
    <Label>缓存目录</Label>
    <ButtonGroup>
      <Input value={setting_model.cache} readonly />
      <Button
        color="primary"
        on:click={async () => {
          const new_folder = await browse_folder();
          if (new_folder) {
            setting_model.cache = new_folder;
            await invoke("set_cache_path", {
              cachePath: setting_model.cache,
            });
          }
        }}>Browse</Button
      >
      <Button
        color="alternative"
        on:click={async () => {
          await invoke("show_in_folder", {
            path: setting_model.cache,
          });
        }}>Open</Button
      >
    </ButtonGroup>

    <Label class="mt-4">输出目录</Label>
    <ButtonGroup>
      <Input value={setting_model.output} readonly />
      <Button
        color="primary"
        on:click={async () => {
          const new_folder = await browse_folder();
          if (new_folder) {
            setting_model.output = new_folder;
            await invoke("set_output_path", {
              outputPath: setting_model.output,
            });
          }
        }}>Browse</Button
      >
      <Button
        color="alternative"
        on:click={async () => {
          await invoke("show_in_folder", {
            path: setting_model.output,
          });
        }}>Open</Button
      >
    </ButtonGroup>
  </Card>
</div>
