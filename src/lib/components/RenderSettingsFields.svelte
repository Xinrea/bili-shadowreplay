<script lang="ts">
  import type { RenderSettings } from "../interface";
  import { defaultRenderSettings } from "../project-timeline";

  interface Props {
    value?: RenderSettings;
    disabled?: boolean;
    onChange?: () => void;
    layout?: "stacked" | "columns" | "rows";
    showHint?: boolean;
  }

  let { value = $bindable(defaultRenderSettings()), disabled = false, onChange, layout = "stacked", showHint = true }: Props = $props();
  const bitrates = [2000, 4000, 6000, 8000, 10000, 12000, 16000, 20000, 30000, 50000];
  let labelClass = $derived(layout === "rows"
    ? "grid grid-cols-[minmax(5rem,1fr)_minmax(0,min(55%,24rem))] items-center gap-4"
    : "block space-y-1.5");
  let captionClass = $derived(layout === "rows"
    ? "text-[12px] font-normal text-fg-secondary"
    : "ui-label");
</script>

<fieldset
  class={layout === "columns"
    ? "grid gap-x-4 gap-y-3 sm:grid-cols-3 disabled:opacity-70"
    : "space-y-3 disabled:opacity-70"}
  {disabled}
  onchange={() => onChange?.()}
>
  <label class={labelClass}>
    <span class={captionClass}>编码格式</span>
    <select class="ui-input w-full" aria-label="编码格式" bind:value={value.codec}>
      <option value="h264">H.264 (AVC)</option>
      <option value="hevc">H.265 (HEVC)</option>
    </select>
  </label>
  <label class={labelClass}>
    <span class={captionClass}>编码偏好</span>
    <select class="ui-input w-full" aria-label="编码偏好" bind:value={value.priority}>
      <option value="quality">画质优先</option>
      <option value="speed">速度优先</option>
    </select>
  </label>
  <label class={labelClass}>
    <span class={captionClass}>视频码率</span>
    <select class="ui-input w-full" aria-label="视频码率" bind:value={value.bitrate_kbps}>
      {#each bitrates as bitrate}
        <option value={bitrate}>{bitrate / 1000} Mbps</option>
      {/each}
      {#if !bitrates.includes(value.bitrate_kbps)}
        <option value={value.bitrate_kbps}>{value.bitrate_kbps / 1000} Mbps</option>
      {/if}
    </select>
  </label>
  {#if showHint}
    <p class="ui-hint sm:col-span-3">画质优先通常耗时更长，实际效果取决于编码器。码率为目标值，越高文件通常越大；H.264 的播放兼容性更广。</p>
  {/if}
</fieldset>
