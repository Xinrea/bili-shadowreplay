<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import type { VideoOverlay } from "../../interface";
  import { VideoFollower } from "../../project-player";
  import type { PlacedOverlay } from "../../project-timeline";

  // One video overlay of the preview, kept on the project's clock: it plays
  // while the project plays over it, and shows the right frame when paused.
  // It is always muted: the preview's sound plays its sound, as every other.

  interface Props {
    placed: PlacedOverlay;
    url: string;
    /** Project time. */
    time: number;
    playing: boolean;
    /** The project's clock waits to start, which an overlay waits for where it is. */
    waiting: boolean;
    /** CSS pixels per pixel of the 1920×1080 frame. */
    unit: number;
    /** Width over height, until the video knows its own. */
    aspect: number;
    onAspect: (aspect: number) => void;
  }

  let { placed, url, time, playing, waiting, unit, aspect, onAspect }: Props = $props();

  /** How far before the end of the file its last frame is held. */
  const LAST_FRAME = 0.05;

  let video: HTMLVideoElement = $state();
  let follower: VideoFollower | null = null;
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
    follower ??= new VideoFollower(video);
    // A material's stored length can run a little past what its file plays.
    // There the last frame is held: playing an ended video restarts it.
    const last = Number.isFinite(video.duration) && video.duration > 0 ? video.duration - LAST_FRAME : Infinity;
    if (active && playing && waiting) follower.hold();
    else if (active && playing) follower.play(target(), last);
    // Waiting to come in, it holds its first frame.
    else follower.rest(Math.min(target(), last));
  }

  $effect(() => {
    time;
    playing;
    waiting;
    placed;
    untrack(sync);
  });

  // A detached element would go on playing.
  onDestroy(() => {
    follower?.dispose();
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
  muted
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
