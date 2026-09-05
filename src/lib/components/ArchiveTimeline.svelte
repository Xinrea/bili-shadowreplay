<script lang="ts">
  import {
    Diamond,
    MessageCircle,
    Pause,
    Play,
    Plus,
    Settings,
    Volume2,
    VolumeX,
  } from "lucide-svelte";
  import type { Marker, Range } from "../interface";

  type HeatPoint = {
    time: number;
    count: number;
    level: "normal" | "extension" | "core";
  };

  interface Props {
    ranges?: Range[];
    markers?: Marker[];
    heatPoints?: HeatPoint[];
    heatThreshold?: number;
    heatThresholdPercent?: number;
    currentTime?: number;
    duration?: number;
    selectedRangeIndex?: number;
    onSeek?: (seconds: number) => void;
    onAddRange?: () => void;
    onAddMarker?: () => void;
    onRangeDragStart?: () => void;
    onRangeDrag?: (seconds: number) => void;
    isPlaying?: boolean;
    volume?: number;
    danmuEnabled?: boolean;
    danmuOffset?: number;
    canSendDanmaku?: boolean;
    onTogglePlayback?: () => void;
    onVolumeChange?: (volume: number) => void;
    onToggleDanmu?: () => void;
    onDanmuOffsetChange?: (offset: number) => void;
    onSendDanmaku?: (message: string) => void;
  }

  let {
    ranges = $bindable([]),
    markers = [],
    heatPoints = [],
    heatThreshold = 0,
    heatThresholdPercent = 80,
    currentTime = 0,
    duration = 0,
    selectedRangeIndex = $bindable(-1),
    onSeek,
    onAddRange,
    onAddMarker,
    onRangeDragStart,
    onRangeDrag,
    isPlaying = false,
    volume = 1,
    danmuEnabled = true,
    danmuOffset = 0,
    canSendDanmaku = false,
    onTogglePlayback,
    onVolumeChange,
    onToggleDanmu,
    onDanmuOffsetChange,
    onSendDanmaku,
  }: Props = $props();
  let danmu_message = $state("");
  let show_danmu_input = $state(false);
  let show_offset_settings = $state(false);

  let visibleHeatPoints = $derived(
    heatPoints.filter(
      (point) => point.time >= 0 && (duration <= 0 || point.time <= duration),
    ),
  );
  let maxHeat = $derived(
    visibleHeatPoints.reduce(
      (maximum, point) => Math.max(maximum, point.count),
      0,
    ),
  );
  let activeRanges = $derived(
    ranges.filter((range) => range.activated !== false),
  );
  let activeDuration = $derived(
    activeRanges.reduce(
      (total, range) => total + Math.max(0, range.end - range.start),
      0,
    ),
  );

  function clampPercent(seconds: number) {
    if (duration <= 0) return 0;
    return Math.min(100, Math.max(0, (seconds / duration) * 100));
  }

  function heatHeight(count: number) {
    if (maxHeat <= 0) return 0;
    return Math.max(3, (count / maxHeat) * 26);
  }

  function heatBarWidth(index: number) {
    if (duration <= 0 || visibleHeatPoints.length < 2) return 0.4;
    const adjacentPoint =
      visibleHeatPoints[index + 1] ||
      visibleHeatPoints[index - 1] ||
      visibleHeatPoints[index];
    const gap = Math.abs(adjacentPoint.time - visibleHeatPoints[index].time);
    return Math.max(0.15, (gap / duration) * 100);
  }

  let thresholdPosition = $derived(
    Math.min(100, Math.max(0, heatThresholdPercent)),
  );
  let range_track: HTMLElement;
  let drag_state: { index: number; edge: "start" | "end" } | null = null;

  function formatTime(seconds: number) {
    const total = Math.max(0, Math.floor(seconds || 0));
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const rest = total % 60;
    return [hours, minutes, rest]
      .map((part) => part.toString().padStart(2, "0"))
      .join(":");
  }

  function handleTrackClick(event: MouseEvent) {
    if (duration <= 0) return;
    const target = event.currentTarget as HTMLElement;
    const rect = target.getBoundingClientRect();
    const ratio = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
    onSeek?.(ratio * duration);
  }

  function selectRange(event: MouseEvent, index: number, start: number) {
    event.stopPropagation();
    selectedRangeIndex = index;
    onSeek?.(start);
  }

  function beginRangeDrag(
    event: PointerEvent,
    index: number,
    edge: "start" | "end",
  ) {
    event.preventDefault();
    event.stopPropagation();
    drag_state = { index, edge };
    selectedRangeIndex = index;
    onRangeDragStart?.();
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function updateRangeDrag(event: PointerEvent) {
    if (!drag_state || !range_track || duration <= 0) return;
    const rect = range_track.getBoundingClientRect();
    const ratio = Math.min(
      1,
      Math.max(0, (event.clientX - rect.left) / rect.width),
    );
    const seconds = ratio * duration;
    const { index, edge } = drag_state;
    const range = ranges[index];
    if (!range) return;

    const nextRange =
      edge === "start"
        ? { ...range, start: Math.min(seconds, range.end) }
        : { ...range, end: Math.max(seconds, range.start) };
    ranges = ranges.map((item, rangeIndex) =>
      rangeIndex === index ? nextRange : item,
    );
    onRangeDrag?.(edge === "start" ? nextRange.start : nextRange.end);
  }

  function endRangeDrag(event: PointerEvent) {
    if (!drag_state) return;
    const target = event.currentTarget as HTMLElement;
    if (target.hasPointerCapture(event.pointerId)) {
      target.releasePointerCapture(event.pointerId);
    }
    drag_state = null;
  }

  function submitDanmu() {
    const message = danmu_message.trim();
    if (!message) return;
    onSendDanmaku?.(message);
    danmu_message = "";
  }
</script>

<section class="timeline-panel" aria-label="片段时间线">
  <div class="timeline-header">
    <div class="timeline-controls">
      <button
        type="button"
        class="control-icon playback-control"
        title={isPlaying ? "暂停" : "播放"}
        onclick={onTogglePlayback}
      >
        {#if isPlaying}<Pause size={15} />{:else}<Play size={15} />{/if}
      </button>
      <button
        type="button"
        class="control-icon"
        title={danmuEnabled ? "关闭弹幕预览" : "开启弹幕预览"}
        class:enabled={danmuEnabled}
        onclick={onToggleDanmu}
      >
        <MessageCircle size={15} />
      </button>
      {#if canSendDanmaku}
        <div
          class="danmu-sender"
          class:expanded={show_danmu_input}
          role="group"
          aria-label="发送弹幕"
          onmouseenter={() => (show_danmu_input = true)}
          onmouseleave={() => {
            if (!danmu_message) show_danmu_input = false;
          }}
        >
          <button
            type="button"
            class="control-icon"
            title="发送弹幕"
            onclick={() => (show_danmu_input = true)}
          >
            <MessageCircle size={15} />
          </button>
          {#if show_danmu_input}
            <input
              bind:value={danmu_message}
              placeholder="回车发送弹幕"
              onkeydown={(event) => {
                if (event.key === "Enter") submitDanmu();
                if (event.key === "Escape") {
                  danmu_message = "";
                  show_danmu_input = false;
                }
              }}
            />
          {/if}
        </div>
      {/if}
      <div class="volume-control">
        {#if volume === 0}
          <VolumeX size={15} />
        {:else}
          <Volume2 size={15} />
        {/if}
        <input
          type="range"
          min="0"
          max="1"
          step="0.01"
          value={volume}
          aria-label="音量"
          oninput={(event) =>
            onVolumeChange?.(
              Number((event.currentTarget as HTMLInputElement).value),
            )}
        />
      </div>
      <span class="time-display">
        {formatTime(currentTime)} / {formatTime(duration)}
      </span>
    </div>
    <div class="timeline-actions">
      <button type="button" onclick={onAddRange}>
        <Plus size={14} />
        新建选区
      </button>
      <button type="button" onclick={onAddMarker}>
        <Diamond size={12} />
        添加标记
      </button>
      <div class="settings-control">
        <button
          type="button"
          class="control-icon"
          class:enabled={show_offset_settings}
          title="弹幕偏移设置"
          aria-expanded={show_offset_settings}
          onclick={() => (show_offset_settings = !show_offset_settings)}
        >
          <Settings size={15} />
        </button>
        {#if show_offset_settings}
          <label class="offset-control">
            <span>偏移</span>
            <input
              type="number"
              value={danmuOffset}
              onchange={(event) =>
                onDanmuOffsetChange?.(
                  Number((event.currentTarget as HTMLInputElement).value),
                )}
            />
            <span>秒</span>
          </label>
        {/if}
      </div>
    </div>
  </div>

  <div class="timeline-surface">
    <div class="time-grid">
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        class="heat-row"
        role="slider"
        aria-label="弹幕热度时间轴"
        aria-valuemin="0"
        aria-valuemax={duration}
        aria-valuenow={currentTime}
        tabindex="0"
        onclick={handleTrackClick}
        onkeydown={(event) => {
          if (event.key === "ArrowLeft") onSeek?.(Math.max(0, currentTime - 3));
          if (event.key === "ArrowRight")
            onSeek?.(Math.min(duration, currentTime + 3));
        }}
      >
        <span class="heat-label">弹幕热度</span>
        <div class="heat-bars" aria-hidden="true">
          {#each visibleHeatPoints as point, index}
            <span
              class="heat-bar"
              class:extension={point.level === "extension"}
              class:hot={point.level === "core"}
              style:left={`${clampPercent(point.time)}%`}
              style:width={`${heatBarWidth(index)}%`}
              style:height={`${heatHeight(point.count)}px`}
            ></span>
          {/each}
          {#if heatThreshold > 0 && maxHeat > 0}
            <span
              class="threshold-line"
              style:bottom={`${thresholdPosition}%`}
              title={`智能推荐阈值：${Math.ceil(heatThreshold)} 条 / 30 秒`}
            ></span>
          {/if}
        </div>
      </div>

      <div class="time-ruler" aria-hidden="true">
        <span>{formatTime(0)}</span>
        <span>{formatTime(duration * 0.25)}</span>
        <span>{formatTime(duration * 0.5)}</span>
        <span>{formatTime(duration * 0.75)}</span>
        <span>{formatTime(duration)}</span>
      </div>

      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        bind:this={range_track}
        class="range-track"
        role="slider"
        aria-label="录播时间线"
        aria-valuemin="0"
        aria-valuemax={duration}
        aria-valuenow={currentTime}
        tabindex="0"
        onclick={handleTrackClick}
        onkeydown={(event) => {
          if (event.key === "ArrowLeft") onSeek?.(Math.max(0, currentTime - 3));
          if (event.key === "ArrowRight")
            onSeek?.(Math.min(duration, currentTime + 3));
        }}
      >
        {#each ranges as range, index}
          <button
            type="button"
            class="range-block"
            class:selected={selectedRangeIndex === index}
            class:inactive={range.activated === false}
            style:left={`${clampPercent(range.start)}%`}
            style:width={`${Math.max(0.8, clampPercent(range.end) - clampPercent(range.start))}%`}
            title={`选区 ${index + 1}：${formatTime(range.start)} → ${formatTime(range.end)}`}
            onclick={(event) => selectRange(event, index, range.start)}
          >
            <span
              class="range-handle range-handle-start"
              role="slider"
              aria-label={`调整选区 ${index + 1} 起点`}
              aria-valuemin="0"
              aria-valuemax={range.end}
              aria-valuenow={range.start}
              tabindex="0"
              onpointerdown={(event) =>
                beginRangeDrag(event, index, "start")}
              onpointermove={updateRangeDrag}
              onpointerup={endRangeDrag}
              onpointercancel={endRangeDrag}
              onclick={(event) => event.stopPropagation()}
            ></span>
            <span class="range-label">选区 {index + 1}</span>
            <span
              class="range-handle range-handle-end"
              role="slider"
              aria-label={`调整选区 ${index + 1} 终点`}
              aria-valuemin={range.start}
              aria-valuemax={duration}
              aria-valuenow={range.end}
              tabindex="0"
              onpointerdown={(event) => beginRangeDrag(event, index, "end")}
              onpointermove={updateRangeDrag}
              onpointerup={endRangeDrag}
              onpointercancel={endRangeDrag}
              onclick={(event) => event.stopPropagation()}
            ></span>
          </button>
        {/each}

        {#each markers as marker}
          <button
            type="button"
            class="marker"
            style:left={`${clampPercent(marker.offset)}%`}
            title={`${marker.content || "标记"} · ${formatTime(marker.offset)}`}
            onclick={(event) => {
              event.stopPropagation();
              onSeek?.(marker.offset);
            }}
          ></button>
        {/each}
      </div>

      <span
        class="playhead"
        style:left={`${clampPercent(currentTime)}%`}
        aria-hidden="true"
      ></span>
    </div>

    <div class="timeline-footer">
      <span>{formatTime(currentTime)} / {formatTime(duration)}</span>
      <span>
        已启用 {activeRanges.length} 个选区 · 合成后约 {formatTime(activeDuration)}
      </span>
    </div>
  </div>
</section>

<style>
  .timeline-panel {
    display: flex;
    height: 196px;
    flex: 0 0 196px;
    flex-direction: column;
    gap: 8px;
    border-top: 1px solid #2b3340;
    background: #0e1219;
    padding: 12px 16px 14px;
    color: #e9eef8;
  }

  .timeline-header,
  .timeline-actions,
  .timeline-footer {
    display: flex;
    align-items: center;
  }

  .timeline-header {
    position: relative;
    min-height: 34px;
    justify-content: space-between;
    gap: 12px;
  }

  .timeline-footer {
    color: #7f8a9c;
    font-size: 10px;
  }

  .timeline-controls {
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 7px;
  }

  .control-icon {
    display: inline-flex;
    width: 30px;
    height: 30px;
    align-items: center;
    justify-content: center;
    border: 1px solid #303947;
    border-radius: 8px;
    background: #202733;
    color: #cbd7e8;
  }

  .playback-control {
    position: absolute;
    top: 0;
    left: 50%;
    width: 38px;
    height: 34px;
    transform: translateX(-50%);
  }

  .control-icon:hover,
  .control-icon.enabled {
    border-color: #168de0;
    background: #153b5c;
    color: #65c2ff;
  }

  .offset-control,
  .volume-control {
    display: inline-flex;
    height: 30px;
    align-items: center;
    gap: 5px;
    border: 1px solid #303947;
    border-radius: 8px;
    background: #202733;
    padding: 0 8px;
    color: #8490a3;
    font-size: 10px;
    white-space: nowrap;
  }

  .settings-control .offset-control {
    width: 126px;
    justify-content: space-between;
  }

  .offset-control input {
    width: 46px;
    border: 0;
    outline: 0;
    background: transparent;
    color: #dce5f3;
    font-size: 10px;
    text-align: right;
    appearance: textfield;
  }

  .offset-control input::-webkit-outer-spin-button,
  .offset-control input::-webkit-inner-spin-button {
    margin: 0;
    appearance: none;
  }

  .volume-control input {
    width: 66px;
    accent-color: #0a84ff;
  }

  .time-display {
    color: #b6c0cf;
    font-size: 10px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .danmu-sender {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .danmu-sender input {
    width: 0;
    height: 30px;
    border: 1px solid #3b4758;
    border-radius: 8px;
    outline: 0;
    background: #202733;
    color: #e9eef8;
    font-size: 10px;
    opacity: 0;
    transition:
      width 160ms ease,
      opacity 160ms ease;
  }

  .danmu-sender.expanded input {
    width: 150px;
    padding: 0 8px;
    opacity: 1;
  }

  .timeline-actions {
    position: relative;
    flex: 0 0 auto;
    gap: 8px;
  }

  .timeline-actions button {
    height: 30px;
  }

  .timeline-actions button {
    display: inline-flex;
    height: 30px;
    align-items: center;
    gap: 6px;
    border: 1px solid #303947;
    border-radius: 8px;
    background: #202733;
    padding: 0 10px;
    color: #dce5f3;
    font-size: 11px;
    transition:
      border-color 150ms ease,
      background-color 150ms ease;
  }

  .timeline-actions button:hover {
    border-color: #465469;
    background: #293240;
  }

  .settings-control {
    position: relative;
    display: flex;
    align-items: center;
  }

  .settings-control .offset-control {
    position: absolute;
    top: 38px;
    right: 0;
    z-index: 10;
    box-shadow: 0 8px 20px rgb(0 0 0 / 28%);
  }

  .timeline-surface {
    min-height: 0;
    flex: 1;
    border: 1px solid #29313d;
    border-radius: 10px;
    background: #151a23;
    padding: 8px 12px;
  }

  .time-grid {
    position: relative;
  }

  .heat-row {
    position: relative;
    display: flex;
    width: 100%;
    height: 30px;
    align-items: flex-end;
    cursor: pointer;
  }

  .heat-row:focus-visible {
    outline: 2px solid #0a84ff;
    outline-offset: 2px;
  }

  .heat-label {
    position: absolute;
    z-index: 2;
    top: 2px;
    left: 0;
    border-radius: 4px;
    background: rgb(21 26 35 / 86%);
    padding: 2px 5px;
    color: #7f8a9c;
    font-size: 10px;
    pointer-events: none;
  }

  .heat-bars {
    position: relative;
    width: 100%;
    height: 28px;
    min-width: 0;
  }

  .heat-bar {
    position: absolute;
    bottom: 0;
    min-width: 2px;
    transform: translateX(-50%);
    border-radius: 2px 2px 0 0;
    background: #3a4b63;
  }

  .heat-bar.hot {
    background: #35a7ff;
    box-shadow: 0 0 6px rgb(53 167 255 / 45%);
  }

  .heat-bar.extension {
    background: #737fd1;
    box-shadow: 0 0 4px rgb(115 127 209 / 30%);
  }

  .threshold-line {
    position: absolute;
    right: 0;
    left: 0;
    z-index: 3;
    height: 1px;
    border-top: 1px dashed #ffc857;
    filter: drop-shadow(0 0 2px rgb(255 200 87 / 45%));
    pointer-events: none;
  }

  .time-ruler {
    display: flex;
    width: 100%;
    height: 15px;
    align-items: flex-end;
    justify-content: space-between;
    color: #687589;
    font-size: 9px;
    font-variant-numeric: tabular-nums;
  }

  .range-track {
    position: relative;
    height: 42px;
    margin-top: 4px;
    border-radius: 7px;
    background:
      linear-gradient(90deg, transparent 24.8%, #202733 25%, transparent 25.2%),
      linear-gradient(90deg, transparent 49.8%, #202733 50%, transparent 50.2%),
      linear-gradient(90deg, transparent 74.8%, #202733 75%, transparent 75.2%),
      #0d1118;
    cursor: crosshair;
  }

  .range-track:focus-visible {
    outline: 2px solid #0a84ff;
    outline-offset: 2px;
  }

  .range-block {
    position: absolute;
    top: 5px;
    height: 32px;
    min-width: 6px;
    overflow: hidden;
    border: 1px solid #4db5ff;
    border-radius: 6px;
    background: #168de0cc;
    padding: 0 8px;
    color: white;
    text-align: left;
    white-space: nowrap;
  }

  .range-handle {
    position: absolute;
    top: 0;
    bottom: 0;
    z-index: 2;
    width: 10px;
    height: auto;
    border: 0;
    border-radius: 0;
    background: transparent;
    cursor: ew-resize;
    touch-action: none;
  }

  .range-handle-start {
    left: -5px;
  }

  .range-handle-end {
    right: -5px;
  }

  .range-handle:focus-visible {
    outline: 2px solid rgb(255 255 255 / 75%);
    outline-offset: 1px;
  }

  .range-label {
    display: block;
    overflow: hidden;
    padding: 0 4px;
    text-overflow: ellipsis;
  }

  .range-block:nth-of-type(even) {
    border-color: #7b86ff;
    background: #5966dfcc;
  }

  .range-block.selected {
    box-shadow:
      0 0 0 2px #0d1118,
      0 0 0 4px #9cd7ff;
  }

  .range-block.inactive {
    border-color: #596273;
    background: #3c4554;
    opacity: 0.62;
  }

  .range-block span {
    font-size: 10px;
  }

  .marker {
    position: absolute;
    top: -5px;
    z-index: 4;
    width: 9px;
    height: 9px;
    transform: translateX(-50%) rotate(45deg);
    border: 1px solid #fff1b6;
    background: #ffc857;
  }

  .playhead {
    position: absolute;
    top: 0;
    bottom: 0;
    z-index: 3;
    width: 1px;
    background: #ff6677;
    pointer-events: none;
  }

  .playhead::before {
    position: absolute;
    top: 41px;
    left: -4px;
    width: 9px;
    height: 7px;
    background: #ff6677;
    clip-path: polygon(0 0, 100% 0, 50% 100%);
    content: "";
  }

  .timeline-footer {
    justify-content: space-between;
    padding-top: 8px;
  }

  @media (max-height: 680px) {
    .timeline-panel {
      height: 164px;
      flex-basis: 164px;
    }

    .heat-row {
      display: none;
    }
  }
</style>
