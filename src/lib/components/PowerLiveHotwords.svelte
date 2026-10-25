<script lang="ts">
  import TagInput from "./TagInput.svelte";
  import type { Config } from "../interface";
  import { invoke } from "../invoker";
  import { hotwordProblem, MAX_HOTWORDS, roomHotwords, splitHotwords, type HotwordRoom } from "../powerlive-subtitle";

  /** Without a room, edits go to the global dictionary. */
  let { hotwords = $bindable([]), room }: { hotwords?: string[]; room?: HotwordRoom } = $props();
  let dictionaryLabel = $derived(room ? "直播间词库" : "全局词库");

  let problem = $state("");
  let saving: Promise<void> = Promise.resolve();
  let saves = 0;

  function save(next: string[]) {
    hotwords = next;
    const id = ++saves;
    const scope = room;
    // In order, so a quick add then remove is stored as the last one says.
    saving = saving.then(async () => {
      try {
        const stored = await invoke<string[]>("update_powerlive_hotwords", { hotwords: next, ...scope });
        // An earlier reply would undo the edits made since.
        if (id === saves && scope === room) hotwords = stored;
      } catch (error) {
        problem = "保存热词失败：" + error;
        // Show what is still stored.
        const config = await invoke<Config>("get_config").catch(() => null);
        if (config && id === saves && scope === room) {
          hotwords = scope ? roomHotwords(config.powerlive_room_hotwords, scope) : config.powerlive_hotwords;
        }
      }
    });
  }
</script>

<TagInput
  value={hotwords}
  onChange={save}
  label="热词"
  addLabel={`向${dictionaryLabel}添加热词`}
  removeLabel={`从${dictionaryLabel}删除热词`}
  hint="主播名、游戏和角色名等专有名词，不要放常用词"
  max={MAX_HOTWORDS}
  split={splitHotwords}
  validate={hotwordProblem}
  bind:problem
/>
