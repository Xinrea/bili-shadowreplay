<script lang="ts">
  import { X } from "lucide-svelte";
  import type { Config } from "../interface";
  import { invoke } from "../invoker";
  import { hotwordProblem, MAX_HOTWORDS, splitHotwords } from "../powerlive-subtitle";

  /** The global PowerLive hotwords; edits are saved as they are made. */
  let { hotwords = $bindable([]) }: { hotwords?: string[] } = $props();

  let draft = $state("");
  let problem = $state("");
  let saving: Promise<void> = Promise.resolve();
  let saves = 0;

  function save(next: string[]) {
    hotwords = next;
    const id = ++saves;
    // In order, so a quick add then remove is stored as the last one says.
    saving = saving.then(async () => {
      try {
        const stored = await invoke<string[]>("update_powerlive_hotwords", { hotwords: next });
        // An earlier reply would undo the edits made since.
        if (id === saves) hotwords = stored;
      } catch (error) {
        problem = "保存热词失败：" + error;
        // Show what is still stored.
        const config = await invoke<Config>("get_config").catch(() => null);
        if (config && id === saves) hotwords = config.powerlive_hotwords;
      }
    });
  }

  /** Add the words in the draft, leaving the refused ones to be fixed. */
  function commit() {
    const next = [...hotwords];
    const refused: string[] = [];
    problem = "";
    for (const word of splitHotwords(draft)) {
      if (next.includes(word)) continue;
      const reason = hotwordProblem(word);
      if (reason) {
        refused.push(word);
        problem ||= `「${word}」${reason}`;
      } else if (next.length >= MAX_HOTWORDS) {
        refused.push(word);
        problem ||= `最多 ${MAX_HOTWORDS} 个热词`;
      } else {
        next.push(word);
      }
    }
    draft = refused.join("，");
    if (next.length !== hotwords.length) save(next);
  }

  function remove(word: string) {
    save(hotwords.filter((entry) => entry !== word));
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.isComposing) return;
    if (event.key === "Enter" || event.key === "," || event.key === "，") {
      event.preventDefault();
      commit();
    } else if (event.key === "Backspace" && !draft && hotwords.length > 0) {
      remove(hotwords[hotwords.length - 1]);
    }
  }
</script>

<div>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div
    class="custom-scrollbar-light flex max-h-32 min-h-[30px] cursor-text flex-wrap items-center gap-1 overflow-y-auto rounded-[8px] border border-line-strong bg-control px-1.5 py-1 transition-colors focus-within:border-accent"
    onclick={(event) => (event.currentTarget.querySelector("input") as HTMLInputElement)?.focus()}
  >
    {#each hotwords as word (word)}
      <span class="inline-flex items-center gap-0.5 rounded-[5px] bg-accent-soft py-px pl-1.5 pr-0.5 text-[12px] leading-5 text-accent-fg">
        {word}
        <button type="button" class="rounded-[3px] p-px opacity-70 hover:bg-accent/20 hover:opacity-100" aria-label="删除热词 {word}" onclick={() => remove(word)}>
          <X class="h-3 w-3" />
        </button>
      </span>
    {/each}
    <input
      type="text"
      class="h-5 min-w-[8rem] flex-1 border-0 bg-transparent p-0 px-1 text-[12px] text-fg shadow-none outline-none placeholder:text-fg-faint focus:ring-0"
      bind:value={draft}
      {onkeydown}
      onblur={() => draft.trim() && commit()}
      oninput={() => (problem = "")}
      placeholder={hotwords.length ? "" : "输入后按回车添加，可用逗号分隔一次添加多个"}
      aria-label="添加热词"
      data-hotkey-block
    />
  </div>
  <div class="mt-1 flex justify-between gap-3 text-[11px]">
    <span class={problem ? "text-danger-fg" : "text-fg-muted"}>
      {problem || "主播名、游戏和角色名等专有名词，不要放常用词"}
    </span>
    <span class="shrink-0 text-fg-faint">{hotwords.length} / {MAX_HOTWORDS}</span>
  </div>
</div>
