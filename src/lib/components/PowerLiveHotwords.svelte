<script lang="ts">
  import TagInput from "./TagInput.svelte";
  import type { Config } from "../interface";
  import { invoke } from "../invoker";
  import { hotwordProblem, MAX_HOTWORDS, splitHotwords } from "../powerlive-subtitle";

  /** The global PowerLive hotwords; edits are saved as they are made. */
  let { hotwords = $bindable([]) }: { hotwords?: string[] } = $props();

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
</script>

<TagInput
  value={hotwords}
  onChange={save}
  label="热词"
  hint="主播名、游戏和角色名等专有名词，不要放常用词"
  max={MAX_HOTWORDS}
  split={splitHotwords}
  validate={hotwordProblem}
  bind:problem
/>
