<script lang="ts">
  import { tick } from "svelte";
  import { Check, ChevronDown } from "lucide-svelte";
  import { fontFamily } from "../../cover-elements";
  import { loadSubtitleFontFamilies, type SubtitleFontFamily } from "../../subtitle-fonts";
  import { fontAvailable } from "./render";

  interface Props {
    /** Family name; empty for the default. */
    value: string;
    onSelect: (family: string) => void;
  }

  let { value, onSelect }: Props = $props();
  const listId = $props.id();
  const DEFAULT: SubtitleFontFamily = { family: "", localized: "默认黑体", chinese: true };

  let input: HTMLInputElement = $state();
  let list: HTMLUListElement = $state();
  let open = $state(false);
  let query = $state("");
  let active = $state(0);
  let families: SubtitleFontFamily[] | null = $state(null);
  let loadError = $state("");
  let loading: Promise<void> | null = null;

  // The installed families, minus those this webview cannot draw.
  function load() {
    loading ??= (async () => {
      loadError = "";
      try {
        const all = await loadSubtitleFontFamilies();
        const usable: SubtitleFontFamily[] = [];
        for (let index = 0; index < all.length; index++) {
          if (fontAvailable(all[index].family)) usable.push(all[index]);
          // Checking draws text; let the list open meanwhile.
          if (index % 40 === 39) await new Promise((resolve) => setTimeout(resolve));
        }
        families = [
          DEFAULT,
          ...usable.filter((font) => font.chinese),
          ...usable.filter((font) => !font.chinese),
        ];
      } catch (error) {
        loadError = error instanceof Error ? error.message : String(error);
        loading = null;
      }
    })();
  }

  let filtered = $derived.by(() => {
    const term = query.trim().toLowerCase();
    const all = families ?? [DEFAULT];
    if (!term) return all;
    return all.filter(
      (font) => font.family.toLowerCase().includes(term) || font.localized?.toLowerCase().includes(term),
    );
  });
  let label = $derived(
    value ? (families?.find((font) => font.family === value)?.localized ?? value) : DEFAULT.localized,
  );

  async function reveal() {
    await tick();
    list?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }

  function show() {
    if (open) return;
    load();
    query = "";
    open = true;
    active = Math.max(0, filtered.findIndex((font) => font.family === value));
    reveal();
  }

  function close() {
    open = false;
    query = "";
  }

  function choose(font: SubtitleFontFamily) {
    close();
    input.blur();
    if (font.family !== value) onSelect(font.family);
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (!open) return show();
      const step = event.key === "ArrowDown" ? 1 : -1;
      active = Math.min(Math.max(active + step, 0), Math.max(filtered.length - 1, 0));
      reveal();
    } else if (event.key === "Enter" && open && filtered[active]) {
      event.preventDefault();
      choose(filtered[active]);
    } else if (event.key === "Escape" && open) {
      // Close only the list, not the editor.
      event.preventDefault();
      event.stopPropagation();
      close();
    } else if (event.key === "Tab") {
      close();
    }
  }
</script>

<div class="relative w-[150px]">
  <input
    bind:this={input}
    type="text"
    role="combobox"
    aria-label="字体"
    aria-expanded={open}
    aria-controls={listId}
    aria-autocomplete="list"
    aria-activedescendant={open && filtered[active] ? `${listId}-${active}` : undefined}
    value={open ? query : label}
    placeholder={label}
    spellcheck="false"
    autocomplete="off"
    class="ui-input h-[26px] w-full pr-6 text-[12px]"
    style:font-family={open ? undefined : fontFamily(value)}
    onfocus={show}
    onclick={show}
    onblur={close}
    oninput={(event) => {
      query = event.currentTarget.value;
      active = 0;
      if (!open) show();
    }}
    onkeydown={keydown}
  />
  <ChevronDown class="pointer-events-none absolute right-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-fg-muted" />

  {#if open}
    <ul
      bind:this={list}
      id={listId}
      role="listbox"
      aria-label="字体"
      class="ui-menu custom-scrollbar-light absolute left-0 top-full z-30 mt-1 max-h-[320px] w-[260px] overflow-y-auto"
      onmousedown={(event) => event.preventDefault()}
    >
      {#each filtered as font, index (font.family)}
        {#if index > 0 && filtered[index - 1].chinese !== font.chinese}
          <li role="presentation" class="px-2.5 pb-1 pt-2 text-[10px] text-fg-faint">其他字体</li>
        {/if}
        <!-- The combobox input handles keyboard selection through aria-activedescendant. -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <li
          id="{listId}-{index}"
          data-index={index}
          role="option"
          aria-selected={font.family === value}
          class="flex cursor-pointer items-center gap-2 rounded-[6px] px-2.5 py-1.5 text-[13px] {index === active
            ? 'bg-control-hover text-fg'
            : 'text-fg-secondary'}"
          onmouseenter={() => (active = index)}
          onclick={() => choose(font)}
        >
          <span class="min-w-0 flex-1 truncate" style:font-family={fontFamily(font.family)}>
            {font.family ? (font.localized ?? font.family) : font.localized}
          </span>
          {#if font.family && font.localized}
            <span class="max-w-[90px] shrink-0 truncate text-[10px] text-fg-faint">{font.family}</span>
          {/if}
          {#if font.family === value}<Check class="h-3.5 w-3.5 shrink-0 text-accent-fg" />{/if}
        </li>
      {/each}
      {#if loadError}
        <li class="px-2.5 py-2 text-[11px] text-danger-fg">读取字体列表失败：{loadError}</li>
      {:else if !families}
        <li class="px-2.5 py-2 text-[11px] text-fg-muted">正在读取已安装的字体…</li>
      {:else if filtered.length === 0}
        <li class="px-2.5 py-2 text-[11px] text-fg-muted">没有匹配的字体</li>
      {/if}
    </ul>
  {/if}
</div>
