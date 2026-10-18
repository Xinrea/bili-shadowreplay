<script lang="ts">
  import type { Component } from "svelte";
  import BilibiliIcon from "./BilibiliIcon.svelte";
  import DouyinIcon from "./DouyinIcon.svelte";
  import DouyuIcon from "./DouyuIcon.svelte";
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
    douyu: DouyuIcon,
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

<div class="space-y-1.5">
  <span class="ui-label block">平台</span>
  <div
    class="grid grid-cols-[repeat(auto-fit,minmax(4.25rem,1fr))] gap-0.5 rounded-[9px] bg-sunken p-[3px]"
    role="radiogroup"
    aria-label="平台"
  >
    {#each platforms as platform (platform.id)}
      {@const Icon = icons[platform.id]}
      <button
        type="button"
        class="flex min-w-0 flex-col items-center justify-center gap-1 rounded-[7px] px-1 py-2 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent {value ===
        platform.id
          ? 'bg-selected text-fg shadow-sm'
          : 'text-fg-muted hover:bg-selected/60 hover:text-fg-secondary'}"
        aria-label={platform.label}
        aria-checked={value === platform.id}
        role="radio"
        onclick={() => select(platform.id)}
      >
        <span class="text-fg">
          {#if Icon}
            <Icon class="w-5 h-5 object-contain" />
          {/if}
        </span>
        <span class="w-full text-[11px] leading-none text-center truncate">{platform.label}</span>
      </button>
    {/each}
  </div>
</div>
