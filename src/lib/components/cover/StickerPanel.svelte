<script lang="ts">
  import { ImagePlus, Loader2, X } from "lucide-svelte";
  import {
    builtinStickerRef,
    customStickerRef,
    deleteCustomSticker,
    importSticker,
    loadCustomStickers,
    type CustomSticker,
  } from "./custom-stickers";
  import { STICKER_GROUPS, stickerUrl } from "./render";

  interface Props {
    /** Paste images only while the editor is open. */
    active: boolean;
    group: string;
    /**
     * `src` is the image's URL and `ref` names it for good, as kept in
     * projects; `size` is for pictures that should start larger than an emoji.
     */
    onPick: (sticker: { src: string; ref: string }, size?: number) => void;
  }

  const MINE = "我的";
  let { active, group = $bindable(), onPick }: Props = $props();
  let stickers = $derived(STICKER_GROUPS.find((g) => g.name === group)?.stickers ?? []);

  let custom: CustomSticker[] | null = $state(null);
  let loadError = $state("");
  let importing = $state(0);
  let dragging = $state(false);
  let fileInput: HTMLInputElement = $state();
  // Deleted while the first list was loading, which may still name them.
  const deleted = new Set<string>();

  $effect(() => {
    if (group === MINE && custom === null) void refresh();
  });

  async function refresh() {
    loadError = "";
    try {
      const listed = (await loadCustomStickers()).filter((sticker) => !deleted.has(sticker.id));
      // Keep stickers imported while the list was loading.
      const imported = (custom ?? []).filter((sticker) => !listed.some((s) => s.id === sticker.id));
      custom = [...listed, ...imported];
    } catch (error) {
      loadError = String(error);
    }
  }

  async function importFiles(files: Iterable<File>) {
    const images = [...files].filter((file) => file.type.startsWith("image/"));
    if (!images.length) return;
    group = MINE;
    importing += images.length;
    const failures: string[] = [];
    for (const file of images) {
      try {
        const sticker = await importSticker(file);
        custom = [...(custom ?? []), sticker];
      } catch (error) {
        failures.push(`${file.name || "图片"}：${error instanceof Error ? error.message : error}`);
      } finally {
        importing--;
      }
    }
    if (failures.length) alert("导入贴纸失败\n" + failures.join("\n"));
  }

  async function remove(sticker: CustomSticker) {
    // Tauri's dialog plugin makes confirm() asynchronous.
    if (!(await confirm("删除这个贴纸吗？已保存的封面不受影响，但切片中用到它的地方将无法显示。"))) return;
    try {
      await deleteCustomSticker(sticker.id);
      deleted.add(sticker.id);
      custom = custom.filter((s) => s.id !== sticker.id);
    } catch (error) {
      alert("删除贴纸失败：" + error);
    }
  }

  function handlePaste(event: ClipboardEvent) {
    if (!active) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest?.("input, textarea, [contenteditable]")) return;
    const files = [...(event.clipboardData?.files ?? [])];
    if (!files.some((file) => file.type.startsWith("image/"))) return;
    event.preventDefault();
    void importFiles(files);
  }
</script>

<svelte:window onpaste={handlePaste} />

<!-- Dropped images are imported; the import button offers the same. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="relative min-h-full space-y-3"
  ondragover={(event) => {
    if (!event.dataTransfer?.types.includes("Files")) return;
    event.preventDefault();
    dragging = true;
  }}
  ondragleave={(event) => {
    if (!event.currentTarget.contains(event.relatedTarget as Node)) dragging = false;
  }}
  ondrop={(event) => {
    event.preventDefault();
    dragging = false;
    void importFiles(event.dataTransfer?.files ?? []);
  }}
>
  <div class="grid grid-cols-7" role="tablist" aria-label="贴纸分类">
    {#each [...STICKER_GROUPS.map((g) => g.name), MINE] as name (name)}
      <button
        type="button"
        role="tab"
        aria-selected={group === name}
        class="h-[26px] rounded-[7px] text-[12px] transition-colors {group === name
          ? 'bg-accent-soft font-medium text-accent-fg'
          : 'text-fg-muted hover:bg-control-hover hover:text-fg'}"
        onclick={() => (group = name)}
      >
        {name}
      </button>
    {/each}
  </div>

  {#if group === MINE}
    <div class="grid grid-cols-4 gap-1.5" role="tabpanel" aria-label={MINE}>
      <button
        type="button"
        class="flex aspect-square flex-col items-center justify-center gap-1 rounded-[8px] border border-dashed border-line-strong text-[10px] text-fg-muted transition-colors hover:border-accent hover:text-accent-fg"
        title="导入图片，也可以拖入或粘贴"
        disabled={importing > 0}
        onclick={() => fileInput.click()}
      >
        {#if importing}
          <Loader2 class="h-4 w-4 animate-spin" />
          导入中
        {:else}
          <ImagePlus class="h-4 w-4" />
          导入
        {/if}
      </button>
      {#each custom ?? [] as sticker (sticker.id)}
        <div class="group relative aspect-square">
          <button
            type="button"
            class="flex h-full w-full items-center justify-center rounded-[8px] bg-control/60 p-1 transition-colors hover:bg-control-hover focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
            aria-label="添加贴纸"
            onclick={() => onPick({ src: sticker.url, ref: customStickerRef(sticker) }, 420)}
          >
            <img
              src={sticker.url}
              alt=""
              class="max-h-full max-w-full object-contain"
              loading="lazy"
              decoding="async"
              draggable="false"
            />
          </button>
          <button
            type="button"
            class="absolute -right-1 -top-1 hidden h-4 w-4 items-center justify-center rounded-full bg-fg-secondary text-surface shadow hover:bg-danger group-hover:flex group-focus-within:flex"
            title="删除贴纸"
            aria-label="删除贴纸"
            onclick={() => remove(sticker)}
          >
            <X class="h-2.5 w-2.5" />
          </button>
        </div>
      {/each}
    </div>
    <input
      bind:this={fileInput}
      type="file"
      accept="image/png,image/jpeg,image/webp,image/gif,image/svg+xml"
      multiple
      class="hidden"
      onchange={(event) => {
        void importFiles(event.currentTarget.files ?? []);
        event.currentTarget.value = "";
      }}
    />
    {#if loadError}
      <p class="text-[11px] text-danger-fg">
        读取贴纸失败：{loadError}
        <button type="button" class="ml-1 text-accent-fg hover:underline" onclick={refresh}>重试</button>
      </p>
    {:else if custom === null}
      <p class="ui-hint">正在读取…</p>
    {:else if custom.length === 0}
      <p class="ui-hint">还没有贴纸。导入透明背景的 PNG 效果最好，四周的透明边距会自动裁掉。</p>
    {/if}
    <p class="ui-hint">
      可以导入自己下载的图片，比如いらすとや（irasutoya.com）的插图，也可以拖入或粘贴。贴纸保存在服务端，所有设备共用；使用他人的素材请遵守其使用规定。
    </p>
  {:else}
    <div class="grid grid-cols-6 gap-0.5" role="tabpanel" aria-label={group}>
      {#each stickers as sticker (sticker.id)}
        <button
          type="button"
          class="flex aspect-square items-center justify-center rounded-[8px] transition-[background-color,transform] hover:scale-110 hover:bg-control-hover focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
          title={sticker.glyph}
          aria-label={sticker.name}
          onclick={() => onPick({ src: stickerUrl(sticker.id), ref: builtinStickerRef(sticker.id) })}
        >
          <img src={stickerUrl(sticker.id)} alt="" class="h-7 w-7" loading="lazy" decoding="async" draggable="false" />
        </button>
      {/each}
    </div>
  {/if}

  {#if dragging}
    <div
      class="pointer-events-none absolute inset-0 flex items-center justify-center rounded-[10px] border-2 border-dashed border-accent bg-accent-soft/80 text-[12px] font-medium text-accent-fg"
    >
      松开以导入为贴纸
    </div>
  {/if}
</div>
