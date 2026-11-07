<script lang="ts">
  import { AlignCenter, AlignLeft, AlignRight, Bold, FlipHorizontal2 } from "lucide-svelte";
  import { clamp, type ElementChanges, type StickerElement, type TextElement } from "../../cover-elements";
  import ColorPicker from "./ColorPicker.svelte";
  import CoverFontSelect from "./CoverFontSelect.svelte";

  // Controls for the selected text or sticker, shared by the cover and
  // project editors.

  interface Props {
    text?: TextElement | null;
    sticker?: StickerElement | null;
    /** A change that may come in a quick run, like dragging a color. */
    onUpdate: (changes: ElementChanges) => void;
    /** A font picked from the list. */
    onFont: (font: string) => void;
  }

  let { text = null, sticker = null, onUpdate, onFont }: Props = $props();

  const ALIGNMENTS = [
    { value: "left", label: "左对齐", icon: AlignLeft },
    { value: "center", label: "居中", icon: AlignCenter },
    { value: "right", label: "右对齐", icon: AlignRight },
  ] as const;
</script>

{#if text}
  <CoverFontSelect value={text.font} onSelect={onFont} />
  <input
    type="number"
    min="12"
    max="800"
    value={text.fontSize}
    class="ui-input h-[26px] w-[66px] pr-1 tabular-nums"
    title="字号"
    aria-label="字号"
    onchange={(event) => {
      const size = Math.round(Number(event.currentTarget.value));
      onUpdate({ fontSize: clamp(size || text.fontSize, 12, 800) });
      event.currentTarget.value = String(text.fontSize);
    }}
  />
  <span class="toolbar-separator"></span>
  <button
    type="button"
    class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost {text.bold ? 'ui-btn-active' : ''}"
    title="加粗"
    aria-label="加粗"
    aria-pressed={text.bold}
    onclick={() => onUpdate({ bold: !text.bold })}
  >
    <Bold class="h-3.5 w-3.5" />
  </button>
  <ColorPicker kind="fill" label="文字颜色" value={text.fill} onchange={(fill) => onUpdate({ fill })} />
  <ColorPicker kind="stroke" label="描边" allowNone value={text.stroke} onchange={(stroke) => onUpdate({ stroke })}>
    <label class="flex items-center gap-2 text-[11px] text-fg-muted">
      粗细
      <input
        type="range"
        min="0.02"
        max="0.25"
        step="0.01"
        value={text.strokeWidth}
        disabled={!text.stroke}
        class="min-w-0 flex-1"
        oninput={(event) => onUpdate({ strokeWidth: Number(event.currentTarget.value) })}
      />
    </label>
  </ColorPicker>
  <ColorPicker
    kind="background"
    label="背景色"
    allowNone
    value={text.background}
    onchange={(background) => onUpdate({ background })}
  />
  <button
    type="button"
    class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost {text.shadow ? 'ui-btn-active' : ''}"
    title="阴影"
    aria-label="阴影"
    aria-pressed={text.shadow}
    onclick={() => onUpdate({ shadow: !text.shadow })}
  >
    <span class="shadow-glyph text-[13px] font-bold leading-none">A</span>
  </button>
  {@const alignment = ALIGNMENTS.find((a) => a.value === text.align) ?? ALIGNMENTS[1]}
  {@const multiline = text.content.includes("\n")}
  <button
    type="button"
    class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost"
    title={multiline ? `对齐：${alignment.label}` : "多行文字可设置对齐"}
    aria-label="对齐方式"
    disabled={!multiline}
    onclick={() => onUpdate({ align: ALIGNMENTS[(ALIGNMENTS.indexOf(alignment) + 1) % ALIGNMENTS.length].value })}
  >
    <alignment.icon class="h-3.5 w-3.5" />
  </button>
{:else if sticker}
  <button
    type="button"
    class="ui-btn ui-btn-sm {sticker.flip ? 'ui-btn-active' : ''}"
    aria-pressed={sticker.flip}
    onclick={() => onUpdate({ flip: !sticker.flip })}
  >
    <FlipHorizontal2 class="h-3.5 w-3.5" />
    水平翻转
  </button>
{/if}

<style>
  .toolbar-separator {
    margin: 0 4px;
    height: 16px;
    width: 1px;
    background: rgb(var(--c-line-strong));
  }

  .shadow-glyph {
    text-shadow: 1.5px 1.5px 1px rgb(var(--c-fg-faint));
  }

  input[type="range"] {
    -webkit-appearance: none;
    appearance: none;
    height: 4px;
    border-radius: 2px;
    background: rgb(var(--c-fg-faint) / 50%);
    cursor: pointer;
  }

  input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: white;
    box-shadow: 0 1px 3px rgb(0 0 0 / 35%);
  }

  input[type="range"]::-moz-range-thumb {
    width: 12px;
    height: 12px;
    border: none;
    border-radius: 50%;
    background: white;
  }

  input[type="range"]:focus {
    outline: none;
  }

  input[type="range"]:focus-visible {
    outline: 2px solid rgb(var(--c-accent));
    outline-offset: 4px;
  }

  input[type="range"]:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }
</style>
