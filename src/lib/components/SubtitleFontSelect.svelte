<script lang="ts">
  import { tick } from "svelte";
  import { Check, ChevronDown } from "lucide-svelte";
  import { loadSubtitleFontFamilies, type SubtitleFontFamily } from "../subtitle-fonts";

  interface Props {
    value: string;
    disabled?: boolean;
    onSelect: (family: string) => void;
  }

  let { value, disabled = false, onSelect }: Props = $props();
  const listId = $props.id();
  let input: HTMLInputElement = $state();
  let list: HTMLUListElement = $state();
  let open = $state(false);
  let query = $state("");
  let active = $state(0);
  let families: SubtitleFontFamily[] | null = $state(null);
  let loadError = $state("");
  let position = $state({ left: 0, top: 0, width: 0, maxHeight: 0, above: false });

  function load() {
    loadError = "";
    loadSubtitleFontFamilies()
      .then((result) => (families = result))
      .catch((error) => (loadError = error instanceof Error ? error.message : String(error)));
  }
  $effect(load);

  // Chinese-capable fonts first; each group keeps the backend's name order.
  let sorted = $derived(
    families ? [...families.filter((font) => font.chinese), ...families.filter((font) => !font.chinese)] : [],
  );
  let filtered = $derived.by(() => {
    const term = query.trim().toLowerCase();
    if (!term) return sorted;
    return sorted.filter(
      (font) => font.family.toLowerCase().includes(term) || font.localized?.toLowerCase().includes(term),
    );
  });
  let current = $derived.by(() => {
    const name = value.trim().toLowerCase();
    return families?.find((font) => font.family.toLowerCase() === name || font.localized?.toLowerCase() === name);
  });

  function place() {
    const rect = input.getBoundingClientRect();
    const below = window.innerHeight - rect.bottom - 12;
    const above = rect.top - 12;
    const upward = below < 300 && above > below;
    position = {
      left: rect.left,
      top: upward ? rect.top - 4 : rect.bottom + 4,
      width: Math.max(rect.width, 300),
      maxHeight: Math.min(320, upward ? above : below),
      above: upward,
    };
  }

  async function reveal() {
    await tick();
    list?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }

  function show() {
    if (disabled || open) return;
    query = "";
    open = true;
    place();
    active = Math.max(0, current ? sorted.indexOf(current) : 0);
    reveal();
  }

  function close() {
    open = false;
    query = "";
  }

  function choose(font: SubtitleFontFamily) {
    close();
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
      // Close only the list, not the surrounding style dialog.
      event.preventDefault();
      event.stopPropagation();
      close();
    } else if (event.key === "Tab") {
      close();
    }
  }

  $effect(() => {
    if (!open) return;
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  });
</script>

<div class="relative">
  <input
    bind:this={input}
    type="text"
    role="combobox"
    aria-expanded={open}
    aria-controls={listId}
    aria-autocomplete="list"
    aria-activedescendant={open && filtered[active] ? `${listId}-${active}` : undefined}
    value={open ? query : value}
    placeholder={value}
    {disabled}
    spellcheck="false"
    autocomplete="off"
    class="style-input pr-16"
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
  <div class="pointer-events-none absolute inset-y-0 right-2 flex items-center gap-1 text-gray-500">
    {#if families && !current && !open}
      <span class="rounded bg-amber-500/15 px-1 text-[10px] text-amber-300" title="压制机器未安装该字体">未安装</span>
    {/if}
    <ChevronDown class="h-3.5 w-3.5" />
  </div>
</div>

{#if open}
  <ul
    bind:this={list}
    id={listId}
    role="listbox"
    aria-label="字体"
    class="sidebar-scrollbar fixed z-10 overflow-y-auto rounded-lg border border-gray-700 bg-[#2c2c2e] py-1 text-sm shadow-2xl"
    style:left="{position.left}px"
    style:top="{position.top}px"
    style:width="{position.width}px"
    style:max-height="{position.maxHeight}px"
    style:transform={position.above ? "translateY(-100%)" : undefined}
    onmousedown={(event) => event.preventDefault()}
  >
    {#if loadError}
      <li class="px-3 py-2 text-xs text-amber-300">
        读取字体列表失败：{loadError}
        <button type="button" class="ml-2 text-blue-400 hover:text-blue-300" onclick={load}>重试</button>
      </li>
    {:else if !families}
      <li class="px-3 py-2 text-xs text-gray-400">正在读取压制机器的字体…</li>
    {:else if filtered.length === 0}
      <li class="px-3 py-2 text-xs text-gray-400">没有匹配的字体</li>
    {:else}
      {#each filtered as font, index (font.family)}
        {#if index === 0 || filtered[index - 1].chinese !== font.chinese}
          <li role="presentation" class="px-3 pb-1 pt-2 text-[11px] text-gray-500">
            {font.chinese ? "含中文字形" : "其他字体"}
          </li>
        {/if}
        <!-- The combobox input handles keyboard selection through aria-activedescendant. -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <li
          id="{listId}-{index}"
          data-index={index}
          role="option"
          aria-selected={font === current}
          class="flex cursor-pointer items-center gap-2 px-3 py-1.5 {index === active ? 'bg-[#0A84FF]/25 text-white' : 'text-gray-200'}"
          onmouseenter={() => (active = index)}
          onclick={() => choose(font)}
        >
          <span class="min-w-0 flex-1 truncate">
            {font.family}
            {#if font.localized}<span class="ml-1.5 text-xs text-gray-400">{font.localized}</span>{/if}
          </span>
          {#if font === current}<Check class="h-3.5 w-3.5 shrink-0 text-[#0A84FF]" />{/if}
        </li>
      {/each}
    {/if}
  </ul>
{/if}

<style>
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

  .style-input:focus {
    border-color: #0a84ff;
  }

  .style-input:disabled {
    opacity: 0.5;
  }
</style>
