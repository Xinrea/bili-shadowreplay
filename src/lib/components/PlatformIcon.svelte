<script lang="ts">
  import { Globe } from "lucide-svelte";
  import BilibiliIcon from "./BilibiliIcon.svelte";
  import DouyinIcon from "./DouyinIcon.svelte";
  import DouyuIcon from "./DouyuIcon.svelte";
  import HuyaIcon from "./HuyaIcon.svelte";
  import KuaishouIcon from "./KuaishouIcon.svelte";
  import TikTokIcon from "./TikTokIcon.svelte";
  import TwitchIcon from "./TwitchIcon.svelte";
  import YouTubeIcon from "./YouTubeIcon.svelte";

  let { platform, class: className = "h-4 w-4" }: { platform: string; class?: string } = $props();
  const icons = {
    bilibili: BilibiliIcon, douyin: DouyinIcon, douyu: DouyuIcon, huya: HuyaIcon,
    kuaishou: KuaishouIcon, tiktok: TikTokIcon, twitch: TwitchIcon, youtube: YouTubeIcon,
  };
  const Icon = $derived(icons[platform] ?? Globe);
</script>

<Icon class={className} />
