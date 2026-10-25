<script lang="ts">
  import { X } from "lucide-svelte";

  let {
    value,
    onChange,
    label = "标签",
    addLabel,
    removeLabel,
    hint = "输入后按回车添加，点击 × 删除",
    disabled = false,
    max,
    split = (text: string) => text.split(/[,，\n\r]+/).map((word) => word.trim()).filter(Boolean),
    validate = (_word: string): string | null => null,
    problem = $bindable(""),
  }: {
    value: string[];
    onChange: (next: string[]) => void;
    label?: string;
    addLabel?: string;
    removeLabel?: string;
    hint?: string;
    disabled?: boolean;
    max?: number;
    split?: (text: string) => string[];
    validate?: (word: string) => string | null;
    problem?: string;
  } = $props();

  let draft = $state("");

  /** Add the draft, leaving refused entries available to correct. */
  function commit() {
    if (disabled) return;
    const next = [...value];
    const refused: string[] = [];
    problem = "";
    for (const word of split(draft)) {
      if (next.includes(word)) continue;
      const reason = validate(word);
      if (reason) {
        refused.push(word);
        problem ||= `「${word}」${reason}`;
      } else if (max !== undefined && next.length >= max) {
        refused.push(word);
        problem ||= `最多 ${max} 个${label}`;
      } else {
        next.push(word);
      }
    }
    draft = refused.join("，");
    if (next.length !== value.length) onChange(next);
  }

  function remove(word: string) {
    if (disabled) return;
    onChange(value.filter((entry) => entry !== word));
  }

  function onkeydown(event: KeyboardEvent) {
    if (disabled || event.isComposing) return;
    if (event.key === "Enter" || event.key === "," || event.key === "，") {
      event.preventDefault();
      commit();
    } else if (event.key === "Backspace" && !draft && value.length > 0) {
      remove(value[value.length - 1]);
    }
  }
</script>

<div>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div
    class="custom-scrollbar-light flex max-h-32 min-h-[30px] cursor-text flex-wrap items-center gap-1 overflow-y-auto rounded-[8px] border border-line-strong bg-control px-1.5 py-1 transition-colors focus-within:border-accent"
    onclick={(event) => (event.currentTarget.querySelector("input") as HTMLInputElement)?.focus()}
  >
    {#each value as word (word)}
      <span class="inline-flex items-center gap-0.5 rounded-[5px] bg-accent-soft py-px pl-1.5 pr-0.5 text-[12px] leading-5 text-accent-fg">
        {word}
        <button type="button" class="rounded-[3px] p-px opacity-70 hover:bg-accent/20 hover:opacity-100" aria-label="{removeLabel ?? `删除${label}`} {word}" {disabled} onclick={() => remove(word)}>
          <X class="h-3 w-3" />
        </button>
      </span>
    {/each}
    <input
      type="text"
      class="h-5 min-w-[8rem] flex-1 border-0 bg-transparent p-0 px-1 text-[12px] text-fg shadow-none outline-none placeholder:text-fg-faint focus:ring-0"
      bind:value={draft}
      {disabled}
      {onkeydown}
      onblur={() => draft.trim() && commit()}
      oninput={() => (problem = "")}
      placeholder={value.length ? "" : "输入后按回车添加，可用逗号分隔一次添加多个"}
      aria-label={addLabel ?? `添加${label}`}
      data-hotkey-block
    />
  </div>
  <div class="mt-1 flex justify-between gap-3 text-[11px]">
    <span class={problem ? "text-danger-fg" : "text-fg-muted"}>
      {problem || hint}
    </span>
    {#if max !== undefined}
      <span class="shrink-0 text-fg-faint">{value.length} / {max}</span>
    {/if}
  </div>
</div>
