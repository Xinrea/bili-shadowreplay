<script lang="ts">
  import {
    Diamond,
    MessageCircle,
    Pause,
    Play,
    Plus,
    Settings,
    Radio,
    Volume2,
    VolumeX,
  } from "lucide-svelte";
  import type { Marker, Range, RecorderInfo } from "../interface";

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
    isLive?: boolean;
    volume?: number;
    danmuEnabled?: boolean;
    danmuOffset?: number;
    canSendDanmaku?: boolean;
    danmuAccounts?: { uid: string; name: string }[];
    danmuAccountUid?: string;
    onTogglePlayback?: () => void;
    onSeekLive?: () => void;
    onVolumeChange?: (volume: number) => void;
    onToggleDanmu?: () => void;
    onDanmuOffsetChange?: (offset: number) => void;
    onSendDanmaku?: (message: string) => void;
    onDanmuAccountChange?: (uid: string) => void;
    recorders?: RecorderInfo[];
    onExportDanmu?: (ass: boolean) => void;
    onNavigateLive?: (recorder: RecorderInfo) => void;
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
    isLive = false,
    volume = 1,
    danmuEnabled = true,
    danmuOffset = 0,
    canSendDanmaku = false,
    danmuAccounts = [],
    danmuAccountUid = "",
    onTogglePlayback,
    onSeekLive,
    onVolumeChange,
    onToggleDanmu,
    onDanmuOffsetChange,
    onSendDanmaku,
    onDanmuAccountChange,
    recorders = [],
    onExportDanmu,
    onNavigateLive,
  }: Props = $props();
  let danmu_message = $state("");
  let show_offset_settings = $state(false);

  let visibleHeatPoints = $derived(
    heatPoints.filter(
      (point) => point.time >= timelineStart && point.time <= timelineEnd,
    ),
  );
  let maxHeat = $derived(
    visibleHeatPoints.reduce(
      (maximum, point) => Math.max(maximum, point.count),
      0,
    ),
  );
  let visibleRanges = $derived(
    ranges
      .map((range, index) => ({ range, index }))
      .filter(
        ({ range }) =>
          range.end >= timelineStart && range.start <= timelineEnd,
      ),
  );
  let visibleMarkers = $derived(
    markers.filter(
      (marker) =>
        marker.offset >= timelineStart && marker.offset <= timelineEnd,
    ),
  );
  let zoomed = $state(false);
  let zoom_start = $state(0);
  let zoom_end = $state(0);
  let timelineStart = $derived(zoomed ? zoom_start : 0);
  let timelineEnd = $derived(zoomed ? Math.min(duration, zoom_end) : duration);
  let timelineDuration = $derived(Math.max(0.001, timelineEnd - timelineStart));

  $effect(() => {
    if (!zoomed) {
      zoom_start = 0;
      zoom_end = duration;
    }
  });

  function clampPercent(seconds: number) {
    if (timelineDuration <= 0) return 0;
    return Math.min(
      100,
      Math.max(0, ((seconds - timelineStart) / timelineDuration) * 100),
    );
  }

  function heatHeight(count: number) {
    if (maxHeat <= 0) return 0;
    return Math.max(3, (count / maxHeat) * 26);
  }

  function heatBarGap(point: HeatPoint) {
    const pointIndex = heatPoints.indexOf(point);
    const next = heatPoints[pointIndex + 1];
    const previous = heatPoints[pointIndex - 1];
    return Math.abs((next || previous || point).time - point.time);
  }

  function heatBarStart(point: HeatPoint) {
    return clampPercent(point.time - heatBarGap(point) / 2);
  }

  function heatBarWidth(point: HeatPoint) {
    if (timelineDuration <= 0) return 0.15;
    const start = heatBarStart(point);
    const width = (heatBarGap(point) / timelineDuration) * 100;
    return Math.max(0.15, Math.min(width, 100 - start));
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
    if (timelineDuration <= 0) return;
    const target = event.currentTarget as HTMLElement;
    const rect = target.getBoundingClientRect();
    const ratio = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
    onSeek?.(timelineStart + ratio * timelineDuration);
  }

  function handleTimelineWheel(event: WheelEvent) {
    if (duration <= 0) return;
    event.preventDefault();
    const target = event.currentTarget as HTMLElement;
    const rect = target.getBoundingClientRect();
    const ratio = Math.min(
      1,
      Math.max(0, (event.clientX - rect.left) / rect.width),
    );
    const focusTime = timelineStart + ratio * timelineDuration;
    const factor = event.deltaY < 0 ? 0.8 : 1.25;
    const nextDuration = Math.min(
      duration,
      Math.max(30, timelineDuration * factor),
    );
    if (nextDuration >= duration * 0.99) {
      zoomed = false;
      return;
    }
    zoomed = true;
    zoom_start = Math.max(0, focusTime - ratio * nextDuration);
    zoom_end = Math.min(duration, zoom_start + nextDuration);
    zoom_start = Math.max(0, zoom_end - nextDuration);
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
    if (!drag_state || !range_track || timelineDuration <= 0) return;
    const rect = range_track.getBoundingClientRect();
    const ratio = Math.min(
      1,
      Math.max(0, (event.clientX - rect.left) / rect.width),
    );
    const seconds = timelineStart + ratio * timelineDuration;
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
          role="group"
          aria-label="发送弹幕"
        >
          <MessageCircle size={15} />
          <select
            value={danmuAccountUid}
            aria-label="发送弹幕账号"
            onchange={(event) =>
              onDanmuAccountChange?.(
                (event.currentTarget as HTMLSelectElement).value,
              )}
          >
            {#each danmuAccounts as account}
              <option value={account.uid}>{account.name}</option>
            {/each}
          </select>
          <input
            bind:value={danmu_message}
            placeholder="回车发送弹幕"
            onkeydown={(event) => {
              if (event.key === "Enter") submitDanmu();
              if (event.key === "Escape") danmu_message = "";
            }}
          />
        </div>
      {/if}
    </div>
    <div class="playback-cluster">
      <button
        type="button"
        class="control-icon playback-control"
        title={isPlaying ? "暂停" : "播放"}
        onclick={onTogglePlayback}
      >
        {#if isPlaying}<Pause size={15} />{:else}<Play size={15} />{/if}
      </button>
      <div class="volume-control">
        <button
          type="button"
          class="control-icon volume-button"
          aria-label={`音量 ${Math.round(volume * 100)}%`}
        >
          {#if volume === 0}
            <VolumeX size={15} />
          {:else}
            <Volume2 size={15} />
          {/if}
        </button>
        <div class="volume-popover">
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
          <output>{Math.round(volume * 100)}%</output>
        </div>
      </div>
      <div class="time-row">
        <span class="time-display">
          {formatTime(currentTime)} / {formatTime(duration)}
        </span>
        {#if isLive}
          <button
            type="button"
            class="live-button"
            class:at-live-edge={duration > 0 && duration - currentTime <= 3}
            title="跳转到直播位置"
            onclick={onSeekLive}
          >
            <span class="live-dot"></span>
            直播
          </button>
        {/if}
      </div>
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
          <div class="settings-menu">
            <label class="offset-control">
              <span>弹幕偏移</span>
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
            <div class="menu-divider"></div>
            <span class="menu-heading">弹幕导出</span>
            <button
              type="button"
              class="menu-item"
              onclick={() => onExportDanmu?.(false)}
            >
              导出弹幕为 TXT
            </button>
            <button
              type="button"
              class="menu-item"
              onclick={() => onExportDanmu?.(true)}
            >
              导出弹幕为 ASS
            </button>
            <div class="menu-divider"></div>
            <span class="menu-heading">快捷跳转</span>
            {#each recorders as recorder}
              <button
                type="button"
                class="menu-item live-room-item"
                onclick={() => onNavigateLive?.(recorder)}
              >
                <Radio size={12} />
                <span>
                  正在直播 · {recorder.user_info.user_name ||
                    recorder.room_info.room_title}
                </span>
              </button>
            {:else}
              <span class="menu-empty">没有其它正在直播的房间</span>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  </div>

  <div class="timeline-surface">
    <div class="time-grid" onwheel={handleTimelineWheel}>
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
          {#each visibleHeatPoints as point}
            <span
              class="heat-bar"
              class:extension={point.level === "extension"}
              class:hot={point.level === "core"}
              style:left={`${heatBarStart(point)}%`}
              style:width={`${heatBarWidth(point)}%`}
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
        <span>{formatTime(timelineStart)}</span>
        <span>{formatTime(timelineStart + timelineDuration * 0.25)}</span>
        <span>{formatTime(timelineStart + timelineDuration * 0.5)}</span>
        <span>{formatTime(timelineStart + timelineDuration * 0.75)}</span>
        <span>{formatTime(timelineEnd)}</span>
      </div>

      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        bind:this={range_track}
        class="range-track"
        role="slider"
        aria-label="录播时间线"
        aria-valuemin="0"
        aria-valuemax={timelineEnd}
        aria-valuenow={currentTime}
        tabindex="0"
        onclick={handleTrackClick}
        onkeydown={(event) => {
          if (event.key === "ArrowLeft") onSeek?.(Math.max(timelineStart, currentTime - 3));
          if (event.key === "ArrowRight")
            onSeek?.(Math.min(timelineEnd, currentTime + 3));
        }}
      >
        {#each visibleRanges as item}
          {@const range = item.range}
          {@const index = item.index}
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

        {#each visibleMarkers as marker}
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

    <div class="shortcut-hint" aria-label="快捷键说明">
      快捷键：Space 播放/暂停 · [ 创建前向 10 秒选区 · ] 创建后向 10 秒选区 ·
      Q/E 跳转选区起点/终点 · Backspace 删除选区 · P 添加标记 · ←/→ 前后跳转 3 秒
    </div>
  </div>
</section>

<style>
  .timeline-panel {
    display: flex;
    height: 176px;
    flex: 0 0 176px;
    flex-direction: column;
    gap: 8px;
    border-top: 1px solid #2b3340;
    background: #0e1219;
    padding: 6px 16px 8px;
    color: #e9eef8;
  }

  .timeline-header,
  .timeline-actions {
    display: flex;
    align-items: center;
  }

  .timeline-header {
    position: relative;
    display: grid;
    min-height: 38px;
    flex: 0 0 38px;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
    align-items: center;
    gap: 12px;
  }

  .timeline-controls {
    display: flex;
    grid-column: 2;
    min-width: 0;
    align-items: center;
    justify-content: center;
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
    width: 30px;
    height: 30px;
  }

  .playback-cluster {
    grid-column: 1;
    grid-row: 1;
    display: flex;
    width: auto;
    align-items: center;
    justify-self: start;
    gap: 6px;
  }

  .control-icon:hover,
  .control-icon.enabled {
    border-color: #168de0;
    background: #153b5c;
    color: #65c2ff;
  }

  .offset-control {
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

  .volume-control {
    position: relative;
    display: flex;
    align-items: center;
  }

  .volume-popover {
    position: absolute;
    bottom: 28px;
    left: 50%;
    z-index: 20;
    display: flex;
    width: 42px;
    height: 142px;
    align-items: center;
    justify-content: space-between;
    flex-direction: column;
    border: 1px solid #303947;
    border-radius: 10px;
    background: #202733;
    padding: 10px 5px 7px;
    opacity: 0;
    pointer-events: none;
    transform: translate(-50%, 6px);
    transition:
      opacity 150ms ease,
      transform 150ms ease;
    box-shadow: 0 8px 20px rgb(0 0 0 / 35%);
  }

  .volume-control:hover .volume-popover,
  .volume-control:focus-within .volume-popover {
    opacity: 1;
    pointer-events: auto;
    transform: translate(-50%, 0);
  }

  .volume-popover input {
    width: 96px;
    height: 18px;
    accent-color: #0a84ff;
    transform: translateY(42px) rotate(-90deg);
    transform-origin: center;
  }

  .volume-popover output {
    color: #dce5f3;
    font-size: 10px;
    font-variant-numeric: tabular-nums;
  }

  .volume-button {
    flex: 0 0 30px;
  }

  .time-display {
    color: #b6c0cf;
    font-size: 9px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .time-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .live-button {
    display: inline-flex;
    height: 18px;
    align-items: center;
    gap: 4px;
    border: 1px solid #435064;
    border-radius: 5px;
    background: #202733;
    padding: 0 5px;
    color: #aeb9c9;
    font-size: 9px;
    font-weight: 600;
  }

  .live-button:hover,
  .live-button.at-live-edge {
    border-color: #ef5668;
    background: #3b2028;
    color: #ff8591;
  }

  .live-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: currentColor;
  }

  .danmu-sender {
    display: flex;
    height: 30px;
    align-items: center;
    gap: 5px;
    border: 1px solid #3b4758;
    border-radius: 8px;
    background: #202733;
    padding: 0 8px;
    color: #8490a3;
  }

  .danmu-sender input {
    width: 150px;
    height: 28px;
    border: 0;
    outline: 0;
    background: transparent;
    color: #e9eef8;
    font-size: 10px;
  }

  .danmu-sender select {
    width: 88px;
    min-width: 0;
    border: 0;
    outline: 0;
    background: transparent;
    color: #aeb9c9;
    font-size: 10px;
    padding: 0;
    appearance: none;
  }

  .timeline-actions {
    grid-column: 3;
    grid-row: 1;
    position: relative;
    flex: 0 0 auto;
    gap: 8px;
    justify-self: end;
  }

  .timeline-actions > button {
    height: 30px;
  }

  .timeline-actions > button {
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

  .timeline-actions > button:hover {
    border-color: #465469;
    background: #293240;
  }

  .settings-control {
    position: relative;
    display: flex;
    align-items: center;
  }

  .settings-control > .control-icon {
    width: 30px;
    height: 30px;
    padding: 0;
  }

  .settings-control .offset-control {
    position: absolute;
    top: 38px;
    right: 0;
    z-index: 10;
    box-shadow: 0 8px 20px rgb(0 0 0 / 28%);
  }

  .settings-menu {
    position: absolute;
    bottom: 38px;
    right: 0;
    z-index: 20;
    display: flex;
    width: 220px;
    flex-direction: column;
    gap: 3px;
    border: 1px solid #303947;
    border-radius: 10px;
    background: #202733;
    padding: 8px;
    box-shadow: 0 10px 24px rgb(0 0 0 / 35%);
  }

  .settings-menu .offset-control {
    position: static;
    width: 100%;
    justify-content: space-between;
    box-shadow: none;
  }

  .menu-heading,
  .menu-empty {
    padding: 4px 6px 2px;
    color: #718095;
    font-size: 10px;
  }

  .menu-divider {
    height: 1px;
    margin: 5px 0;
    background: #303947;
  }

  .menu-item {
    display: flex;
    width: 100%;
    height: 28px;
    align-items: center;
    gap: 7px;
    border-radius: 7px;
    padding: 0 7px;
    color: #cbd7e8;
    font-size: 10px;
    text-align: left;
  }

  .menu-item:hover {
    background: #2c3848;
    color: #65c2ff;
  }

  .live-room-item span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .timeline-surface {
    min-height: 0;
    flex: 1;
    border: 1px solid #29313d;
    border-radius: 10px;
    background: #151a23;
    padding: 8px 12px;
  }

  .shortcut-hint {
    overflow-x: auto;
    padding-top: 6px;
    color: #687589;
    font-size: 9px;
    line-height: 14px;
    white-space: nowrap;
    scrollbar-width: none;
  }

  .shortcut-hint::-webkit-scrollbar {
    display: none;
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

  @media (max-height: 680px) {
    .timeline-panel {
      height: 150px;
      flex-basis: 150px;
    }

    .heat-row {
      display: none;
    }
  }
</style>
