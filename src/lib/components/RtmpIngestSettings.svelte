<script lang="ts">
  import { Copy } from "lucide-svelte";
  import { rtmpServerUrl, type RtmpIngestInfo } from "../rtmp";

  let { info }: { info: RtmpIngestInfo } = $props();
  let copied = $state("");
  let copyError = $state("");
  const endpoint = localStorage.getItem("endpoint");
  const backendHostname = typeof window.__TAURI_INTERNALS__ !== "undefined"
    ? "127.0.0.1"
    : new URL(endpoint || window.location.href, window.location.href).hostname;
  const serverUrl = $derived(rtmpServerUrl(info, backendHostname));

  async function copy(value: string, label: string) {
    copyError = "";
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(value);
      } else {
        // LAN HTTP pages do not expose the Clipboard API.
        const field = document.createElement("textarea");
        field.value = value;
        field.style.position = "fixed";
        field.style.opacity = "0";
        document.body.appendChild(field);
        field.select();
        try {
          if (!document.execCommand("copy")) throw new Error("copy failed");
        } finally {
          field.remove();
        }
      }
      copied = label;
    } catch {
      copyError = "复制失败，请选中输入框中的内容手动复制。";
    }
  }
</script>

<div class="space-y-2.5">
  {#if !info.listening}
    <p class="text-[12px] text-danger" role="alert">推流服务未启动，请检查端口占用和日志。</p>
  {/if}
  <div class="grid gap-3 sm:grid-cols-2">
    {#each [{ label: "推流地址", value: serverUrl }, { label: "推流密钥", value: info.stream_key }] as field}
      <label class="block min-w-0 space-y-1.5">
        <span class="ui-label">{field.label}</span>
        <div class="flex gap-2">
          <input class="ui-input min-w-0 flex-1 font-mono" readonly value={field.value} onclick={(event) => event.currentTarget.select()} />
          <button class="ui-btn ui-btn-icon" aria-label={`复制${field.label}`} onclick={() => copy(field.value, field.label)}><Copy class="h-4 w-4" /></button>
        </div>
      </label>
    {/each}
  </div>
  <details class="text-[11px] text-fg-muted">
    <summary class="cursor-pointer">OBS 推流设置说明</summary>
    <div class="mt-2 space-y-1.5 leading-5">
      <p>在 OBS「设置 → 推流」中选择「自定义」，将推流地址填入服务器，并填写推流密钥。推荐 H.264 视频、AAC 音频，关键帧间隔 2 秒。</p>
      <p>其他电脑上的 OBS 请将地址中的主机替换为运行本程序的电脑 IP，并放行 TCP {info.port} 端口。Docker 需映射此端口。</p>
      <p>停用房间会断开推流；重新启用后请让 OBS 重连。推流密钥仅分享给需要推流的人。</p>
    </div>
  </details>
  {#if copyError || copied}<p class="text-[11px] text-fg-secondary" role="status">{copyError || `已复制${copied}`}</p>{/if}
</div>
