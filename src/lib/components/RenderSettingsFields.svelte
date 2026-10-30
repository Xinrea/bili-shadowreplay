<script lang="ts">
  import { untrack } from "svelte";
  import type { AvailableVideoEncoder, RenderSettings } from "../interface";
  import { invoke } from "../invoker";
  import { defaultRenderSettings } from "../project-timeline";

  interface Props {
    value?: RenderSettings;
    disabled?: boolean;
    onChange?: () => void;
    layout?: "stacked" | "columns" | "rows";
    showHint?: boolean;
    nativeGpu?: boolean;
  }

  let { value = $bindable(defaultRenderSettings()), disabled = false, onChange, layout = "stacked", showHint = true, nativeGpu = false }: Props = $props();
  let encoders: AvailableVideoEncoder[] = $state([]);
  let loading = $state(true);
  let error = $state("");
  let sequence = 0;
  let codecEncoders = $derived(encoders.filter((item) => item.codec === value.codec));
  let selected = $derived(codecEncoders.find((item) =>
    (value.encoder === null || item.encoder === value.encoder) &&
    (value.render_backend === null || item.render_backend === value.render_backend)));
  let constantQuality = $derived(selected?.encoder.startsWith("libx26") && value.options.crf != null && value.options.crf !== "-1");
  let labelClass = $derived(layout === "rows"
    ? "grid grid-cols-[minmax(5rem,1fr)_minmax(0,min(55%,24rem))] items-center gap-4"
    : "block min-w-0 space-y-1.5");
  let captionClass = $derived(layout === "rows"
    ? "text-[12px] font-normal text-fg-secondary"
    : "ui-label");

  $effect(() => {
    const gpu = nativeGpu;
    const priority = value.priority;
    untrack(() => { void loadEncoders(gpu, priority); });
  });

  async function loadEncoders(gpu: boolean, priority: RenderSettings["priority"]) {
    const request = ++sequence;
    loading = true;
    error = "";
    encoders = [];
    try {
      const result = await invoke<AvailableVideoEncoder[]>("get_available_video_encoders", { nativeGpu: gpu, priority });
      if (request !== sequence) return;
      encoders = result;
    } catch (cause) {
      if (request === sequence) error = String(cause);
    } finally {
      if (request === sequence) loading = false;
    }
  }

  function encoderKey(encoder: AvailableVideoEncoder) {
    return `${encoder.render_backend}:${encoder.encoder}`;
  }

  function encoderLabel(encoder: AvailableVideoEncoder) {
    const backend = { cpu: "CPU", metal: "Metal", vulkan: "Vulkan" }[encoder.render_backend];
    const names: Record<string, string> = {
      libx264: "x264", libx265: "x265", nvenc: "NVENC", videotoolbox: "VideoToolbox",
      vulkan: "Vulkan", qsv: "QSV", amf: "AMF", mf: "Media Foundation",
      vaapi: "VAAPI", v4l2m2m: "V4L2 M2M",
    };
    const family = encoder.encoder.replace(/^(h264|hevc)_/, "");
    return `${backend}+${names[family] ?? encoder.encoder}`;
  }

  function selectCodec(codec: RenderSettings["codec"]) {
    const backend = value.render_backend ?? selected?.render_backend;
    const counterpart = value.encoder?.replace(/^(h264|hevc)_/, `${codec}_`)
      .replace(/^libx26[45]$/, codec === "h264" ? "libx264" : "libx265");
    const encoder = encoders.find((item) => item.codec === codec && item.render_backend === backend && item.encoder === counterpart)
      ?? encoders.find((item) => item.codec === codec && item.render_backend === backend)
      ?? encoders.find((item) => item.codec === codec);
    value = { ...value, codec, render_backend: encoder?.render_backend ?? null, encoder: encoder?.encoder ?? null, options: {} };
    onChange?.();
  }

  function selectEncoder(key: string) {
    const encoder = codecEncoders.find((item) => encoderKey(item) === key);
    value = { ...value, render_backend: encoder?.render_backend ?? null, encoder: encoder?.encoder ?? null, options: {} };
    onChange?.();
  }

  function setOption(name: string, option: string) {
    value = { ...value, render_backend: selected?.render_backend ?? null, encoder: selected?.encoder ?? null, options: { ...value.options, [name]: option } };
    onChange?.();
  }
</script>

<fieldset
  class={layout === "columns"
    ? "grid gap-x-4 gap-y-3 sm:grid-cols-2 disabled:opacity-70"
    : "space-y-3 disabled:opacity-70"}
  disabled={disabled || loading}
>
  <label class={labelClass}>
    <span class={captionClass}>编码格式</span>
    <select class="ui-input w-full" aria-label="编码格式" value={value.codec}
      onchange={(event) => selectCodec(event.currentTarget.value as RenderSettings["codec"])}>
      <option value="h264" disabled={!encoders.some((item) => item.codec === "h264")}>H.264 (AVC)</option>
      <option value="hevc" disabled={!encoders.some((item) => item.codec === "hevc")}>H.265 (HEVC)</option>
    </select>
  </label>
  <label class={labelClass}>
    <span class={captionClass}>渲染与编码</span>
    <select class="ui-input w-full" aria-label="渲染与编码" value={value.encoder ? (selected ? encoderKey(selected) : value.encoder) : ""}
      onchange={(event) => selectEncoder(event.currentTarget.value)}>
      <option value="" disabled={codecEncoders.length === 0}>{codecEncoders.length ? `自动（${encoderLabel(codecEncoders[0])}）` : loading ? "探测编码器中…" : "无可用编码器"}</option>
      {#if value.encoder && !selected}<option value={value.encoder} disabled>请选择可用编码器</option>{/if}
      {#each codecEncoders as encoder (encoderKey(encoder))}
        <option value={encoderKey(encoder)}>{encoderLabel(encoder)}</option>
      {/each}
    </select>
  </label>
  {#if selected && selected.options.length === 0}
    <label class={labelClass}>
      <span class={captionClass}>编码偏好</span>
      <select class="ui-input w-full" aria-label="编码偏好" value={value.priority}
        onchange={(event) => { value = { ...value, priority: event.currentTarget.value as RenderSettings["priority"], options: {} }; onChange?.(); }}>
        <option value="quality">画质优先</option>
        <option value="speed">速度优先</option>
      </select>
    </label>
  {/if}
  <label class={labelClass}>
    <span class={captionClass}>视频码率</span>
    <span class="flex min-w-0 items-center gap-2">
      <input type="number" class="ui-input min-w-0 w-full" aria-label="视频码率"
        value={value.bitrate_kbps / 1000} min="1" max="100" step="1"
        onchange={(event) => { value = { ...value, bitrate_kbps: Math.round(event.currentTarget.valueAsNumber * 1000) }; onChange?.(); }} />
      <span class="ui-hint shrink-0">Mbps</span>
    </span>
  </label>
  {#each selected?.options ?? [] as option (option.name)}
    <label class={labelClass}>
      <span class={captionClass} title={option.hint}>{option.label}</span>
      {#if option.range}
        <input type="number" class="ui-input w-full" aria-label={option.label} title={option.hint}
          value={value.options[option.name] ?? option.default} min={option.range[0]} max={option.range[1]} step="1"
          onchange={(event) => setOption(option.name, event.currentTarget.value)} />
      {:else}
        <select class="ui-input w-full" aria-label={option.label} title={option.hint}
          value={value.options[option.name] ?? option.default}
          onchange={(event) => setOption(option.name, event.currentTarget.value)}>
          {#each option.choices as [choice, label]}<option value={choice}>{label}</option>{/each}
        </select>
      {/if}
    </label>
  {/each}
  {#if showHint}
    <p class="ui-hint sm:col-span-2">
      仅显示当前导出可用的组合。较慢的预设通常提高压缩效率；
      {#if constantQuality}
        CRF 按质量编码，最大码率为所设码率的 1.25 倍，缓冲区大小为所设码率的 2 倍。
      {:else}
        码率为目标值。
      {/if}
    </p>
  {/if}
</fieldset>
{#if error}
  <p class="mt-2 text-xs text-red-500" role="alert">{error}</p>
  <button type="button" class="ui-btn ui-btn-sm mt-2" onclick={() => loadEncoders(nativeGpu, value.priority)}>重新检测</button>
{:else if !loading && !selected}
  <p class="mt-2 text-xs text-red-500" role="alert">所选编码器当前不可用，请重新选择。</p>
{/if}
