<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    count: number;
    itemLabel: string;
    onclear: () => void;
    actions: Snippet;
    children: Snippet;
  }

  let { count, itemLabel, onclear, actions, children }: Props = $props();
  let barHeight = $state(0);
</script>

<div class="relative flex min-h-0 flex-1 flex-col bg-canvas">
  <div
    class="min-h-0 flex-1 overflow-auto p-5 custom-scrollbar-light"
    style:padding-bottom={count > 0 ? `${barHeight + 40}px` : undefined}
  >
    {@render children()}
  </div>

  {#if count > 0}
    <div class="pointer-events-none absolute inset-x-5 bottom-5 z-20 flex justify-center">
      <div
        bind:offsetHeight={barHeight}
        role="group"
        aria-label={`${itemLabel}批量操作`}
        class="pointer-events-auto flex max-w-full flex-wrap items-center justify-center gap-x-4 gap-y-2 rounded-[12px] border border-line-strong bg-surface/95 px-4 py-3 shadow-xl backdrop-blur-sm"
      >
        <span class="whitespace-nowrap text-[12px] text-fg-secondary">已选择 {count} 个{itemLabel}</span>
        <div class="flex flex-wrap items-center justify-center gap-2">
          <button class="ui-btn" onclick={onclear}>取消选择</button>
          {@render actions()}
        </div>
      </div>
    </div>
  {/if}
</div>
