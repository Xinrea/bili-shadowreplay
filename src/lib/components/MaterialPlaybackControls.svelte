<script lang="ts">
  import { Maximize, MessageCircle, Minimize, Pause, Play, RotateCcw, Volume2, VolumeX } from "lucide-svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import { TAURI_ENV } from "../invoker";
  import { PLAYBACK_RATES } from "../playback-rates";
  import { formatTimecode } from "../time-format";

  interface Props {
    video: HTMLVideoElement | undefined;
    container: HTMLElement | undefined;
    fullscreen?: boolean;
    showDanmu?: boolean;
    hasDanmu: boolean;
  }

  let { video, container, fullscreen = $bindable(false), showDanmu = $bindable(true), hasDanmu }: Props = $props();
  let paused = $state(true);
  let ended = $state(false);
  let currentTime = $state(0);
  let duration = $state(0);
  let volume = $state(1);
  let muted = $state(false);
  let playbackRate = $state(1);
  let waiting = $state(false);
  let mediaError = $state("");
  let actionError = $state("");
  let fullscreenBusy = $state(false);
  let canFullscreen = $state(TAURI_ENV);
  let disposed = false;
  let ownsNativeFullscreen = false;
  let isMuted = $derived(muted || volume === 0);
  let canSeek = $derived(duration > 0 && !mediaError);
  let progress = $derived(duration > 0 ? Math.min(100, currentTime / duration * 100) : 0);

  function time(seconds: number) {
    return formatTimecode(Number.isFinite(seconds) ? Math.floor(seconds) : 0, 0);
  }

  // Media events are the source of truth, including seeks from the danmaku list.
  $effect(() => {
    const media = video;
    if (!media) return;
    const sync = () => {
      paused = media.paused;
      ended = media.ended;
      currentTime = Number.isFinite(media.currentTime) ? media.currentTime : 0;
      duration = Number.isFinite(media.duration) && media.duration > 0 ? media.duration : 0;
      volume = media.volume;
      muted = media.muted;
      playbackRate = media.playbackRate;
      mediaError = media.error ? "无法播放此素材，请检查文件是否存在及视频编码是否受支持。" : "";
    };
    const onWaiting = () => { waiting = true; };
    const onReady = () => { waiting = false; sync(); };
    const onReset = () => { waiting = false; actionError = ""; sync(); };
    const events = ["loadedmetadata", "durationchange", "timeupdate", "play", "pause", "ended", "volumechange", "ratechange"];
    const readyEvents = ["playing", "canplay", "seeked", "error"];
    events.forEach((event) => media.addEventListener(event, sync));
    readyEvents.forEach((event) => media.addEventListener(event, onReady));
    media.addEventListener("waiting", onWaiting);
    media.addEventListener("loadstart", onReset);
    media.addEventListener("emptied", onReset);
    sync();
    return () => {
      events.forEach((event) => media.removeEventListener(event, sync));
      readyEvents.forEach((event) => media.removeEventListener(event, onReady));
      media.removeEventListener("waiting", onWaiting);
      media.removeEventListener("loadstart", onReset);
      media.removeEventListener("emptied", onReset);
    };
  });

  export async function togglePlayback() {
    if (!video || mediaError) return;
    actionError = "";
    if (!video.paused) {
      video.pause();
      return;
    }
    try {
      if (video.ended) video.currentTime = 0;
      await video.play();
    } catch (error) {
      // A pause or source change can legitimately interrupt a pending play().
      if (!(error instanceof DOMException && error.name === "AbortError")) {
        actionError = "播放未能开始，请再次点击播放。";
      }
    }
  }

  function seek(seconds: number) {
    if (!video || !canSeek || !Number.isFinite(seconds)) return;
    video.currentTime = Math.max(0, Math.min(duration, seconds));
    currentTime = video.currentTime;
  }

  function setVolume(value: number) {
    if (!video) return;
    video.volume = Math.max(0, Math.min(1, value));
    video.muted = false;
  }

  function toggleMute() {
    if (!video) return;
    if (video.muted || video.volume === 0) {
      if (video.volume === 0) video.volume = 0.5;
      video.muted = false;
    } else {
      video.muted = true;
    }
  }

  async function syncNativeFullscreen() {
    try {
      const value = await getCurrentWindow().isFullscreen();
      if (!disposed) fullscreen = value;
    } catch {
      if (!disposed) actionError = "无法读取全屏状态。";
    }
  }

  function syncFullscreen() {
    if (!TAURI_ENV) fullscreen = document.fullscreenElement === container;
  }

  async function restoreNativeFullscreen() {
    if (!ownsNativeFullscreen) return;
    ownsNativeFullscreen = false;
    try {
      await getCurrentWindow().setFullscreen(false);
    } catch (error) {
      console.warn("恢复素材预览窗口状态失败", error);
    }
  }

  export async function toggleFullscreen() {
    if (!container || !canFullscreen || fullscreenBusy) return;
    fullscreenBusy = true;
    actionError = "";
    try {
      if (TAURI_ENV) {
        const window = getCurrentWindow();
        const wasFullscreen = await window.isFullscreen();
        if (disposed) return;
        await window.setFullscreen(!wasFullscreen);
        ownsNativeFullscreen = !wasFullscreen;
        if (disposed) await restoreNativeFullscreen();
        else await syncNativeFullscreen();
      } else if (document.fullscreenElement === container) {
        await document.exitFullscreen();
      } else {
        await container.requestFullscreen();
      }
    } catch {
      actionError = "无法切换全屏，请重试。";
    } finally {
      fullscreenBusy = false;
    }
  }

  onMount(() => {
    canFullscreen = TAURI_ENV || document.fullscreenEnabled;
    if (!TAURI_ENV) return;
    let unlisten: (() => void) | undefined;
    void getCurrentWindow().onResized(() => void syncNativeFullscreen()).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    }).catch(() => { actionError = "无法监听全屏状态变化。"; });
    void syncNativeFullscreen();
    return () => {
      disposed = true;
      unlisten?.();
      fullscreen = false;
      void restoreNativeFullscreen();
    };
  });

  $effect(() => {
    if (!fullscreen || !container) return;
    const previousFocus = document.activeElement;
    if (!container.contains(previousFocus)) container.querySelector("button")?.focus();
    return () => {
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus();
    };
  });

  function onKeydown(event: KeyboardEvent) {
    if (event.defaultPrevented || event.isComposing || event.altKey || event.ctrlKey || event.metaKey || event.repeat) return;
    if (event.key === "Escape" && TAURI_ENV && fullscreen) {
      event.preventDefault();
      void toggleFullscreen();
      return;
    }
    const target = event.target;
    if (!(target instanceof Element) || !container?.contains(target)) return;
    if (target.closest("input, select, textarea, [contenteditable]")) return;
    switch (event.key.toLowerCase()) {
      case "k": void togglePlayback(); break;
      case "m": toggleMute(); break;
      case "f": void toggleFullscreen(); break;
      case "arrowleft": seek(currentTime - 5); break;
      case "arrowright": seek(currentTime + 5); break;
      default: return;
    }
    event.preventDefault();
  }
</script>

<svelte:document onfullscreenchange={syncFullscreen} />
<svelte:window onkeydown={onKeydown} />

<div class="playback-controls">
  {#if mediaError || actionError}
    <p class="playback-error" role="alert">{mediaError || actionError}</p>
  {/if}
  <input
    class="seek-slider"
    type="range"
    aria-label="播放进度"
    aria-valuetext={`${time(currentTime)} / ${time(duration)}`}
    min="0"
    max={duration || 1}
    step="0.1"
    value={currentTime}
    disabled={!canSeek}
    style:--progress={`${progress}%`}
    oninput={(event) => seek(Number(event.currentTarget.value))}
  />
  <div class="control-row">
    <div class="control-group">
      <button class="control-button play-button" type="button" disabled={!video || !!mediaError}
        aria-label={ended ? "重新播放" : paused ? "播放" : "暂停"}
        title={`${ended ? "重新播放" : paused ? "播放" : "暂停"} (K)`} onclick={togglePlayback}>
        {#if ended}<RotateCcw size={17} />{:else if paused}<Play size={17} fill="currentColor" />{:else}<Pause size={17} fill="currentColor" />{/if}
      </button>
      <span class="time-display">{time(currentTime)} <span>/ {time(duration)}</span></span>
      {#if waiting && !paused && !mediaError}<span class="loading-status" role="status">缓冲中…</span>{/if}
    </div>
    <div class="control-group secondary-controls">
      <div class="volume-control">
        <button class="control-button" type="button" aria-label="静音" aria-pressed={isMuted}
          title={`${isMuted ? "取消静音" : "静音"} (M)`} onclick={toggleMute}>
          {#if isMuted}<VolumeX size={17} />{:else}<Volume2 size={17} />{/if}
        </button>
        <input class="volume-slider" type="range" aria-label="音量" aria-valuetext={`${Math.round((isMuted ? 0 : volume) * 100)}%`}
          min="0" max="1" step="0.01" value={isMuted ? 0 : volume}
          style:--progress={`${(isMuted ? 0 : volume) * 100}%`}
          oninput={(event) => setVolume(Number(event.currentTarget.value))} />
      </div>
      <select aria-label="播放速度" title="播放速度" value={playbackRate}
        onchange={(event) => { if (video) video.playbackRate = Number(event.currentTarget.value); }}>
        {#each PLAYBACK_RATES as rate}<option value={rate}>{rate}×</option>{/each}
      </select>
      <button class="control-button" class:enabled={hasDanmu && showDanmu} type="button"
        aria-label="画面弹幕" aria-pressed={hasDanmu && showDanmu}
        title={hasDanmu ? (showDanmu ? "隐藏画面弹幕" : "显示画面弹幕") : "此素材没有弹幕"}
        disabled={!hasDanmu} onclick={() => (showDanmu = !showDanmu)}>
        <MessageCircle size={17} />
      </button>
      {#if canFullscreen}
        <button class="control-button" type="button" aria-label={fullscreen ? "退出全屏" : "全屏"}
          title={`${fullscreen ? "退出全屏" : "全屏"} (F)`} disabled={fullscreenBusy} onclick={toggleFullscreen}>
          {#if fullscreen}<Minimize size={17} />{:else}<Maximize size={17} />{/if}
        </button>
      {/if}
    </div>
  </div>
</div>

<style>
  .playback-controls {
    flex-shrink: 0;
    padding: 7px 14px 10px;
    border-top: 1px solid #2b3340;
    background: #171c25;
    color: #e9eef8;
    color-scheme: dark;
  }
  .control-row, .control-group, .volume-control {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .control-row { justify-content: space-between; flex-wrap: wrap; gap: 6px 16px; }
  .secondary-controls { margin-left: auto; }
  .control-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: 30px;
    height: 30px;
    border: 1px solid transparent;
    border-radius: 8px;
    background: transparent;
    color: #b6c0cf;
  }
  .control-button:hover:not(:disabled) { background: #293341; color: #e9eef8; }
  .control-button:disabled, input:disabled { opacity: 0.4; cursor: not-allowed; }
  .play-button { background: #0a84ff; color: white; }
  .play-button:hover:not(:disabled) { background: #0076eb; color: white; }
  .enabled { color: #65c2ff; background: #153b5c; border-color: #0a84ff70; }
  .time-display { margin-left: 4px; font-size: 11px; font-variant-numeric: tabular-nums; white-space: nowrap; }
  .time-display span, .loading-status { color: #8c98aa; }
  .loading-status { font-size: 11px; }
  select {
    height: 28px;
    min-width: 62px;
    padding: 0 22px 0 8px;
    border: 1px solid #344052;
    border-radius: 7px;
    background-color: #202733;
    background-position: right 4px center;
    color: #e9eef8;
    font-size: 11px;
    cursor: pointer;
  }
  input[type="range"] {
    display: block;
    appearance: none;
    height: 20px;
    margin: 0;
    border: 0;
    padding: 0;
    background: transparent;
    cursor: pointer;
    box-shadow: none;
  }
  input.seek-slider { width: 100%; margin-bottom: 5px; }
  .volume-slider { width: 64px; }
  input[type="range"]::-webkit-slider-runnable-track {
    height: 3px;
    border-radius: 3px;
    background: linear-gradient(to right, #0a84ff var(--progress), #344052 var(--progress));
  }
  input[type="range"]::-moz-range-track {
    height: 3px;
    border-radius: 3px;
    background: linear-gradient(to right, #0a84ff var(--progress), #344052 var(--progress));
  }
  input[type="range"]::-webkit-slider-thumb {
    appearance: none;
    width: 10px;
    height: 10px;
    margin-top: -3.5px;
    border: 0;
    border-radius: 50%;
    background: #e9eef8;
  }
  input[type="range"]::-moz-range-thumb {
    width: 10px;
    height: 10px;
    border: 0;
    border-radius: 50%;
    background: #e9eef8;
  }
  button:focus-visible, select:focus-visible, input:focus-visible {
    outline: 2px solid #0a84ff;
    outline-offset: 2px;
  }
  .playback-error { margin: 3px 0 6px; color: #ff8591; font-size: 12px; }
</style>
