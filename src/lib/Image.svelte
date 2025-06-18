<script lang="ts">
  import { get, log } from "./invoker";
  export let src = "";
  export let iclass = "";
  let b = "";
  async function getImage(url: string) {
    if (!url) {
      return "/imgs/douyin.png";
    }
    if (url.startsWith("data")) {
      return url;
    }
    const response = await get(url);
    return URL.createObjectURL(await response.blob());
  }
  async function init() {
    try {
      b = await getImage(src);
    } catch (e) {
      log.error("Failed to get image:", e);
    }
  }
  init();
</script>

<img src={b} class={iclass} alt="" />
