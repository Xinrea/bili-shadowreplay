<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import { invoke } from "../invoker";
  import { GeneratedSubtitlePreviewQueue, parseGeneratedSubtitlePreview } from "../generated-subtitle-preview";
  import type { MediaClock } from "../project-player";

  interface Props {
    assContent: string;
    video?: HTMLVideoElement | MediaClock;
    fontName?: string;
  }

  let { assContent, video, fontName = "Arial" }: Props = $props();
  let image = $state("");
  let loading = $state(false);
  let error = $state("");
  let retry = $state(0);
  const queue = new GeneratedSubtitlePreviewQueue(
    (request) => invoke<string>("render_generated_subtitle_preview", { ...request }),
    (state) => { image = state.image; loading = state.loading; error = state.error; },
  );
  onDestroy(() => queue.dispose());

  $effect(() => {
    const content = assContent;
    const family = fontName;
    const media = video;
    const attempt = retry;
    // Do not subscribe this effect to the reactive player's currentTime. Clock
    // events and RAF sample it without rebuilding the document every frame.
    return untrack(() => {
      let document: ReturnType<typeof parseGeneratedSubtitlePreview>;
      try {
        document = parseGeneratedSubtitlePreview(content);
      } catch (cause) {
        queue.fail(cause);
        return;
      }
      let frame = 0;
      let first = true;
      const sync = () => {
        if (frame) cancelAnimationFrame(frame);
        frame = 0;
        try {
          const time = media?.currentTime ?? 1;
          queue.update(document.at(time), time, family, first && attempt > 0);
          first = false;
        } catch (cause) {
          queue.fail(cause);
        }
        if (media && !media.paused && !media.seeking && media.readyState >= 3) {
          frame = requestAnimationFrame(sync);
        }
      };
      const stop = () => {
        if (frame) cancelAnimationFrame(frame);
        frame = 0;
      };
      const clear = () => { stop(); queue.update(null, 0, family); };
      const events = ["playing", "pause", "seeking", "seeked", "timeupdate", "ratechange", "loadedmetadata", "ended"];
      for (const event of events) media?.addEventListener(event, sync);
      media?.addEventListener("waiting", stop);
      media?.addEventListener("emptied", clear);
      sync();
      return () => {
        stop();
        for (const event of events) media?.removeEventListener(event, sync);
        media?.removeEventListener("waiting", stop);
        media?.removeEventListener("emptied", clear);
        // Invalidate in-flight replies before replacement props or unmount.
        queue.update(null, 0, family);
      };
    });
  });
</script>

<div class="absolute inset-0 pointer-events-none" aria-label="原生彩色字幕预览">
  {#if image}
    <img src={image} class="absolute inset-0 w-full h-full" alt="" aria-hidden="true" />
  {/if}
  {#if loading || error}
    <div class="absolute top-2 right-2 max-w-[85%] rounded bg-black/75 px-2 py-1 text-xs text-gray-300" role="status">
      {#if error}
        <span class="text-amber-300">{error}</span>
        <button class="pointer-events-auto ml-2 text-blue-400 hover:text-blue-300" onclick={() => retry++}>重试</button>
      {:else}
        正在加载字幕渲染器和字体…
      {/if}
    </div>
  {/if}
</div>
