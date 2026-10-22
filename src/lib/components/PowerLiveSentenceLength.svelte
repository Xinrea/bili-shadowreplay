<script lang="ts">
  import type { Config } from "../interface";
  import { invoke } from "../invoker";
  import { clampSentenceLength, SENTENCE_MAX_LENGTH } from "../powerlive-subtitle";

  /** The global PowerLive subtitle line length; saved when changed. */
  let { value = $bindable(SENTENCE_MAX_LENGTH.default), id = undefined }: { value?: number; id?: string } =
    $props();

  async function save() {
    value = clampSentenceLength(value);
    try {
      value = await invoke<number>("update_powerlive_sentence_max_length", { sentenceMaxLength: value });
    } catch (error) {
      console.error("Failed to save the PowerLive subtitle line length:", error);
      // Show what is still stored.
      value = await invoke<Config>("get_config")
        .then((config) => config.powerlive_sentence_max_length)
        .catch(() => value);
    }
  }
</script>

<div class="flex items-center gap-1.5">
  <input
    {id}
    type="number"
    class="ui-input w-16 text-right"
    min={SENTENCE_MAX_LENGTH.min}
    max={SENTENCE_MAX_LENGTH.max}
    step="1"
    bind:value
    onchange={save}
    data-hotkey-block
  />
  <span class="text-[12px] text-fg-muted">字</span>
</div>
