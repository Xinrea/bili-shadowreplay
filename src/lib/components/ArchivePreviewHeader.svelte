<script lang="ts">
  import { ArrowLeft, Keyboard } from "lucide-svelte";
  import type { RecordItem } from "../db";

  interface Props {
    archive?: RecordItem | null;
    platform?: string | null;
    roomId?: string | null;
  }

  let { archive = null, platform = null, roomId = null }: Props = $props();

  function formatCreatedAt(value?: string) {
    if (!value) return "";
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return date.toLocaleString("zh-CN", {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    });
  }

  function formatDuration(seconds?: number) {
    if (!seconds || seconds < 0) return "";
    const total = Math.floor(seconds);
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const rest = total % 60;
    return [hours, minutes, rest]
      .map((part) => part.toString().padStart(2, "0"))
      .join(":");
  }

  function toggleShortcutHelp() {
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "h" }));
  }
</script>

<header class="preview-header">
  <button
    type="button"
    class="icon-button"
    title="关闭预览"
    aria-label="关闭预览"
    onclick={() => window.close()}
  >
    <ArrowLeft size={18} />
  </button>

  <div class="min-w-0 flex-1">
    <div class="flex items-center gap-2 text-[10px] uppercase tracking-[0.14em] text-sky-400">
      <span class="h-1.5 w-1.5 rounded-full bg-sky-400"></span>
      录播预览
    </div>
    <div class="mt-0.5 flex min-w-0 items-center gap-3">
      <h1 class="truncate text-[15px] font-semibold text-white">
        {archive?.title || `直播间 ${roomId || ""}`}
      </h1>
      <span class="platform-pill">{archive?.platform || platform || "未知平台"}</span>
      <span class="hidden truncate text-[11px] text-slate-400 lg:inline">
        {#if roomId}房间 {roomId}{/if}
        {#if archive?.created_at} · {formatCreatedAt(archive.created_at)}{/if}
        {#if archive?.length} · {formatDuration(archive.length)}{/if}
      </span>
    </div>
  </div>

  <button
    type="button"
    class="toolbar-button"
    title="显示快捷键说明（H）"
    onclick={toggleShortcutHelp}
  >
    <Keyboard size={15} />
    <span>快捷键</span>
  </button>
</header>

<style>
  .preview-header {
    display: flex;
    height: 58px;
    flex: 0 0 58px;
    align-items: center;
    gap: 12px;
    border-bottom: 1px solid #2b3340;
    background: #171c25;
    padding: 0 16px;
  }

  .icon-button,
  .toolbar-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid transparent;
    color: #dce5f3;
    transition:
      background-color 160ms ease,
      border-color 160ms ease;
  }

  .icon-button {
    height: 32px;
    width: 32px;
    flex: 0 0 32px;
    border-radius: 9px;
    background: #242c39;
  }

  .toolbar-button {
    height: 32px;
    gap: 7px;
    border-radius: 9px;
    background: #242c39;
    padding: 0 12px;
    font-size: 12px;
  }

  .icon-button:hover,
  .toolbar-button:hover {
    border-color: #3a4657;
    background: #2c3543;
  }

  .icon-button:focus-visible,
  .toolbar-button:focus-visible {
    outline: 2px solid #0a84ff;
    outline-offset: 2px;
  }

  .platform-pill {
    flex: 0 0 auto;
    border-radius: 999px;
    background: #242c39;
    padding: 3px 10px;
    font-size: 10px;
    color: #9da9ba;
    text-transform: capitalize;
  }
</style>
