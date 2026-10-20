<script lang="ts">
  import { X } from "lucide-svelte";

  interface Props {
    show?: boolean;
  }

  let { show = $bindable(false) }: Props = $props();

  interface Shortcut {
    // 每一项是一组同时按下的键，多组之间是“或”的关系
    keys: string[][];
    description: string;
  }

  const timingShortcuts: Shortcut[] = [
    { keys: [["Z"], ["X"]], description: "选中上一条 / 下一条字幕" },
    { keys: [["S"]], description: "播放选中字幕" },
    { keys: [["Q"]], description: "播放选中字幕前 500ms" },
    { keys: [["W"]], description: "播放选中字幕后 500ms" },
    { keys: [["E"]], description: "播放选中字幕开头 500ms" },
    { keys: [["D"]], description: "播放选中字幕结尾 500ms" },
    { keys: [["Ctrl/⌘", "3"]], description: "开始时间设为播放位置" },
    { keys: [["Ctrl/⌘", "4"]], description: "结束时间设为播放位置" },
  ];

  const playbackShortcuts: Shortcut[] = [
    { keys: [["Space"]], description: "播放 / 暂停" },
    { keys: [["←"], ["→"]], description: "后退 / 前进 5 秒" },
    { keys: [["Shift", "←"], ["Shift", "→"]], description: "后退 / 前进一帧" },
  ];

  const editShortcuts: Shortcut[] = [
    { keys: [["Ctrl/⌘", "B"]], description: "在播放头处分割选中的叠加内容或片段" },
    { keys: [["Delete"], ["Backspace"]], description: "删除选中的片段、字幕或叠加内容" },
    { keys: [["Ctrl/⌘", "D"]], description: "复制选中的文字、贴纸或画中画" },
    { keys: [["Enter"]], description: "修改选中的文字" },
    { keys: [["←"], ["→"], ["↑"], ["↓"]], description: "点击画面后，微调选中叠加内容的位置（Shift 加大步长）" },
    { keys: [["Ctrl/⌘", "Z"]], description: "撤销" },
    { keys: [["Ctrl/⌘", "Shift", "Z"], ["Ctrl", "Y"]], description: "重做" },
    { keys: [["Ctrl/⌘", "S"]], description: "立即保存" },
  ];
</script>

<svelte:window
  onkeydown={(event) => {
    if (show && event.key === "Escape") {
      event.preventDefault();
      event.stopImmediatePropagation();
      show = false;
    }
  }}
/>

{#snippet shortcutGroup(title: string, items: Shortcut[], hint = "")}
  <section>
    <div class="mb-2 flex items-baseline justify-between gap-2">
      <h3 class="text-xs font-medium text-gray-300">{title}</h3>
      {#if hint}
        <span class="text-[11px] text-gray-500">{hint}</span>
      {/if}
    </div>
    <ul class="space-y-2">
      {#each items as item}
        <li class="flex items-center justify-between gap-4">
          <span class="text-[13px] text-gray-300">{item.description}</span>
          <span class="flex shrink-0 items-center gap-1 text-[11px] text-gray-500">
            {#each item.keys as combo, comboIndex}
              {#if comboIndex > 0}
                <span class="px-0.5">/</span>
              {/if}
              {#each combo as key, keyIndex}
                {#if keyIndex > 0}
                  <span>+</span>
                {/if}
                <kbd
                  class="inline-flex h-5 min-w-[1.25rem] items-center justify-center rounded border border-gray-600 bg-[#2c2c2e] px-1.5 font-mono text-[11px] leading-none text-gray-200"
                  >{key}</kbd
                >
              {/each}
            {/each}
          </span>
        </li>
      {/each}
    </ul>
  </section>
{/snippet}

{#if show}
  <div class="fixed inset-0 z-[1100] flex items-center justify-center p-4">
    <button
      type="button"
      class="absolute inset-0 bg-black/60"
      aria-label="关闭快捷键说明"
      onclick={() => (show = false)}
    ></button>
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="shortcut-help-title"
      tabindex="-1"
      class="relative flex w-[720px] max-w-full max-h-[92vh] flex-col rounded-lg bg-[#1c1c1e] shadow-2xl"
    >
      <div
        class="flex shrink-0 items-center justify-between border-b border-gray-800/50 px-4 py-3"
      >
        <h2 id="shortcut-help-title" class="text-base font-medium text-white">
          快捷键说明
        </h2>
        <button
          type="button"
          class="text-gray-400 hover:text-white"
          aria-label="关闭"
          onclick={() => (show = false)}
        >
          <X class="h-5 w-5" />
        </button>
      </div>

      <div class="sidebar-scrollbar overflow-y-auto px-4 py-4">
        <div class="grid gap-x-8 gap-y-5 sm:grid-cols-2">
          {@render shortcutGroup("字幕打轴", timingShortcuts, "与 Aegisub 默认键位一致")}
          <div class="space-y-5">
            {@render shortcutGroup("播放", playbackShortcuts)}
            {@render shortcutGroup("剪辑", editShortcuts)}
          </div>
        </div>

        <ul
          class="mt-5 list-disc space-y-1 border-t border-gray-800/50 pl-4 pt-3 text-xs leading-relaxed text-gray-500"
        >
          <li>
            点击时间轴上的字幕块或列表序号选中字幕；未选中时，打轴快捷键作用于播放位置所在的字幕。
          </li>
          <li>
            拖动字幕或片段边缘时会吸附到其它字幕、片段边界和播放头，按住 Shift 拖动可暂时关闭吸附。
          </li>
          <li>字幕跟随所在片段：移动、裁剪片段时字幕随画面移动，超出片段范围的部分会被隐藏。</li>
          <li>在输入框中编辑时快捷键不生效，按 Enter 或 Esc 退出编辑。</li>
        </ul>
      </div>
    </div>
  </div>
{/if}
