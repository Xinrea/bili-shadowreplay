<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import type { VideoOverlay } from "../../interface";
  import type { PlacedOverlay } from "../../project-timeline";

  // One video overlay of the preview, kept on the project's clock: it plays
  // while the project plays over it, and shows the right frame when paused.

  interface Props {
    placed: PlacedOverlay;
    url: string;
    /** Project time. */
    time: number;
    playing: boolean;
    /** CSS pixels per pixel of the 1920×1080 frame. */
    unit: number;
    /** Width over height, until the video knows its own. */
    aspect: number;
    volume: number;
    muted: boolean;
    onAspect: (aspect: number) => void;
  }

  let { placed, url, time, playing, unit, aspect, volume, muted, onAspect }: Props = $props();

  // Within this much a playing overlay is left alone; seeking it stutters.
  const PLAYING_DRIFT = 0.3;
  const PAUSED_DRIFT = 0.02;
  /** How far before the end of the file its last frame is held. */
  const LAST_FRAME = 0.05;

  let video: HTMLVideoElement = $state();
  let item = $derived(placed.item as VideoOverlay);
  let active = $derived(time >= placed.start && time < placed.end);
  let width = $derived(item.element.width * unit);
  let height = $derived(width / aspect);

  /** Material time to show at the project's time. */
  function target() {
    const into = Math.min(Math.max(time - placed.start, 0), placed.end - placed.start);
    return item.source_start + placed.skip + into;
  }

  function sync() {
    if (!video || video.readyState < 1) return;
    // A material's stored length can run a little past what its file plays.
    // There the last frame is held: playing an ended video restarts it.
    const last = Number.isFinite(video.duration) && video.duration > 0 ? video.duration - LAST_FRAME : Infinity;
    const to = Math.min(target(), last);
    const drift = Math.abs(video.currentTime - to);
    if (active && playing && to < last) {
      if (video.paused) {
        if (drift > PAUSED_DRIFT) video.currentTime = to;
        video.play().catch(() => {});
      } else if (drift > PLAYING_DRIFT && !video.seeking) {
        video.currentTime = to;
      }
    } else {
      if (!video.paused) video.pause();
      // Waiting to come in, it holds its first frame.
      if (drift > PAUSED_DRIFT && !video.seeking) video.currentTime = to;
    }
  }

  $effect(() => {
    time;
    playing;
    placed;
    untrack(sync);
  });

  $effect(() => {
    if (!video) return;
    video.volume = Math.min(1, Math.max(0, volume * item.volume));
    video.muted = muted || item.volume <= 0;
  });

  // A detached element would go on playing.
  onDestroy(() => {
    if (!video) return;
    video.pause();
    video.removeAttribute("src");
    video.load();
  });
</script>

<!-- svelte-ignore a11y_media_has_caption -->
<video
  bind:this={video}
  src={url}
  preload="auto"
  playsinline
  class="overlay-video"
  class:visible={active}
  style:left="{item.element.x * unit - width / 2}px"
  style:top="{item.element.y * unit - height / 2}px"
  style:width="{width}px"
  style:height="{height}px"
  style:transform="rotate({item.element.rotation}deg)"
  style:opacity={item.opacity ?? 1}
  onloadedmetadata={() => {
    if (video.videoWidth && video.videoHeight) onAspect(video.videoWidth / video.videoHeight);
    sync();
  }}
  onseeked={sync}
></video>

<style>
  .overlay-video {
    position: absolute;
    object-fit: contain;
    visibility: hidden;
    pointer-events: none;
  }

  .overlay-video.visible {
    visibility: visible;
  }
</style>
