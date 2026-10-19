<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    value: string | null;
    label: string;
    /** How the button shows the color: text, outline or box. */
    kind: "fill" | "stroke" | "background";
    allowNone?: boolean;
    onchange: (value: string | null) => void;
    /** More controls under the colors, like the outline width. */
    children?: Snippet;
  }

  let { value, label, kind, allowNone = false, onchange, children }: Props = $props();
  let open = $state(false);
  let root: HTMLDivElement = $state();

  const SWATCHES = [
    "#FFFFFF", "#1A1A1A", "#8E8E93", "#FFE14D", "#FFB800", "#FF7F00", "#FF3B30",
    "#FF4F9A", "#AF52DE", "#5856D6", "#0A84FF", "#00C7BE", "#34C759", "#A2845E",
  ];

  const same = (a: string | null, b: string | null) => (a ?? "").toUpperCase() === (b ?? "").toUpperCase();
</script>

<svelte:window
  onpointerdown={(event) => {
    if (open && !root.contains(event.target as Node)) open = false;
  }}
  onkeydowncapture={(event) => {
    // Close only the popover, not the editor.
    if (open && event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      open = false;
    }
  }}
/>

<div class="relative" bind:this={root}>
  <button
    type="button"
    class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost {open ? 'bg-control-hover text-fg' : ''}"
    title={label}
    aria-label={label}
    aria-haspopup="true"
    aria-expanded={open}
    onclick={() => (open = !open)}
  >
    {#if kind === "fill"}
      <span class="flex flex-col items-center gap-px">
        <span class="text-[13px] font-bold leading-none">A</span>
        <span class="h-[3px] w-3.5 rounded-full ring-1 ring-line-strong" style:background={value ?? "transparent"}></span>
      </span>
    {:else if kind === "stroke"}
      <span
        class="h-3.5 w-3.5 rounded-[3px] ring-1 ring-line-strong {value ? 'border-[3px]' : 'none-swatch'}"
        style:border-color={value ?? undefined}
      ></span>
    {:else}
      <span
        class="flex h-3.5 w-3.5 items-center justify-center rounded-[3px] text-[9px] font-bold leading-none ring-1 ring-line-strong {value
          ? ''
          : 'none-swatch'}"
        style:background={value ?? undefined}
        style:color={value ? (same(value, "#1A1A1A") ? "#fff" : "#1A1A1A") : undefined}>T</span
      >
    {/if}
  </button>

  {#if open}
    <div class="ui-menu absolute left-0 top-full z-30 mt-1.5 w-[212px] space-y-2.5 p-2.5">
      <div class="text-[11px] font-medium text-fg-secondary">{label}</div>
      <div class="grid grid-cols-7 gap-1.5">
        {#if allowNone}
          <button
            type="button"
            class="swatch none-swatch {value === null ? 'swatch-active' : ''}"
            title="无"
            aria-label="无"
            onclick={() => onchange(null)}
          ></button>
        {/if}
        {#each SWATCHES as color (color)}
          <button
            type="button"
            class="swatch {same(value, color) ? 'swatch-active' : ''}"
            style:background={color}
            title={color}
            aria-label={color}
            onclick={() => onchange(color)}
          ></button>
        {/each}
      </div>
      <label class="flex items-center gap-2 border-t border-line pt-2.5 text-[11px] text-fg-muted">
        <input
          type="color"
          value={value ?? "#FFFFFF"}
          oninput={(event) => onchange(event.currentTarget.value.toUpperCase())}
        />
        自定义
        <span class="ml-auto font-mono text-fg-secondary">{value ? value.toUpperCase() : "无"}</span>
      </label>
      {@render children?.()}
    </div>
  {/if}
</div>

<style>
  .swatch {
    height: 22px;
    width: 22px;
    border-radius: 6px;
    border: 1px solid rgb(0 0 0 / 15%);
    transition: transform 0.1s;
  }

  .swatch:hover {
    transform: scale(1.1);
  }

  .swatch-active {
    outline: 2px solid rgb(var(--c-accent));
    outline-offset: 1px;
  }

  .none-swatch {
    background: linear-gradient(
      to top right,
      transparent calc(50% - 1px),
      rgb(var(--c-danger)) calc(50% - 1px),
      rgb(var(--c-danger)) calc(50% + 1px),
      transparent calc(50% + 1px)
    );
  }

  input[type="color"] {
    -webkit-appearance: none;
    appearance: none;
    width: 22px;
    height: 22px;
    flex-shrink: 0;
    border: none;
    padding: 0;
    background: transparent;
    cursor: pointer;
  }

  input[type="color"]::-webkit-color-swatch-wrapper {
    padding: 0;
  }

  input[type="color"]::-webkit-color-swatch {
    border: 1px solid rgb(var(--c-line-strong));
    border-radius: 6px;
  }

  input[type="color"]::-moz-color-swatch {
    border: 1px solid rgb(var(--c-line-strong));
    border-radius: 6px;
  }
</style>
