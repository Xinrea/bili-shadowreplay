<script lang="ts">
  import { Plus } from "lucide-svelte";
  import { onMount } from "svelte";
  import { fontFamily, sameStyle, styleOf, TEXT_STYLES, type TextElement, type TextStyle } from "../../cover-elements";
  import { drawText, FONT_PICKS, fontAvailable } from "./render";

  interface Props {
    /** The selected text, which the styles and fonts change. */
    selected: TextElement | null;
    onAdd: () => void;
    onStyle: (style: TextStyle) => void;
    onFont: (family: string) => void;
  }

  let { selected, onAdd, onStyle, onFont }: Props = $props();
  let tiles: HTMLCanvasElement[] = $state([]);
  let fonts: { label: string; family: string }[] = $state([]);

  onMount(() => {
    fonts = [
      { label: "黑体", family: "" },
      ...FONT_PICKS.map((pick) => ({ label: pick.label, family: pick.families.find(fontAvailable) ?? null })).filter(
        (pick) => pick.family !== null,
      ),
    ];
    TEXT_STYLES.forEach((style, index) => {
      const canvas = tiles[index];
      const ctx = canvas.getContext("2d");
      drawText(ctx, {
        kind: "text",
        id: 0,
        x: canvas.width / 2,
        y: canvas.height / 2,
        rotation: 0,
        content: style.label,
        font: "",
        fontSize: 44,
        bold: true,
        align: "center",
        ...styleOf(style),
      });
    });
  });
</script>

<div class="space-y-4">
  <button type="button" class="ui-btn h-9 w-full text-[13px]" onclick={onAdd}>
    <Plus class="h-4 w-4" />
    添加文字
  </button>

  <section class="space-y-2">
    <h4 class="text-[12px] font-medium text-fg-secondary">花字样式</h4>
    <div class="grid grid-cols-2 gap-2">
      {#each TEXT_STYLES as style, index (style.id)}
        <button
          type="button"
          class="tile h-[54px] {selected && sameStyle(selected, style) ? 'tile-active' : ''}"
          title={selected ? `将选中文字改为「${style.label}」` : `添加「${style.label}」文字`}
          onclick={() => onStyle(style)}
        >
          <canvas bind:this={tiles[index]} width="224" height="108" class="h-full w-full"></canvas>
        </button>
      {/each}
    </div>
  </section>

  <section class="space-y-2">
    <h4 class="text-[12px] font-medium text-fg-secondary">字体</h4>
    <div class="grid grid-cols-2 gap-2">
      {#each fonts as font (font.label)}
        <button
          type="button"
          class="tile h-[46px] text-[20px] font-bold text-white {selected?.font === font.family ? 'tile-active' : ''}"
          style:font-family={fontFamily(font.family)}
          title={font.family || "默认字体"}
          onclick={() => onFont(font.family)}
        >
          {font.label}
        </button>
      {/each}
    </div>
    <p class="ui-hint">只列出本机已安装的字体，工具栏中可选择全部字体。</p>
  </section>
</div>

<style>
  .tile {
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    border-radius: 8px;
    border: 2px solid transparent;
    background: #8a93a6;
    transition:
      border-color 0.15s,
      filter 0.15s;
  }

  .tile:hover {
    filter: brightness(1.08);
  }

  .tile:focus-visible {
    outline: 2px solid rgb(var(--c-accent));
    outline-offset: 2px;
  }

  .tile-active {
    border-color: rgb(var(--c-accent));
  }
</style>
