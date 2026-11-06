<script lang="ts">
  import type { RenderResolution, RenderSettings } from "../interface";
  import { defaultRenderSettings } from "../project-timeline";

  interface Props {
    value?: RenderSettings;
    disabled?: boolean;
    streamCopy?: boolean;
    originalResolutionSource?: "first" | "largest";
    onChange?: () => void;
    layout?: "stacked" | "columns" | "rows";
    showHint?: boolean;
  }

  let { value = $bindable(defaultRenderSettings()), disabled = false, streamCopy = false, originalResolutionSource = "first", onChange, layout = "stacked", showHint = true }: Props = $props();
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
    ? "grid gap-x-4 gap-y-3 sm:grid-cols-2 disabled:opacity-70"
    : "space-y-3 disabled:opacity-70"}
  disabled={disabled || streamCopy}
  onchange={() => onChange?.()}
>
  <label class={labelClass}>
    <span class={captionClass}>分辨率</span>
    <select
      class="ui-input w-full"
      aria-label="分辨率"
      bind:value={() => streamCopy ? "original" : value.resolution, (resolution: RenderResolution) => { if (!streamCopy) value.resolution = resolution; }}
    >
      <option value="original">使用原分辨率</option>
      <option value="4k">4K（3840 × 2160）</option>
      <option value="2k">2K（2560 × 1440）</option>
      <option value="1080p">1080p（1920 × 1080）</option>
    </select>
  </label>
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
    <p class="ui-hint sm:col-span-2">
      {#if streamCopy}
        不重新编码时保留各片段原有分辨率、编码格式和码率，编码设置不生效。调整这些参数需开启重新编码。
      {:else}
        预设分辨率跟随首个素材的横竖屏方向；使用原分辨率时{originalResolutionSource === "largest" ? "按像素面积最大的片段尺寸输出（面积相同取首个）" : "按首个素材尺寸输出"}，其它片段等比缩放并补黑边。画质优先通常耗时更长，实际效果取决于编码器。码率为目标值，越高文件通常越大；H.264 的播放兼容性更广。
      {/if}
    </p>
  {/if}
</fieldset>
