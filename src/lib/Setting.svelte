<script lang="ts">
  import { invoke } from "@tauri-apps/api";
  import { open } from "@tauri-apps/api/dialog";
  import { Button, ButtonGroup, Input, Label } from "flowbite-svelte";

  let setting_model = {
    cache_path: "",
    clip_path: "",
  };

  interface Config {
    cache: string;
    output: string;
  }

  async function get_config() {
    let config: Config = await invoke("get_config");
    setting_model.cache_path = config.cache;
    setting_model.clip_path = config.output;
    console.log(config);
  }

  async function browse_folder() {
    const selected = await open({ directory: true });
    return Array.isArray(selected) ? selected[0] : selected;
  }

  get_config();
</script>

<Label>缓存目录</Label>
<ButtonGroup>
  <Input value={setting_model.cache_path} readonly />
  <Button
    on:click={async () => {
      const new_folder = await browse_folder();
      if (new_folder) {
        setting_model.cache_path = new_folder;
        await invoke("set_cache_path", {
          cachePath: setting_model.cache_path,
        });
      }
    }}>Browse</Button
  >
</ButtonGroup>

<Label class="mt-4">输出目录</Label>
<ButtonGroup>
  <Input value={setting_model.clip_path} readonly />
  <Button
    on:click={async () => {
      const new_folder = await browse_folder();
      if (new_folder) {
        setting_model.clip_path = new_folder;
        await invoke("set_output_path", {
          outputPath: setting_model.clip_path,
        });
      }
    }}>Browse</Button
  >
</ButtonGroup>
