<script lang="ts" module>
  import type { AudioItem, TimelineClip, VideoOverlay } from "../../interface";

  /** What the inspector shows the properties of, with where it plays in project time. */
  export type InspectedItem =
    | { kind: "clip"; item: TimelineClip; start: number; end: number }
    /** `aspect` is the material's width over height, null until known. */
    | { kind: "overlay"; item: VideoOverlay; start: number; end: number; aspect: number | null }
    | { kind: "audio"; item: AudioItem; start: number; end: number };
</script>

<script lang="ts">
  import { Crosshair, Film, Maximize, Music, PictureInPicture2, Volume2, VolumeX } from "lucide-svelte";
  import { clamp, ELEMENT_MIDDLE, ELEMENT_SPACE } from "../../cover-elements";
  import type { VideoPlacement } from "../../interface";
  import { formatTimecode } from "../../time-format";

  // The properties of the clip, picture in picture or sound selected on the
  // timeline. Sliders change it as they move, a run of changes being one undo
  // step; typed values apply when entered.

  interface Props {
    inspected: InspectedItem;
    /** The material or audio file it plays. */
    name: string;
    onClipVolume: (volume: number) => void;
    onOverlay: (update: (item: VideoOverlay) => VideoOverlay) => void;
    onAudio: (update: (item: AudioItem) => AudioItem) => void;
    /** Close a run of changes as one undo step. */
    onSettle: () => void;
    onFillFrame: () => void;
  }

  let { inspected, name, onClipVolume, onOverlay, onAudio, onSettle, onFillFrame }: Props = $props();

  /** Narrowest a picture in picture is made, in pixels of the 1920×1080 picture. */
  const MIN_WIDTH = 16;
  const MAX_WIDTH = ELEMENT_SPACE.width * 4;

  const KINDS = {
    clip: { label: "主轨片段", icon: Film },
    overlay: { label: "画中画", icon: PictureInPicture2 },
    audio: { label: "音频", icon: Music },
  } as const;

  let kind = $derived(KINDS[inspected.kind]);

  function place(changes: Partial<VideoPlacement>) {
    onOverlay((item) => ({ ...item, element: { ...item.element, ...changes } }));
  }

  function setOpacity(opacity: number) {
    onOverlay((item) => {
      const { opacity: _, ...rest } = item;
      return opacity >= 1 ? rest : { ...rest, opacity };
    });
  }

  function setVolume(volume: number) {
    if (inspected.kind === "clip") onClipVolume(volume);
    else if (inspected.kind === "overlay") onOverlay((item) => ({ ...item, volume }));
    else onAudio((item) => ({ ...item, volume }));
  }

  /** Set a fade of the sound, in seconds; none at 0. */
  function setFade(edge: "fade_in" | "fade_out", seconds: number) {
    onAudio((item) => {
      const next = { ...item };
      if (seconds > 0) next[edge] = Math.round(seconds * 10) / 10;
      else delete next[edge];
      return next;
    });
  }

  /**
   * Apply a typed number as one undo step. The field then shows what was
   * applied, also when that is the value it had before.
   */
  function commitNumber(event: Event & { currentTarget: HTMLInputElement }, current: number, apply: (value: number) => number) {
    const typed = event.currentTarget.valueAsNumber;
    const applied = Number.isFinite(typed) ? apply(typed) : current;
    onSettle();
    event.currentTarget.value = String(applied);
  }

  let volume = $derived(inspected.kind === "clip" ? (inspected.item.volume ?? 1) : inspected.item.volume);
</script>

{#snippet numberField(label: string, value: number, unit: string, apply: (value: number) => number, step = 1)}
  <label class="number-field">
    <span class="number-label">{label}</span>
    <input
      type="number"
      {step}
      {value}
      onchange={(event) => commitNumber(event, value, apply)}
      onkeydown={(event) => {
        if (event.key === "Enter") event.currentTarget.blur();
      }}
    />
    {#if unit}<span class="number-unit">{unit}</span>{/if}
  </label>
{/snippet}

<!-- A slider of `min`–`max` beside the same value typed, both in the field's unit. -->
{#snippet sliderRow(
  label: string,
  value: number,
  [min, max, step]: [number, number, number],
  unit: string,
  set: (value: number) => void,
)}
  <div class="slider-row">
    <span class="row-label">{label}</span>
    <input
      type="range"
      {min}
      {max}
      {step}
      {value}
      aria-label={label}
      oninput={(event) => set(event.currentTarget.valueAsNumber)}
      onchange={onSettle}
    />
    <label class="number-field compact">
      <input
        type="number"
        {step}
        value={Math.round(value)}
        aria-label={label}
        onchange={(event) =>
          commitNumber(event, Math.round(value), (typed) => {
            const applied = clamp(Math.round(typed), min, max);
            set(applied);
            return applied;
          })}
        onkeydown={(event) => {
          if (event.key === "Enter") event.currentTarget.blur();
        }}
      />
      <span class="number-unit">{unit}</span>
    </label>
  </div>
{/snippet}

{#snippet volumeRow(label: string)}
  <div class="slider-row">
    <button
      type="button"
      class="mute-button"
      title={volume > 0 ? "静音" : "取消静音"}
      aria-label={volume > 0 ? "静音" : "取消静音"}
      onclick={() => {
        setVolume(volume > 0 ? 0 : 1);
        onSettle();
      }}
    >
      {#if volume > 0}
        <Volume2 class="h-3.5 w-3.5" />
      {:else}
        <VolumeX class="h-3.5 w-3.5" />
      {/if}
    </button>
    <input
      type="range"
      min="0"
      max="100"
      step="1"
      value={volume * 100}
      aria-label={label}
      oninput={(event) => setVolume(event.currentTarget.valueAsNumber / 100)}
      onchange={onSettle}
    />
    <label class="number-field compact">
      <input
        type="number"
        step="1"
        value={Math.round(volume * 100)}
        aria-label={label}
        onchange={(event) =>
          commitNumber(event, Math.round(volume * 100), (typed) => {
            const applied = clamp(Math.round(typed), 0, 100);
            setVolume(applied / 100);
            return applied;
          })}
        onkeydown={(event) => {
          if (event.key === "Enter") event.currentTarget.blur();
        }}
      />
      <span class="number-unit">%</span>
    </label>
  </div>
{/snippet}

<header class="item-header">
  <span class="item-icon"><kind.icon class="h-4 w-4" /></span>
  <div class="min-w-0">
    <strong title={name}>{name}</strong>
    <span>
      {kind.label} · {formatTimecode(inspected.start)} – {formatTimecode(inspected.end)}
      · {(inspected.end - inspected.start).toFixed(1)} 秒
    </span>
  </div>
</header>

{#if inspected.kind === "overlay"}
  {@const element = inspected.item.element}
  {@const aspect = inspected.aspect}
  <section class="group">
    <h3>画面</h3>
    {@render sliderRow("不透明度", (inspected.item.opacity ?? 1) * 100, [0, 100, 1], "%", (value) =>
      setOpacity(value / 100),
    )}
    {@render sliderRow("缩放", (element.width / ELEMENT_SPACE.width) * 100, [5, 200, 1], "%", (value) =>
      place({ width: Math.round((ELEMENT_SPACE.width * value) / 100) }),
    )}
    {@render sliderRow("旋转", element.rotation, [-180, 180, 1], "°", (value) => place({ rotation: value }))}
  </section>

  <section class="group">
    <h3>大小</h3>
    <div class="field-grid">
      {@render numberField("宽", Math.round(element.width), "px", (typed) => {
        const width = Math.round(clamp(typed, MIN_WIDTH, MAX_WIDTH));
        place({ width });
        return width;
      })}
      {#if aspect}
        {@render numberField("高", Math.round(element.width / aspect), "px", (typed) => {
          const width = Math.round(clamp(typed * aspect, MIN_WIDTH, MAX_WIDTH));
          place({ width });
          return Math.round(width / aspect);
        })}
      {:else}
        <!-- Not guessed: a height typed against a wrong ratio would set a wrong width. -->
        <label class="number-field disabled" title="素材的宽高比还未读到">
          <span class="number-label">高</span>
          <input type="text" value="—" disabled />
          <span class="number-unit">px</span>
        </label>
      {/if}
    </div>
    <p class="hint">{aspect ? "高度随素材的宽高比变化。" : "素材的宽高比还未读到，暂不能按高度调整。"}</p>
  </section>

  <section class="group">
    <h3>位置</h3>
    <div class="field-grid">
      {@render numberField("X", Math.round(element.x), "px", (typed) => {
        const x = Math.round(clamp(typed, 0, ELEMENT_SPACE.width));
        place({ x });
        return x;
      })}
      {@render numberField("Y", Math.round(element.y), "px", (typed) => {
        const y = Math.round(clamp(typed, 0, ELEMENT_SPACE.height));
        place({ y });
        return y;
      })}
    </div>
    <p class="hint">画中画中心在 {ELEMENT_SPACE.width}×{ELEMENT_SPACE.height} 画面上的位置。</p>
    <div class="button-row">
      <button
        type="button"
        class="tool-button"
        onclick={() => {
          place({ ...ELEMENT_MIDDLE });
          onSettle();
        }}
      >
        <Crosshair class="h-3.5 w-3.5" />
        居中
      </button>
      <button type="button" class="tool-button" onclick={onFillFrame}>
        <Maximize class="h-3.5 w-3.5" />
        铺满画面
      </button>
      <button
        type="button"
        class="tool-button"
        disabled={element.rotation === 0}
        onclick={() => {
          place({ rotation: 0 });
          onSettle();
        }}
      >
        取消旋转
      </button>
    </div>
  </section>

  <section class="group">
    <h3>声音</h3>
    {@render volumeRow("画中画音量")}
  </section>
{:else if inspected.kind === "clip"}
  {@const clip = inspected.item}
  <section class="group">
    <h3>素材范围</h3>
    <dl class="info-grid">
      <dt>入点</dt>
      <dd>{formatTimecode(clip.start, 2)}</dd>
      <dt>出点</dt>
      <dd>{formatTimecode(clip.end, 2)}</dd>
      <dt>时长</dt>
      <dd>{formatTimecode(clip.end - clip.start, 2)}</dd>
    </dl>
    <p class="hint">在时间轴上拖动片段两端裁剪。</p>
  </section>

  <section class="group">
    <h3>声音</h3>
    {@render volumeRow("原声音量")}
  </section>

  <p class="hint">主轨片段铺满整个画面。把它向上拖到其他视频轨道上作为画中画，即可调整它的大小、位置和不透明度。</p>
{:else}
  {@const sound = inspected.item}
  <section class="group">
    <h3>声音</h3>
    {@render volumeRow("音量")}
  </section>

  <section class="group">
    <h3>淡入淡出</h3>
    <div class="field-grid">
      {@render numberField(
        "淡入",
        sound.fade_in ?? 0,
        "秒",
        (typed) => {
          const seconds = Math.max(0, Math.round(typed * 10) / 10);
          setFade("fade_in", seconds);
          return seconds;
        },
        0.1,
      )}
      {@render numberField(
        "淡出",
        sound.fade_out ?? 0,
        "秒",
        (typed) => {
          const seconds = Math.max(0, Math.round(typed * 10) / 10);
          setFade("fade_out", seconds);
          return seconds;
        },
        0.1,
      )}
    </div>
    <p class="hint">{sound.kind === "file" ? "来自音频库" : "素材的声音"}，在时间轴上拖动两端裁剪。</p>
  </section>
{/if}

<style>
  .item-header {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 14px;
    border: 1px solid #293442;
    border-radius: 10px;
    background: #202733;
    padding: 10px 12px;
  }

  .item-icon {
    display: inline-flex;
    width: 30px;
    height: 30px;
    flex-shrink: 0;
    align-items: center;
    justify-content: center;
    border-radius: 8px;
    background: rgb(10 132 255 / 16%);
    color: #55b9ff;
  }

  .item-header strong {
    display: block;
    overflow: hidden;
    color: #e9eef8;
    font-size: 12px;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .item-header span:not(.item-icon) {
    display: block;
    margin-top: 2px;
    color: #7f8b9d;
    font-size: 10px;
    font-variant-numeric: tabular-nums;
  }

  .group {
    display: flex;
    flex-direction: column;
    gap: 10px;
    border-bottom: 1px solid #222a35;
    padding: 4px 0 14px;
    margin-bottom: 12px;
  }

  .group:last-of-type {
    border-bottom: 0;
  }

  .group h3 {
    margin: 0;
    color: #a8b3c3;
    font-size: 11px;
    font-weight: 600;
  }

  .slider-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .slider-row input[type="range"] {
    min-width: 0;
    flex: 1;
  }

  .row-label {
    width: 52px;
    flex-shrink: 0;
    color: #8c98aa;
    font-size: 11px;
  }

  .mute-button {
    display: inline-flex;
    width: 52px;
    flex-shrink: 0;
    color: #8c98aa;
  }

  .mute-button:hover {
    color: #edf4ff;
  }

  .field-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }

  .number-field {
    display: flex;
    height: 30px;
    min-width: 0;
    align-items: center;
    gap: 6px;
    border: 1px solid #293442;
    border-radius: 8px;
    background: #0f131a;
    padding: 0 8px;
    transition: border-color 150ms ease;
  }

  .number-field:hover {
    border-color: #3a4658;
  }

  .number-field:focus-within {
    border-color: #0a84ff;
  }

  .number-field.disabled {
    opacity: 0.5;
  }

  .number-field.compact {
    width: 74px;
    flex-shrink: 0;
  }

  .number-field input {
    min-width: 0;
    flex: 1;
    border: 0;
    background: transparent;
    padding: 0;
    color: #e9eef8;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    outline: none;
    box-shadow: none;
    -moz-appearance: textfield;
    appearance: textfield;
  }

  .number-field input::-webkit-inner-spin-button,
  .number-field input::-webkit-outer-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }

  .number-label {
    color: #7f8b9d;
    font-size: 11px;
  }

  .number-unit {
    color: #5d6878;
    font-size: 10px;
  }

  .button-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .info-grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 12px;
    margin: 0;
    font-size: 11px;
  }

  .info-grid dt {
    color: #7f8b9d;
  }

  .info-grid dd {
    margin: 0;
    color: #dce5f3;
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
</style>
