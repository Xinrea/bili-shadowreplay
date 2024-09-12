<script lang="ts">
  import { fetch, ResponseType } from "@tauri-apps/api/http";
  export let src = "";
  export let iclass = "";
  let b = "";
  async function getImage(url: string) {
    if (!url) {
      return "";
    }
    const response = await fetch<Uint8Array>(url, {
      method: "GET",
      timeout: 3,
      responseType: ResponseType.Binary,
    });
    const binaryArray = new Uint8Array(response.data);
    var blob = new Blob([binaryArray], {
      type: response.headers["content-type"],
    });
    return URL.createObjectURL(blob);
  }
  async function init() {
    b = await getImage(src);
  }
  init();
</script>

<img src={b} class={iclass} alt="" />
