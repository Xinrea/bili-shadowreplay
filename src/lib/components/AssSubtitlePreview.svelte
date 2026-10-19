<script lang="ts">
  import { untrack } from "svelte";
  import type SubtitlesOctopus from "libass-wasm";
  import { loadSubtitleFonts } from "../subtitle-fonts";
  import type { MediaClock } from "../project-player";

  interface Props {
    assContent: string;
    /** A <video>, or a clock that behaves like one, such as a project preview. */
    video?: HTMLVideoElement | MediaClock;
    fontName?: string;
  }

  let { assContent, video, fontName = "Arial" }: Props = $props();
  let canvas: HTMLCanvasElement = $state();
  let instance: SubtitlesOctopus | null = $state(null);
  let loading = $state(true);
  let error = $state("");
  let warning = $state("");
  let retry = $state(0);

  // Only font changes need a fresh worker. Text, timing and style changes use
  // setTrack, so editing a cue does not repeatedly load WASM and native fonts.
  $effect(() => {
    const target = canvas;
    const family = fontName;
    retry;
    if (!target) return;
    let disposed = false;
    let failed = false;
    let owned: SubtitlesOctopus | null = null;
    loading = true;
    error = "";
    warning = "";
    instance = null;
    const clearCanvas = () => {
      target.getContext("2d")?.clearRect(0, 0, target.width, target.height);
    };
    const disposeRenderer = () => {
      const renderer = owned;
      owned = null;
      renderer?.worker?.removeEventListener("message", onReady);
      if (renderer?.worker) renderer.dispose();
    };
    const fail = (message: string) => {
      if (disposed || failed) return;
      failed = true;
      clearTimeout(timeout);
      instance = null;
      loading = false;
      error = message;
      clearCanvas();
    };
    clearCanvas();

    // The package's onReady fires before fonts/WASM finish loading. Its styles
    // query is queued until runtime initialization, even with no active cues.
    const onReady = (event: MessageEvent) => {
      if (disposed || failed || event.data?.target !== "get-styles") return;
      clearTimeout(timeout);
      loading = false;
      instance = owned;
      owned?.worker?.removeEventListener("message", onReady);
    };
    const timeout = setTimeout(() => {
      fail("字幕渲染器或字体加载超时，请重试");
      disposeRenderer();
    }, 30_000);

    const start = setTimeout(async () => {
      try {
        if (typeof WebAssembly === "undefined" || typeof Worker === "undefined") {
          throw new Error("当前浏览器不支持 WebAssembly 字幕预览");
        }
        const [{ default: Renderer }, fonts] = await Promise.all([
          import("libass-wasm"),
          loadSubtitleFonts(family),
        ]);
        if (disposed || failed) return;
        warning = fonts.warning;
        owned = new Renderer({
          canvas: target,
          subContent: untrack(() => assContent),
          workerUrl: new URL(`${import.meta.env.BASE_URL}libass/subtitles-octopus-worker.js`, window.location.href).href,
          fonts: fonts.fonts,
          fallbackFont: fonts.fallbackFont,
          renderMode: "wasm-blend",
          targetFps: 30,
          libassMemoryLimit: 64,
          libassGlyphLimit: 32,
          onError: (cause) => {
            if (disposed) return;
            fail(`字幕预览加载失败：${cause instanceof Error ? cause.message : String(cause)}`);
            // libass-wasm disposes synchronously after calling onError. Wait
            // until it returns, then release anything it left alive. Its
            // dispose() is not safe to call twice because worker becomes null.
            queueMicrotask(() => {
              if (disposed) return;
              disposeRenderer();
              clearCanvas();
            });
          },
        });
        owned.worker?.addEventListener("message", onReady);
        owned.worker?.postMessage({ target: "get-styles" });
      } catch (cause) {
        if (disposed) return;
        fail(cause instanceof Error ? cause.message : String(cause));
        disposeRenderer();
      }
    }, 200);

    return () => {
      disposed = true;
      clearTimeout(start);
      clearTimeout(timeout);
      disposeRenderer();
      clearCanvas();
    };
  });

  $effect(() => {
    const renderer = instance;
    const content = assContent;
    if (!renderer) return;
    const timer = setTimeout(() => {
      renderer.setTrack(content);
      renderer.setCurrentTime(video?.currentTime ?? 1);
    }, 80);
    return () => clearTimeout(timer);
  });

  $effect(() => {
    const renderer = instance;
    const media = video;
    if (!renderer) return;
    if (!media) {
      renderer.setIsPaused(true, 1);
      return;
    }
    const sync = () => {
      renderer.setRate(media.playbackRate);
      renderer.setIsPaused(media.paused || media.seeking || media.readyState < 3, media.currentTime);
      renderer.setCurrentTime(media.currentTime);
    };
    const waiting = () => renderer.setIsPaused(true, media.currentTime);
    const events = ["playing", "pause", "seeking", "seeked", "timeupdate", "ratechange", "loadedmetadata", "emptied"];
    for (const event of events) media.addEventListener(event, sync);
    media.addEventListener("waiting", waiting);
    sync();
    return () => {
      for (const event of events) media.removeEventListener(event, sync);
      media.removeEventListener("waiting", waiting);
    };
  });
</script>

<div class="absolute inset-0 pointer-events-none" aria-label="libass 字幕预览">
  <canvas bind:this={canvas} width="1920" height="1080" class="absolute inset-0 w-full h-full" class:invisible={loading || Boolean(error)} aria-hidden="true"></canvas>
  {#if loading || error || warning}
    <div class="absolute top-2 right-2 max-w-[85%] rounded bg-black/75 px-2 py-1 text-xs text-gray-300" role="status">
      {#if error}
        <span class="text-amber-300">{error}</span>
        <button class="pointer-events-auto ml-2 text-blue-400 hover:text-blue-300" onclick={() => retry++}>重试</button>
      {:else if loading}
        正在加载字幕渲染器和字体…
      {:else}
        {warning}
      {/if}
    </div>
  {/if}
</div>
