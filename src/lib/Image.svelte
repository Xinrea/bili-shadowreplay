<script lang="ts">
  import { fetch } from "@tauri-apps/plugin-http";
  export let src = "";
  export let iclass = "";
  let b = "";
  async function getImage(url: string) {
    if (!url) {
      return "/imgs/douyin.png";
    }
    const response = await fetch(url, {
      method: "GET",
    });
    return URL.createObjectURL(await response.blob());
  }
  async function init() {
    try {
      b = await getImage(src);
    } catch (e) {
      console.error(e);
    }
  }
  init();
</script>

<img src={b} class={iclass} alt="" />
