<script lang="ts">
  import { Bold, Italic, X } from "lucide-svelte";
  import type { SubtitleStyle } from "../interface";
  import {
    ASS_FIELD,
    ASS_STYLE_FORMAT,
    DEFAULT_SUBTITLE_STYLE,
    SUBTITLE_STYLE_PRESETS,
    applySubtitlePreset,
    assStyleFields,
    buildSubtitleAss,
    cssColor,
    loadSubtitleStyle,
    matchSubtitlePreset,
    parseAssStyle,
    saveSubtitleStyle,
    serializeAssStyle,
    setAssColor,
    updateAssFields,
    type SubtitleStylePreset,
  } from "../subtitle-ass";
  import NativeSubtitlePreview from "./NativeSubtitlePreview.svelte";
  import SubtitleFontSelect from "./SubtitleFontSelect.svelte";

  interface Props {
    show?: boolean;
    onClose: () => void;
    onSave?: (style: SubtitleStyle) => void;
    roomId: string;
    /** Edit this style instead of the room's saved one, e.g. a project's. */
    initialStyle?: SubtitleStyle | null;
  }

  let { show = $bindable(false), onClose, onSave, roomId, initialStyle = null }: Props = $props();
  let activeTab = $state<"basic" | "code">("basic");
  let sourceCode = $state(serializeAssStyle(DEFAULT_SUBTITLE_STYLE));
  let saveError = $state("");
  let fieldErrors = $state<Record<string, string>>({});
  let controlsVersion = $state(0);

  // All edits stay in this draft until the user explicitly applies them.
  $effect(() => {
    if (show) {
      sourceCode = serializeAssStyle(initialStyle ?? loadSubtitleStyle(roomId));
      activeTab = "basic";
      saveError = "";
      fieldErrors = {};
    }
  });

  let parsed = $derived.by(() => {
    try {
      return { style: parseAssStyle(sourceCode), error: "" };
    } catch (error) {
      return {
        style: null,
        error: error instanceof Error ? error.message : "ASS 样式代码无效",
      };
    }
  });

  let style = $derived(parsed.style ?? DEFAULT_SUBTITLE_STYLE);
  // Easy controls edit raw ASS fields, so they show actual script coordinates.
  let fields = $derived(assStyleFields(style));
  let activePreset = $derived(parsed.style ? matchSubtitlePreset(parsed.style)?.id : undefined);
  let boxed = $derived(fields[ASS_FIELD.BorderStyle] === "3");
  let previewAss = $derived(
    parsed.style
      ? buildSubtitleAss(
          [{ startTime: 0, endTime: 5, text: "这是一段示例字幕文本\nSubtitle preview · 123", layerOrder: 0 }],
          parsed.style,
        )
      : "",
  );

  // Numeric keypad layout, as in the ASS Alignment field.
  const positions = [
    { value: 7, label: "左上" },
    { value: 8, label: "顶部居中" },
    { value: 9, label: "右上" },
    { value: 4, label: "中间居左" },
    { value: 5, label: "正中" },
    { value: 6, label: "中间居右" },
    { value: 1, label: "左下" },
    { value: 2, label: "底部居中" },
    { value: 3, label: "右下" },
  ];
  let positionLabel = $derived(positions.find((position) => String(position.value) === fields[ASS_FIELD.Alignment])?.label);

  function resetMessages() {
    saveError = "";
    fieldErrors = {};
  }

  function setFields(patch: Record<number, string>, key: string) {
    if (!parsed.style) return;
    try {
      sourceCode = serializeAssStyle(updateAssFields(parsed.style, patch));
      delete fieldErrors[key];
      saveError = Object.values(fieldErrors)[0] ?? "";
    } catch (error) {
      saveError = error instanceof Error ? error.message : "无法更新样式";
      fieldErrors[key] = saveError;
    }
  }

  function setNumber(index: number, event: Event, integer = false) {
    const value = (event.currentTarget as HTMLInputElement).valueAsNumber;
    if (!Number.isFinite(value)) {
      saveError = "请输入有效的数值，不能为空";
      fieldErrors[index] = saveError;
      return;
    }
    setFields({ [index]: String(integer ? Math.round(value) : Math.round(value * 1000) / 1000) }, String(index));
  }

  function setColor(index: number, html: string) {
    setFields({ [index]: setAssColor(fields[index], html) }, String(index));
  }

  function toggle(index: number) {
    setFields({ [index]: fields[index] === "0" ? "-1" : "0" }, String(index));
  }

  function replaceDraft(next: SubtitleStyle) {
    sourceCode = serializeAssStyle(next);
    resetMessages();
    controlsVersion += 1;
  }

  // A small CSS approximation for preset buttons; the main preview uses native Skia.
  function swatchStyle(preset: SubtitleStylePreset): string {
    const values = ["", "", ...preset.fields.split(",")];
    const scale = 14 / Number(values[ASS_FIELD.Fontsize]);
    const outline = Number(values[ASS_FIELD.Outline]) * scale;
    const shadow = Number(values[ASS_FIELD.Shadow]) * scale;
    const outlineColor = cssColor(values[ASS_FIELD.OutlineColour]);
    const box = values[ASS_FIELD.BorderStyle] === "3";
    const shadows: string[] = [];
    if (outline > 0 && !box) {
      const width = Math.max(outline, 0.75);
      for (let step = 0; step < 8; step++) {
        const angle = (step * Math.PI) / 4;
        shadows.push(`${(Math.cos(angle) * width).toFixed(2)}px ${(Math.sin(angle) * width).toFixed(2)}px 0 ${outlineColor}`);
      }
    }
    if (shadow > 0) shadows.push(`${shadow.toFixed(2)}px ${shadow.toFixed(2)}px 0 ${cssColor(values[ASS_FIELD.BackColour])}`);
    return [
      `color: ${cssColor(values[ASS_FIELD.PrimaryColour])}`,
      `font-weight: ${values[ASS_FIELD.Bold] === "0" ? 400 : 700}`,
      `text-shadow: ${shadows.join(", ") || "none"}`,
      box ? `background: ${outlineColor}; padding: 0 ${outline.toFixed(2)}px; border-radius: 2px` : "",
    ].filter(Boolean).join("; ");
  }

  function swatchPlacement(preset: SubtitleStylePreset): string {
    const alignment = Number(preset.fields.split(",")[ASS_FIELD.Alignment - ASS_FIELD.Fontsize]);
    const vertical = alignment >= 7 ? "items-start" : alignment >= 4 ? "items-center" : "items-end";
    const horizontal = ["justify-end", "justify-start", "justify-center"][alignment % 3];
    return `${vertical} ${horizontal}`;
  }

  function cancel() {
    show = false;
    onClose();
  }

  function apply() {
    if (saveError) return;
    try {
      const savedStyle = parseAssStyle(sourceCode);
      saveSubtitleStyle(roomId, savedStyle);
      onSave?.(savedStyle);
      show = false;
      onClose();
    } catch (error) {
      saveError = error instanceof Error ? error.message : "字幕样式保存失败";
    }
  }
</script>

<svelte:window onkeydown={(event) => {
  if (show && event.key === "Escape") {
    event.preventDefault();
    event.stopImmediatePropagation();
    cancel();
  }
}} />

{#if show}
  <div class="fixed inset-0 z-[1100] flex items-center justify-center p-4">
    <button
      type="button"
      class="absolute inset-0 bg-black/60"
      aria-label="取消字幕样式设置"
      onclick={cancel}
    ></button>
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="subtitle-style-title"
      tabindex="-1"
      class="relative flex w-[760px] max-w-full max-h-[92vh] flex-col rounded-lg bg-[#1c1c1e] shadow-2xl"
    >
      <div class="flex shrink-0 items-center justify-between border-b border-gray-800/50 px-4 py-3">
        <h2 id="subtitle-style-title" class="text-base font-medium text-white">字幕压制样式设置</h2>
        <button type="button" class="text-gray-400 hover:text-white" aria-label="取消" onclick={cancel}>
          <X class="h-5 w-5" />
        </button>
      </div>

      <div class="sidebar-scrollbar space-y-4 overflow-y-auto px-4 py-3">
        <div class="space-y-1.5">
          <!-- Dark-to-light backdrop shows outlines, shadows and boxes on both scene tones. -->
          <div class="relative mx-auto aspect-video w-full max-w-[calc(36vh*16/9)] overflow-hidden rounded-lg bg-gradient-to-r from-slate-900 via-slate-500 to-slate-300">
            {#if previewAss}
              <NativeSubtitlePreview assContent={previewAss} fontName={style.fontName} />
            {:else}
              <div class="flex h-full items-center justify-center px-6 text-center text-sm text-gray-300">
                修正 ASS 样式代码后显示预览
              </div>
            {/if}
          </div>
          <p class="text-center text-[11px] text-gray-500">原生彩色字幕预览 · 1920 × 1080 · 与压制使用同一渲染器</p>
        </div>

        <div>
          <div class="mb-1.5 flex items-baseline justify-between">
            <h3 class="text-xs font-medium text-gray-300">预设样式</h3>
            <span class="text-[11px] text-gray-500">应用后保留当前字体</span>
          </div>
          <div class="grid grid-cols-4 gap-2 sm:grid-cols-8">
            {#each SUBTITLE_STYLE_PRESETS as preset (preset.id)}
              <button
                type="button"
                aria-pressed={activePreset === preset.id}
                class="rounded-md border p-1 text-left transition-colors {activePreset === preset.id ? 'border-[#0A84FF] bg-[#0A84FF]/10' : 'border-gray-700/60 hover:border-gray-500'}"
                onclick={() => replaceDraft(applySubtitlePreset(parsed.style ?? DEFAULT_SUBTITLE_STYLE, preset))}
              >
                <span class="flex h-9 overflow-hidden rounded bg-gradient-to-r from-slate-900 via-slate-500 to-slate-300 px-1 py-0.5 {swatchPlacement(preset)}" aria-hidden="true">
                  <span class="whitespace-nowrap text-[14px] leading-none" style={swatchStyle(preset)}>字幕 Aa</span>
                </span>
                <span class="mt-1 block truncate text-center text-[11px] text-gray-300">{preset.name}</span>
              </button>
            {/each}
          </div>
        </div>

        <div class="flex gap-1 border-b border-gray-800/50" role="tablist" aria-label="样式编辑方式">
          <button
            type="button"
            role="tab"
            id="subtitle-basic-tab"
            aria-controls="subtitle-basic-panel"
            aria-selected={activeTab === "basic"}
            class="border-b-2 px-3 py-1.5 text-sm {activeTab === 'basic' ? 'border-[#0A84FF] text-white' : 'border-transparent text-gray-400 hover:text-gray-200'}"
            onclick={() => (activeTab = "basic")}
          >常用设置</button>
          <button
            type="button"
            role="tab"
            id="subtitle-code-tab"
            aria-controls="subtitle-code-panel"
            aria-selected={activeTab === "code"}
            class="border-b-2 px-3 py-1.5 text-sm {activeTab === 'code' ? 'border-[#0A84FF] text-white' : 'border-transparent text-gray-400 hover:text-gray-200'}"
            onclick={() => (activeTab = "code")}
          >ASS 样式代码</button>
        </div>

        {#if activeTab === "basic"}
          <div id="subtitle-basic-panel" role="tabpanel" aria-labelledby="subtitle-basic-tab" class="space-y-2">
            {#key controlsVersion}
            <fieldset disabled={!parsed.style} class="grid grid-cols-4 gap-x-3 gap-y-2.5 disabled:opacity-50">
              <div class="col-span-2">
                <span class="field-label">字体</span>
                <SubtitleFontSelect
                  value={style.fontName}
                  disabled={!parsed.style}
                  onSelect={(family) => setFields({ [ASS_FIELD.Fontname]: family }, "font")}
                />
              </div>
              <label>
                <span class="field-label">字号</span>
                <input type="number" min="1" step="1" value={fields[ASS_FIELD.Fontsize]} oninput={(event) => setNumber(ASS_FIELD.Fontsize, event)} class="style-input" />
              </label>
              <div>
                <span class="field-label">字形</span>
                <div class="flex gap-1.5">
                  <button type="button" class="toggle" aria-pressed={fields[ASS_FIELD.Bold] !== "0"} aria-label="粗体" title="粗体" onclick={() => toggle(ASS_FIELD.Bold)}>
                    <Bold class="h-4 w-4" />
                  </button>
                  <button type="button" class="toggle" aria-pressed={fields[ASS_FIELD.Italic] !== "0"} aria-label="斜体" title="斜体" onclick={() => toggle(ASS_FIELD.Italic)}>
                    <Italic class="h-4 w-4" />
                  </button>
                </div>
              </div>

              <label class="color-field">
                <span class="field-label">文字颜色</span>
                <span class="style-input flex items-center gap-2">
                  <input type="color" value={style.fontColor} oninput={(event) => setColor(ASS_FIELD.PrimaryColour, event.currentTarget.value)} class="h-5 w-6 shrink-0 cursor-pointer rounded border-0 bg-transparent p-0" />
                  <span class="font-mono text-xs text-gray-300">{style.fontColor.toUpperCase()}</span>
                </span>
              </label>
              <label class="color-field">
                <span class="field-label">{boxed ? "底框颜色" : "描边颜色"}</span>
                <span class="style-input flex items-center gap-2">
                  <input type="color" value={style.outlineColor} oninput={(event) => setColor(ASS_FIELD.OutlineColour, event.currentTarget.value)} class="h-5 w-6 shrink-0 cursor-pointer rounded border-0 bg-transparent p-0" />
                  <span class="font-mono text-xs text-gray-300">{style.outlineColor.toUpperCase()}</span>
                </span>
              </label>
              <label>
                <span class="field-label">{boxed ? "底框边距" : "描边宽度"}</span>
                <input type="number" min="0" step="0.5" value={fields[ASS_FIELD.Outline]} oninput={(event) => setNumber(ASS_FIELD.Outline, event)} class="style-input" />
              </label>
              <label>
                <span class="field-label">阴影距离</span>
                <input type="number" min="0" step="0.5" value={fields[ASS_FIELD.Shadow]} oninput={(event) => setNumber(ASS_FIELD.Shadow, event)} class="style-input" />
              </label>

              <div>
                <span class="field-label">位置</span>
                <div class="flex items-center gap-2">
                  <div class="grid h-[32px] w-[56px] shrink-0 grid-cols-3 grid-rows-3 gap-[2px] rounded border border-gray-700 bg-black/40 p-[2px]" role="radiogroup" aria-label="字幕位置">
                    {#each positions as position (position.value)}
                      <button
                        type="button"
                        role="radio"
                        aria-checked={fields[ASS_FIELD.Alignment] === String(position.value)}
                        aria-label={position.label}
                        title={position.label}
                        class="rounded-[2px] {fields[ASS_FIELD.Alignment] === String(position.value) ? 'bg-[#0A84FF]' : 'bg-gray-600/70 hover:bg-gray-500'}"
                        onclick={() => setFields({ [ASS_FIELD.Alignment]: String(position.value) }, "alignment")}
                      ></button>
                    {/each}
                  </div>
                  <span class="truncate text-xs text-gray-400">{positionLabel}</span>
                </div>
              </div>
              <label>
                <span class="field-label">左边距</span>
                <input type="number" min="0" step="1" value={fields[ASS_FIELD.MarginL]} oninput={(event) => setNumber(ASS_FIELD.MarginL, event, true)} class="style-input" />
              </label>
              <label>
                <span class="field-label">右边距</span>
                <input type="number" min="0" step="1" value={fields[ASS_FIELD.MarginR]} oninput={(event) => setNumber(ASS_FIELD.MarginR, event, true)} class="style-input" />
              </label>
              <label>
                <span class="field-label">垂直边距</span>
                <input type="number" min="0" step="1" value={fields[ASS_FIELD.MarginV]} oninput={(event) => setNumber(ASS_FIELD.MarginV, event, true)} class="style-input" />
              </label>
            </fieldset>
            {/key}
            <p class="text-[11px] leading-relaxed text-gray-500">数值均为 1920 × 1080 画面坐标。透明度、缩放、字距、旋转等字段可在「ASS 样式代码」中修改，调整常用设置时会保留。</p>
          </div>
        {:else}
          <div id="subtitle-code-panel" role="tabpanel" aria-labelledby="subtitle-code-tab" class="space-y-2">
            <label for="subtitle-ass-code" class="block text-xs text-gray-400">编辑 [V4+ Styles] 中的 Format 和 Style 行</label>
            <textarea
              id="subtitle-ass-code"
              value={sourceCode}
              oninput={(event) => {
                sourceCode = event.currentTarget.value;
                resetMessages();
              }}
              rows="5"
              spellcheck="false"
              wrap="off"
              class="style-input min-h-[120px] resize-y font-mono text-xs leading-6"
              aria-invalid={Boolean(parsed.error)}
              aria-describedby="subtitle-ass-help"
            ></textarea>
            <p id="subtitle-ass-help" class="text-[11px] leading-relaxed text-gray-500">支持字体、透明度、粗体、斜体、缩放、字距、旋转、描边和阴影等标准 ASS 样式字段。只设置一个 Style；脚本分辨率固定为 1920 × 1080。</p>
            <details class="text-[11px] text-gray-500">
              <summary class="cursor-pointer hover:text-gray-300">查看标准字段顺序</summary>
              <code class="mt-2 block whitespace-pre-wrap break-words rounded bg-black/20 p-2">Format: {ASS_STYLE_FORMAT}</code>
            </details>
          </div>
        {/if}

        {#if parsed.error || saveError}
          <p role="alert" class="rounded-lg border border-red-500/20 bg-red-500/10 px-3 py-2 text-sm text-red-300">{parsed.error || saveError}</p>
        {/if}
      </div>

      <div class="flex shrink-0 items-center justify-between gap-3 border-t border-gray-800/50 px-4 py-3">
        <button
          type="button"
          class="text-sm text-gray-400 hover:text-white"
          onclick={() => replaceDraft(DEFAULT_SUBTITLE_STYLE)}
        >恢复默认</button>
        <div class="flex gap-2">
          <button type="button" onclick={cancel} class="rounded-lg bg-[#2c2c2e] px-4 py-1.5 text-sm text-gray-300 hover:bg-[#3c3c3e]">取消</button>
          <button type="button" onclick={apply} disabled={!parsed.style || Boolean(saveError)} class="rounded-lg bg-[#0A84FF] px-4 py-1.5 text-sm text-white hover:bg-blue-500 disabled:cursor-not-allowed disabled:opacity-40">保存并应用</button>
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .field-label {
    display: block;
    margin-bottom: 0.25rem;
    font-size: 0.75rem;
    line-height: 1rem;
    color: rgb(156 163 175);
  }

  .style-input {
    width: 100%;
    border: 1px solid rgb(55 65 81 / 0.6);
    border-radius: 0.375rem;
    background: #2c2c2e;
    padding: 0.3125rem 0.5rem;
    font-size: 0.8125rem;
    line-height: 1.25rem;
    color: white;
    outline: none;
  }

  .style-input:focus,
  .color-field:focus-within .style-input {
    border-color: #0a84ff;
  }

  .toggle {
    display: flex;
    height: 2rem;
    width: 2rem;
    align-items: center;
    justify-content: center;
    border: 1px solid rgb(55 65 81 / 0.6);
    border-radius: 0.375rem;
    background: #2c2c2e;
    color: rgb(156 163 175);
  }

  .toggle:hover {
    color: white;
  }

  .toggle[aria-pressed="true"] {
    border-color: #0a84ff;
    background: rgb(10 132 255 / 0.2);
    color: white;
  }
</style>
