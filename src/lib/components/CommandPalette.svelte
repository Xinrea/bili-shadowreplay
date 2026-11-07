<script lang="ts">
  import { tick } from "svelte";
  import { Search } from "lucide-svelte";
  import {
    PALETTE_SHORTCUT,
    isPaletteShortcut,
    paletteSections,
    rememberRecent,
    type PaletteCommand,
  } from "../command-palette";
  import {
    closeCommandPalette,
    commandPaletteOpen,
    commands,
    openCommandPalette,
  } from "../stores/commands";

  // Mount once, in the main window. Pages contribute commands with
  // `registerCommands` from `stores/commands`.

  interface Props {
    /** Shows a main window page; commands with a `page` run once it is shown. */
    onNavigate: (page: string) => void;
  }

  let { onNavigate }: Props = $props();

  const RECENT_KEY = "command-palette-recent";

  let query = $state("");
  let activeIndex = $state(0);
  let recent: string[] = $state(loadRecent());
  let input: HTMLInputElement | null = $state(null);
  let list: HTMLElement | null = $state(null);
  /** Focus to give back when the palette is dismissed without running anything. */
  let returnFocus: HTMLElement | null = null;

  // Availability is checked while building the list, so it follows the state
  // of the pages each time the palette opens or the query changes.
  let sections = $derived(
    $commandPaletteOpen ? paletteSections($commands, query, recent) : [],
  );
  let items = $derived(sections.flatMap((section) => section.commands));
  let active = $derived(items[activeIndex]);

  function loadRecent(): string[] {
    try {
      const saved = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
      return Array.isArray(saved) ? saved.filter((id) => typeof id === "string") : [];
    } catch {
      return [];
    }
  }

  function dismiss() {
    closeCommandPalette();
    returnFocus?.focus({ preventScroll: true });
    returnFocus = null;
  }

  async function execute(command: PaletteCommand) {
    // Focus stays put: the command may switch pages or focus something itself.
    returnFocus = null;
    closeCommandPalette();
    recent = rememberRecent(recent, command.id);
    try {
      localStorage.setItem(RECENT_KEY, JSON.stringify(recent));
    } catch (error) {
      // Only the history is lost; the command still runs.
      console.error("保存最近使用的命令失败", error);
    }
    if (command.page) {
      onNavigate(command.page);
      await tick();
    }
    try {
      await command.run?.();
    } catch (error) {
      alert(`执行「${command.title}」失败：${error}`);
    }
  }

  function move(step: number) {
    if (items.length === 0) return;
    activeIndex = (activeIndex + step + items.length) % items.length;
  }

  // Handled in the capture phase: while open, the palette owns the keyboard so
  // page shortcuts underneath never see the keys. Typing still reaches the
  // search field, since stopping propagation keeps default actions.
  function handleKeydown(event: KeyboardEvent) {
    if (isPaletteShortcut(event)) {
      event.preventDefault();
      event.stopImmediatePropagation();
      // Holding the keys repeats them; only the first press toggles.
      if (event.repeat) return;
      if ($commandPaletteOpen) dismiss();
      else openCommandPalette();
      return;
    }
    if (!$commandPaletteOpen) return;
    event.stopPropagation();
    // Keys confirming or cancelling an IME composition belong to the IME.
    if (event.isComposing || event.keyCode === 229) return;
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        move(1);
        break;
      case "ArrowUp":
        event.preventDefault();
        move(-1);
        break;
      case "Enter":
        event.preventDefault();
        if (active) void execute(active);
        break;
      case "Escape":
        // Also keeps the task center, should it be open underneath, open.
        event.preventDefault();
        event.stopImmediatePropagation();
        dismiss();
        break;
      case "Tab":
        // Keep focus in the search field.
        event.preventDefault();
        break;
    }
  }

  // Before the DOM update, so the list never renders with the last query.
  $effect.pre(() => {
    if (!$commandPaletteOpen) return;
    query = "";
    activeIndex = 0;
    returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    void tick().then(() => input?.focus());
  });

  $effect(() => {
    if (!list || items.length === 0) return;
    // The first item sits under a section heading; show that as well.
    if (activeIndex === 0) list.scrollTop = 0;
    else list.querySelector(`[data-index="${activeIndex}"]`)?.scrollIntoView({ block: "nearest" });
  });
</script>

<svelte:window onkeydowncapture={handleKeydown} />

{#snippet key(label: string)}
  <kbd
    class="inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-[5px] border border-line-strong bg-control px-1 font-sans text-[10px] leading-none text-fg-muted"
    >{label}</kbd
  >
{/snippet}

{#if $commandPaletteOpen}
  <!-- Above the task center (z-1200), which may be open underneath. -->
  <div class="fixed inset-0 z-[1300] flex items-start justify-center p-4 pt-[12vh]">
    <button
      type="button"
      tabindex="-1"
      class="absolute inset-0 cursor-default bg-black/50 backdrop-blur-sm"
      aria-label="关闭命令面板"
      onclick={dismiss}
    ></button>

    <div
      role="dialog"
      aria-modal="true"
      aria-label="命令面板"
      class="relative flex max-h-[min(520px,76vh)] w-full max-w-xl flex-col overflow-hidden rounded-[12px] border border-line bg-surface text-fg shadow-2xl"
    >
      <div class="flex items-center gap-2.5 border-b border-line px-4">
        <Search class="h-4 w-4 shrink-0 text-fg-faint" />
        <input
          bind:this={input}
          bind:value={query}
          oninput={() => (activeIndex = 0)}
          class="h-12 min-w-0 flex-1 border-0 bg-transparent p-0 text-[14px] text-fg placeholder:text-fg-faint focus:outline-none focus:ring-0"
          placeholder="搜索页面、功能或设置项…"
          role="combobox"
          aria-expanded="true"
          aria-controls="command-palette-list"
          aria-autocomplete="list"
          aria-activedescendant={active ? `command-palette-option-${activeIndex}` : undefined}
          autocomplete="off"
          spellcheck="false"
        />
        {@render key("Esc")}
      </div>

      <div
        bind:this={list}
        id="command-palette-list"
        role="listbox"
        aria-label="命令"
        class="min-h-0 flex-1 overflow-y-auto p-1.5 custom-scrollbar-light"
      >
        {#each sections as section}
          <div role="group" aria-label={section.label ?? "搜索结果"}>
            {#if section.label}
              <div aria-hidden="true" class="px-2.5 pb-1 pt-2 text-[11px] font-medium text-fg-muted">
                {section.label}
              </div>
            {/if}
            {#each section.commands as command (command.id)}
              {@const index = items.indexOf(command)}
              {@const selected = index === activeIndex}
              <button
                type="button"
                tabindex="-1"
                role="option"
                id="command-palette-option-{index}"
                data-index={index}
                aria-selected={selected}
                class="flex h-9 w-full items-center gap-2.5 rounded-[8px] px-2.5 text-left {selected
                  ? 'bg-accent-soft text-accent-fg'
                  : 'text-fg-secondary'}"
                onpointermove={() => (activeIndex = index)}
                onclick={() => execute(command)}
              >
                {#if command.icon}
                  <command.icon class="h-4 w-4 shrink-0 {selected ? '' : 'text-fg-muted'}" />
                {/if}
                <span class="truncate text-[13px]">{command.title}</span>
                {#if command.detail}
                  <span class="ml-auto shrink-0 pl-3 text-[11px] {selected ? 'text-accent-fg/75' : 'text-fg-faint'}">
                    {command.detail}
                  </span>
                {/if}
              </button>
            {/each}
          </div>
        {:else}
          <div class="py-8 text-center text-[12px] text-fg-muted">没有匹配的命令</div>
        {/each}
      </div>

      <div class="flex items-center gap-4 border-t border-line bg-chrome px-4 py-2 text-[11px] text-fg-muted">
        <span class="flex items-center gap-1">{@render key("↑")}{@render key("↓")} 选择</span>
        <span class="flex items-center gap-1">{@render key("↵")} 执行</span>
        <span class="ml-auto flex items-center gap-1">{@render key(PALETTE_SHORTCUT)} 打开 / 关闭</span>
      </div>
    </div>
  </div>
{/if}
