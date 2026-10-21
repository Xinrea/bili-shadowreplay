<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import { fadeGain, type PlacedAudio } from "../../project-timeline";

  // One audio item of the preview, kept on the project's clock like a video
  // overlay: it plays while the project plays over it, and waits at the right
  // place when paused.

  interface Props {
    placed: PlacedAudio;
    url: string;
    /** Project time. */
    time: number;
    playing: boolean;
    volume: number;
    muted: boolean;
  }

  let { placed, url, time, playing, volume, muted }: Props = $props();

  // Further off than this a playing sound is seeked, which is heard. Further
  // off than `CATCH_UP_DRIFT` it plays a little faster or slower until it is
  // within `IN_STEP_DRIFT` again, as a sound that starts some time after it is
  // asked to otherwise stays behind. Media elements tell their time in steps,
  // which the two apart leave room for.
  const PLAYING_DRIFT = 0.15;
  const PAUSED_DRIFT = 0.02;
  const CATCH_UP_DRIFT = 0.05;
  const IN_STEP_DRIFT = 0.01;
  const CATCH_UP_RATE = 0.05;

  let audio: HTMLAudioElement = $state();
  let catchingUp = false;
  let active = $derived(time >= placed.start && time < placed.end);

  /** Source time to play at the project's time. */
  function target() {
    const into = Math.min(Math.max(time - placed.start, 0), placed.end - placed.start);
    return placed.item.source_start + placed.skip + into;
  }

  function sync() {
    if (!audio || audio.readyState < 1) return;
    // Past the end of its file it is silent, as in export.
    const end = Number.isFinite(audio.duration) && audio.duration > 0 ? audio.duration : Infinity;
    const to = target();
    const drift = Math.abs(audio.currentTime - to);
    if (active && playing && to < end) {
      if (audio.paused) {
        if (drift > PAUSED_DRIFT) audio.currentTime = to;
        audio.play().catch(() => {});
      } else if (drift > PLAYING_DRIFT && !audio.seeking) {
        audio.currentTime = to;
        rate(1);
      } else if (drift > CATCH_UP_DRIFT || (catchingUp && drift > IN_STEP_DRIFT)) {
        rate(audio.currentTime > to ? 1 - CATCH_UP_RATE : 1 + CATCH_UP_RATE);
      } else {
        rate(1);
      }
    } else {
      rate(1);
      if (!audio.paused) audio.pause();
      // Waiting to come in, it is ready at its first sound.
      if (drift > PAUSED_DRIFT && !audio.seeking && to < end) audio.currentTime = to;
    }
  }

  function rate(value: number) {
    catchingUp = value !== 1;
    if (audio.playbackRate !== value) audio.playbackRate = value;
  }

  $effect(() => {
    time;
    playing;
    placed;
    untrack(sync);
  });

  $effect(() => {
    if (!audio) return;
    audio.volume = Math.min(1, Math.max(0, volume * placed.item.volume * fadeGain(placed, time)));
    audio.muted = muted || placed.item.volume <= 0;
  });

  // A detached element would go on playing.
  onDestroy(() => {
    if (!audio) return;
    audio.pause();
    audio.removeAttribute("src");
    audio.load();
  });
</script>

<audio bind:this={audio} src={url} preload="auto" onloadedmetadata={sync} onseeked={sync}></audio>
