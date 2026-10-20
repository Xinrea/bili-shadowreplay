<script lang="ts">
  import { onMount } from "svelte";
  import { fontFamily, layoutText, LINE_HEIGHT, type TextElement } from "../../cover-elements";
  import { measure } from "./render";

  // Edits a text in place over the stage, styled like the drawn text. Mount it
  // anew for each text edited.

  interface Props {
    text: TextElement;
    /** CSS pixels per element-space pixel. */
    unit: number;
    /** The text's center on the stage, in CSS pixels. */
    left: number;
    top: number;
    selectAll?: boolean;
    onInput: (content: string) => void;
    /** Editing ends: the field lost focus, or Escape or ⌘/Ctrl+Enter was pressed. */
    onDone: (byKey: boolean) => void;
  }

  let { text, unit, left, top, selectAll = false, onInput, onDone }: Props = $props();
  let field: HTMLDivElement;
  let layout = $derived(layoutText(text, measure));

  const read = () => field.innerText.replace(/\r/g, "").replace(/\n$/, "");

  /**
   * What the field reads now, with any IME composition in progress, for an
   * edit ended from outside the field before an input event reports it.
   */
  export function content(): string {
    return read();
  }

  onMount(() => {
    try {
      field.contentEditable = "plaintext-only";
    } catch {
      // Older engines reject it; innerText still reads plain text.
      field.contentEditable = "true";
    }
    field.textContent = text.content;
    field.focus();
    const range = document.createRange();
    range.selectNodeContents(field);
    if (!selectAll) range.collapse(false);
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
  });

  /** Report what the field reads as editing ends, should an input event have gone missing. */
  function done(byKey: boolean) {
    onInput(read());
    onDone(byKey);
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.isComposing) return;
    if (event.key === "Escape" || (event.key === "Enter" && (event.metaKey || event.ctrlKey))) {
      // Leave editing, not the editor.
      event.preventDefault();
      event.stopPropagation();
      done(true);
    }
  }
</script>

<!-- Typing and selecting stay inside the field. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="absolute"
  style:left="{left}px"
  style:top="{top}px"
  style:transform="translate(-50%, -50%) rotate({text.rotation}deg)"
  onpointerdown={(event) => event.stopPropagation()}
  ondblclick={(event) => event.stopPropagation()}
>
  <div
    bind:this={field}
    contenteditable="plaintext-only"
    role="textbox"
    tabindex="0"
    aria-multiline="true"
    aria-label="文字内容"
    spellcheck="false"
    class="edit-field"
    style="-webkit-text-stroke: {text.stroke ? `${text.strokeWidth * text.fontSize * 2 * unit}px ${text.stroke}` : '0'}"
    style:font-family={fontFamily(text.font)}
    style:font-weight={text.bold ? "bold" : "normal"}
    style:font-size="{text.fontSize * unit}px"
    style:line-height={LINE_HEIGHT}
    style:color={text.fill}
    style:text-align={text.align}
    style:padding="{layout.padY * unit}px {layout.padX * unit}px"
    style:background={text.background ?? "transparent"}
    style:border-radius="{text.fontSize * 0.18 * unit}px"
    oninput={() => onInput(read())}
    oncompositionend={() => onInput(read())}
    onkeydown={handleKeydown}
    onblur={() => done(false)}
  ></div>
</div>

<style>
  .edit-field {
    min-width: 0.6em;
    white-space: pre;
    outline: 1.5px dashed rgb(var(--c-accent));
    outline-offset: 2px;
    paint-order: stroke fill;
    cursor: text;
    user-select: text;
    -webkit-user-select: text;
  }
</style>
