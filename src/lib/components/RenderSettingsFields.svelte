<script lang="ts">
  import type { RenderSettings } from "../interface";
  import { defaultRenderSettings } from "../project-timeline";

  interface Props {
    value?: RenderSettings;
    disabled?: boolean;
    onChange?: () => void;
  }

  let { value = $bindable(defaultRenderSettings()), disabled = false, onChange }: Props = $props();
  const bitrates = [2000, 4000, 6000, 8000, 12000, 16000, 20000, 30000, 50000];
  // Existing settings already identify the mode: zero uses the quality preset.
  let mode = $derived(value.bitrate_kbps > 0 ? "bitrate" : "quality");

  function changeMode(next: string) {
    value = next === "bitrate"
      ? { quality: "balanced", bitrate_kbps: 8000 }
      : defaultRenderSettings();
  }
</script>

<fieldset class="space-y-3 disabled:opacity-70" {disabled} onchange={() => onChange?.()}>
  <label class="block space-y-1.5">
    <span class="ui-label">编码模式</span>
    <select class="ui-input w-full" aria-label="编码模式" value={mode} onchange={(event) => changeMode(event.currentTarget.value)}>
      <option value="quality">按画质</option>
      <option value="bitrate">按码率</option>
    </select>
  </label>
  {#if mode === "quality"}
    <label class="block space-y-1.5">
      <span class="ui-label">画质</span>
      <select class="ui-input w-full" aria-label="画质" bind:value={value.quality}>
        <option value="high">高画质</option>
        <option value="balanced">均衡（默认）</option>
        <option value="compact">较小体积</option>
      </select>
    </label>
    <p class="ui-hint">根据画质档位自动设置码率。画质越高，文件通常越大。</p>
  {:else}
    <label class="block space-y-1.5">
      <span class="ui-label">视频码率</span>
      <select class="ui-input w-full" aria-label="视频码率" bind:value={value.bitrate_kbps}>
        {#each bitrates as bitrate}
          <option value={bitrate}>{bitrate / 1000} Mbps</option>
        {/each}
        {#if !bitrates.includes(value.bitrate_kbps)}
          <option value={value.bitrate_kbps}>{value.bitrate_kbps / 1000} Mbps</option>
        {/if}
      </select>
    </label>
    <p class="ui-hint">按所选目标码率编码。码率越高，文件通常越大。</p>
  {/if}
</fieldset>
