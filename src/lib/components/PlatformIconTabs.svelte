<script lang="ts">
  import type { Component } from "svelte";
  import BilibiliIcon from "./BilibiliIcon.svelte";
  import DouyinIcon from "./DouyinIcon.svelte";
  import HuyaIcon from "./HuyaIcon.svelte";
  import KuaishouIcon from "./KuaishouIcon.svelte";
  import TikTokIcon from "./TikTokIcon.svelte";
  import TwitchIcon from "./TwitchIcon.svelte";
  import YouTubeIcon from "./YouTubeIcon.svelte";

  interface PlatformOption {
    id: string;
    label: string;
  }

  interface Props {
    platforms: PlatformOption[];
    value?: string;
    onchange?: (id: string) => void;
  }

  let { platforms, value = $bindable("bilibili"), onchange }: Props = $props();

  const icons: Record<string, Component<{ class?: string }>> = {
    bilibili: BilibiliIcon,
    douyin: DouyinIcon,
    huya: HuyaIcon,
    kuaishou: KuaishouIcon,
    tiktok: TikTokIcon,
    twitch: TwitchIcon,
    youtube: YouTubeIcon,
  };

  function select(id: string) {
    value = id;
    onchange?.(id);
  }
</script>

<div class="space-y-2">
  <span class="block text-sm font-medium text-gray-700 dark:text-gray-300">平台</span>
  <div
    class="grid grid-cols-[repeat(auto-fit,minmax(4.25rem,1fr))] gap-1 p-1 bg-[#f5f5f7] dark:bg-[#1c1c1e] rounded-lg"
    role="radiogroup"
    aria-label="平台"
  >
    {#each platforms as platform (platform.id)}
      {@const Icon = icons[platform.id]}
      <button
        type="button"
        class="flex flex-col items-center justify-center gap-1 min-w-0 px-1 py-2 rounded-md transition-colors {value ===
        platform.id
          ? 'bg-white dark:bg-[#3a3a3c] shadow-sm text-gray-900 dark:text-white'
          : 'text-gray-500 dark:text-gray-400 hover:bg-white/70 dark:hover:bg-white/5 hover:text-gray-900 dark:hover:text-white'}"
        aria-label={platform.label}
        aria-checked={value === platform.id}
        role="radio"
        onclick={() => select(platform.id)}
      >
        <span class="text-gray-900 dark:text-white">
          {#if Icon}
            <Icon class="w-5 h-5 object-contain" />
          {/if}
        </span>
        <span class="w-full text-[11px] leading-none text-center truncate">{platform.label}</span>
      </button>
    {/each}
  </div>
</div>
