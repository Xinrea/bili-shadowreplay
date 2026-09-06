<script lang="ts">

  import {
    invoke,
    set_title,
    log,
    get_static_url,
  } from "./lib/invoker";
  import Player from "./lib/components/Player.svelte";
  import type { AccountItem, RecordItem } from "./lib/db";
  import { PanelRightOpen } from "lucide-svelte";
  import {
    type VideoItem,
    type Marker,
    type DanmuEntry,
    type Range,
    type RecorderInfo,
  } from "./lib/interface";
  import ArchiveInspector from "./lib/components/ArchiveInspector.svelte";
  import ArchiveTimeline from "./lib/components/ArchiveTimeline.svelte";
  import { onDestroy, onMount } from "svelte";

  interface PlayerHandle {
    seek(offset: number): void;
    togglePlayback(): void;
    setVolume(volume: number): void;
    toggleDanmu(): void;
    setDanmuOffset(offset: number): void;
    sendDanmaku(message: string): Promise<void>;
    exportDanmu(ass: boolean): Promise<void>;
    seekLive(): void;
  }

  interface PreviewVideo {
    id: number;
    value: number;
    name: string;
    file: string;
    cover: string;
  }

  const urlParams = new URLSearchParams(window.location.search);
  const room_id = urlParams.get("room_id");
  const platform = urlParams.get("platform");
  const live_id = urlParams.get("live_id");

  log.info("AppLive loaded", room_id, platform, live_id);

  // 弹幕相关变量
  let danmu_records: DanmuEntry[] = $state([]);

  // 弹幕峰值检测相关变量
  interface DanmuPeak {
    start: number; // 秒
    end: number; // 秒
    count: number;
    exists: boolean; // 是否已存在于选区列表
  }
  interface DanmuHeatPoint {
    time: number;
    count: number;
    level: "normal" | "extension" | "core";
  }
  let danmu_peaks: DanmuPeak[] = $state([]);
  let danmu_heat_points: DanmuHeatPoint[] = $state([]);
  let danmu_heat_threshold = $state(0);
  let peak_threshold = $state(80); // 阈值百分比
  const DENSITY_WINDOW_SEC = 30; // 内部固定密度计算窗口
  const keywords_storage_key = `danmu_keywords:${room_id}:${live_id}`;
  let danmu_keywords: string[] = $state(
    (() => {
      try {
        const saved = JSON.parse(
          window.localStorage.getItem(keywords_storage_key) || "[]",
        );
        return Array.isArray(saved)
          ? saved.filter((item): item is string => typeof item === "string")
          : [];
      } catch {
        return [];
      }
    })(),
  );

  // 辅助函数：判断两个时间范围是否相似（容差 tolerance 秒）
  function is_range_similar(
    r1: { start: number; end: number },
    r2: { start: number; end: number },
    tolerance: number = 5,
  ): boolean {
    return (
      Math.abs(r1.start - r2.start) < tolerance &&
      Math.abs(r1.end - r2.end) < tolerance
    );
  }

  function filter_danmu_by_keywords(
    records: DanmuEntry[],
    keywords: string[],
  ): DanmuEntry[] {
    if (keywords.length === 0) return records;
    return records.filter((entry) =>
      keywords.some((keyword) =>
        entry.content.toLowerCase().includes(keyword.toLowerCase()),
      ),
    );
  }

  // 检测弹幕峰值区间
  function detect_danmu_peaks(threshold_percent = peak_threshold) {
    const records = filter_danmu_by_keywords(danmu_records, danmu_keywords);
    if (records.length === 0) {
      danmu_peaks = [];
      danmu_heat_points = [];
      danmu_heat_threshold = 0;
      return;
    }

    const window_ms = DENSITY_WINDOW_SEC * 1000;
    const step_ms = 5000; // 5秒滑动步长
    const bucket_ms = step_ms; // 桶大小与步长一致

    // 找出时间范围
    const min_ts = Math.min(...records.map((d) => d.ts));
    const max_ts = Math.max(...records.map((d) => d.ts));

    // 1. 构建弹幕直方图 (O(N))
    const total_buckets = Math.ceil((max_ts - min_ts) / bucket_ms) + 1;
    const histogram = new Array(total_buckets).fill(0);
    for (const d of records) {
      const bucket_idx = Math.floor((d.ts - min_ts) / bucket_ms);
      if (bucket_idx >= 0 && bucket_idx < total_buckets) {
        histogram[bucket_idx]++;
      }
    }

    // 2. 使用滑动窗口计算密度 (O(W))
    const density: { center: number; count: number }[] = [];
    const counts: number[] = [];
    const window_buckets = Math.ceil(window_ms / bucket_ms);

    // 初始化第一个窗口的和
    let window_sum = 0;
    for (let i = 0; i < window_buckets && i < total_buckets; i++) {
      window_sum += histogram[i];
    }

    // 滑动窗口遍历
    for (let i = 0; i + window_buckets <= total_buckets; i++) {
      const center = min_ts + (i + window_buckets / 2) * bucket_ms;
      density.push({ center, count: window_sum });
      counts.push(window_sum);

      // 滑动：移除左边，添加右边
      if (i + window_buckets < total_buckets) {
        window_sum -= histogram[i];
        window_sum += histogram[i + window_buckets];
      }
    }

    if (density.length === 0) {
      danmu_peaks = [];
      danmu_heat_points = [];
      danmu_heat_threshold = 0;
      return;
    }

    // 2. 计算统计特征 (Mean & StdDev)
    const n = counts.length;
    const mean = counts.reduce((a, b) => a + b, 0) / n;
    const variance = counts.reduce((a, b) => a + Math.pow(b - mean, 2), 0) / n;
    const stdDev = Math.sqrt(variance);

    // 3. 将滑块百分比直接映射到当前录播的最大窗口热度。
    // 不再叠加固定弹幕数下限，避免下限覆盖滑块导致阈值不变化。
    const max_density = counts.reduce(
      (maximum, count) => Math.max(maximum, count),
      0,
    );
    const effective_threshold =
      max_density * (Math.min(100, Math.max(0, threshold_percent)) / 100);
    const recording_offset_ms =
      global_offset > 0 ? global_offset * 1000 : min_ts;

    danmu_heat_threshold = effective_threshold;

    // 先将连续超过阈值的柱子聚合为核心区间。
    const core_runs: { start: number; end: number }[] = [];
    let run_start_index = -1;
    for (let i = 0; i <= density.length; i++) {
      const is_above_threshold =
        i < density.length &&
        density[i].count > 0 &&
        density[i].count >= effective_threshold;

      if (is_above_threshold && run_start_index === -1) {
        run_start_index = i;
        continue;
      }
      if (is_above_threshold || run_start_index === -1) {
        continue;
      }

      const run_end_index = i - 1;
      core_runs.push({ start: run_start_index, end: run_end_index });
      run_start_index = -1;
    }

    // 核心区间向两侧扩展至较低的内容完整性基准。相交的扩展区间
    // 合并为一条推荐，避免多个核心峰生成相同或高度重叠的结果。
    const expansion_baseline = Math.min(
      mean + 0.5 * stdDev,
      effective_threshold * 0.8,
    );
    const expanded_runs = core_runs.map((run) => {
      let start = run.start;
      let end = run.end;
      while (
        start > 0 &&
        density[start - 1].count > expansion_baseline
      ) {
        start -= 1;
      }
      while (
        end < density.length - 1 &&
        density[end + 1].count > expansion_baseline
      ) {
        end += 1;
      }
      return { start, end };
    });
    const merged_runs: { start: number; end: number }[] = [];
    for (const run of expanded_runs) {
      const previous = merged_runs[merged_runs.length - 1];
      if (previous && run.start <= previous.end + 1) {
        previous.end = Math.max(previous.end, run.end);
      } else {
        merged_runs.push({ ...run });
      }
    }

    const extension_flags = Array.from(
      { length: density.length },
      () => false,
    );
    for (const run of merged_runs) {
      for (let i = run.start; i <= run.end; i++) {
        extension_flags[i] = true;
      }
    }

    // 时间线与推荐算法共用同一份窗口数据，并区分核心与扩展部分。
    danmu_heat_points = density.map((point, index) => ({
      time: (point.center - recording_offset_ms) / 1000,
      count: point.count,
      level:
        point.count > 0 && point.count >= effective_threshold
          ? "core"
          : extension_flags[index]
            ? "extension"
            : "normal",
    }));

    const half_step_ms = step_ms / 2;
    const final_peaks = merged_runs.flatMap((run): DanmuPeak[] => {
      const final_start = Math.max(
        0,
        (density[run.start].center - half_step_ms - recording_offset_ms) /
          1000,
      );
      const final_end =
        (density[run.end].center + half_step_ms - recording_offset_ms) /
        1000;
      if (final_end <= final_start) return [];

      const peak_count = density
        .slice(run.start, run.end + 1)
        .reduce((maximum, point) => Math.max(maximum, point.count), 0);
      const exists = ranges.some((range) =>
        is_range_similar(range, { start: final_start, end: final_end }),
      );
      return [{
        start: final_start,
        end: final_end,
        count: peak_count,
        exists,
      }];
    });

    // 与时间线顺序保持一致，按推荐区间起始时间升序排列。
    danmu_peaks = final_peaks.sort((a, b) => a.start - b.start);
  }

  // 将峰值添加到选区
  function add_peak_to_ranges(peak: DanmuPeak) {
    // 检查是否已存在类似选区
    const exists = ranges.some((r) => is_range_similar(r, peak));
    if (exists) {
      return;
    }

    ranges = [...ranges, { start: peak.start, end: peak.end, activated: true }];
    peak.exists = true;
    danmu_peaks = [...danmu_peaks]; // 触发响应式更新
  }

  // 一键添加所有峰值
  function add_all_peaks_to_ranges() {
    for (const peak of danmu_peaks) {
      if (!peak.exists) {
        add_peak_to_ranges(peak);
      }
    }
  }

  // 更新峰值的添加状态（根据当前 ranges）
  function update_peak_exists_status() {
    let changed = false;
    for (const peak of danmu_peaks) {
      const exists = ranges.some((range) => is_range_similar(range, peak));
      if (peak.exists !== exists) {
        peak.exists = exists;
        changed = true;
      }
    }
    if (changed) {
      danmu_peaks = [...danmu_peaks]; // 触发响应式更新
    }
  }
  let archive: RecordItem = $state(null);

  // load ranges from local storage
  let ranges: Range[] = $state(JSON.parse(
    window.localStorage.getItem(`ranges:${room_id}:${live_id}`) || "[]"
  ));
  let global_offset = $state(0);

  let clip_running = $state(false);
  let videos: PreviewVideo[] = $state([]);

  let selected_video: PreviewVideo | null = $state(null);

  let video: HTMLVideoElement;
  let current_time = $state(0);
  let player_duration = $state(0);
  let selected_range_index = $state(-1);
  let player_volume = $state(1);
  let player_is_playing = $state(false);
  let player_is_live = $state(false);
  let danmu_enabled = $state(true);
  let danmu_offset = $state(0);
  let can_send_danmaku = $state(false);
  let danmu_accounts = $state<AccountItem[]>([]);
  let danmu_account_uid = $state("");
  let live_recorders = $state<RecorderInfo[]>([]);

  function pauseForRangeDrag() {
    video?.pause();
  }

  function seekDuringRangeDrag(seconds: number) {
    player?.seek(seconds);
    current_time = seconds;
  }

  // Initialize video element when component is mounted
  onMount(() => {
    video = document.getElementById("video") as HTMLVideoElement;
    invoke("get_archive", { roomId: room_id, liveId: live_id }).then(
      (a: RecordItem) => {
        archive = a;
        set_title(`[${room_id}]${archive.title}`);
      }
    );
  });

  function addRangeAtCurrentTime() {
    const total = player_duration || archive?.length || 0;
    if (total <= 0) return;
    const start = Math.min(current_time, total);
    ranges = [
      ...ranges,
      {
        start,
        end: total,
        activated: true,
      },
    ];
    selected_range_index = ranges.length - 1;
  }

  function addMarkerAtCurrentTime() {
    markers = [
      ...markers,
      {
        offset: current_time,
        realtime: global_offset + current_time,
        content: "[空标记点]",
      },
    ].sort((a, b) => a.offset - b.offset);
  }

  get_video_list();

  async function get_video_list() {
    const videoList = (await invoke("get_videos", {
      roomId: room_id,
    })) as VideoItem[];
    videos = await Promise.all(
      videoList.map(async (v) => {
        return {
          id: v.id,
          value: v.id,
          name: v.file,
          file: await get_static_url("output", v.file),
          cover: await get_static_url("output", v.cover),
        };
      })
    );
  }

  async function selectVideo(id: number) {
    selected_video = videos.find((video) => video.value === id) || null;
  }

  async function handleClipGenerated(newVideo: VideoItem) {
    await get_video_list();
    newVideo.cover = await get_static_url("output", newVideo.cover);
    selected_video =
      videos.find((video) => video.value === newVideo.id) || null;
    if (selected_video) {
      selected_video.cover = newVideo.cover;
    }
  }

  async function delete_video(id: number) {
    await invoke("delete_video", { id });
    if (selected_video?.id === id) {
      selected_video = null;
    }
    await get_video_list();
  }

  let player: PlayerHandle = $state();
  let rpanel_collapsed = $state(false);
  let markers: Marker[] = $state([]);
  // load markers from local storage
  markers = JSON.parse(
    window.localStorage.getItem(`markers:${room_id}:${live_id}`) || "[]"
  );

  async function save_video(id: number) {
    const video = videos.find((item) => item.id === id);
    if (!video) {
      return;
    }
    // download video
    const a = document.createElement("a");
    a.href = video.file;
    a.download = video.name;
    a.click();
  }

  async function open_clip(video_id: number) {
    await invoke("open_clip", { videoId: video_id });
  }

  function navigate_to_recorder(recorder: RecorderInfo) {
    const nextUrl =
      `${window.location.origin}${window.location.pathname}` +
      `?platform=${recorder.room_info.platform}` +
      `&room_id=${recorder.room_info.room_id}` +
      `&live_id=${recorder.live_id}`;
    window.location.href = nextUrl;
  }

  function handlePeakThresholdChange(value: number) {
    peak_threshold = value;
  }

  function handleDanmuKeywordsChange(keywords: string[]) {
    danmu_keywords = keywords;
  }

  // 弹幕数据、关键词或阈值变化时刷新智能推荐
  $effect(() => {
    global_offset;
    danmu_keywords;
    detect_danmu_peaks(peak_threshold);
  });
  // 监听 ranges 变化，更新峰值的添加状态
  $effect(() => {
    if (ranges && danmu_peaks.length > 0) {
      update_peak_exists_status();
    }
  });
  // save ranges to local storage when changed
  $effect(() => {
    if (ranges) {
      window.localStorage.setItem(
        `ranges:${room_id}:${live_id}`,
        JSON.stringify(ranges)
      );
    }
  });
  $effect(() => {
    // makers changed, save to local storage
    window.localStorage.setItem(
      `markers:${room_id}:${live_id}`,
      JSON.stringify(markers)
    );
  });
  $effect(() => {
    window.localStorage.setItem(
      keywords_storage_key,
      JSON.stringify(danmu_keywords),
    );
  });
</script>

<main>
  <div class="preview-workspace">
    <div class="preview-main">
      <div class="video-stage">
        <Player
          bind:ranges
          bind:global_offset
          bind:this={player}
          bind:danmu_records
          bind:danmu_enabled
          bind:local_offset={danmu_offset}
          bind:volume={player_volume}
          bind:is_playing={player_is_playing}
          bind:playback_time={current_time}
          bind:duration={player_duration}
          bind:is_live={player_is_live}
          bind:can_send_danmaku={can_send_danmaku}
          bind:danmu_accounts
          bind:danmu_account_uid
          bind:recorders={live_recorders}
          bind:selected_range_index={selected_range_index}
          {platform}
          {room_id}
          {live_id}
          {markers}
          onMarkerAdd={(marker) => {
            markers.push({
              offset: marker.offset,
              realtime: marker.realtime,
              content: "[空标记点]",
            });
            markers = markers.sort((a, b) => a.offset - b.offset);
          }}
        />
      </div>
      <ArchiveTimeline
        bind:ranges
        {markers}
        heatPoints={danmu_heat_points}
        heatThreshold={danmu_heat_threshold}
        heatThresholdPercent={peak_threshold}
        keywordFiltered={danmu_keywords.length > 0}
        currentTime={current_time}
        duration={player_is_live
          ? player_duration
          : player_duration || archive?.length || 0}
        isPlaying={player_is_playing}
        isLive={player_is_live}
        volume={player_volume}
        danmuEnabled={danmu_enabled}
        danmuOffset={danmu_offset}
        canSendDanmaku={can_send_danmaku}
        danmuAccounts={danmu_accounts}
        danmuAccountUid={danmu_account_uid}
        bind:selectedRangeIndex={selected_range_index}
        onSeek={(seconds) => player?.seek(seconds)}
        onAddRange={addRangeAtCurrentTime}
        onAddMarker={addMarkerAtCurrentTime}
        onTogglePlayback={() => player?.togglePlayback()}
        onSeekLive={() => player?.seekLive()}
        onVolumeChange={(value) => player?.setVolume(value)}
        onToggleDanmu={() => player?.toggleDanmu()}
        onDanmuOffsetChange={(value) => player?.setDanmuOffset(value)}
        onSendDanmaku={(message) => player?.sendDanmaku(message)}
        onDanmuAccountChange={(uid) => (danmu_account_uid = uid)}
        recorders={live_recorders}
        onExportDanmu={(ass) => player?.exportDanmu(ass)}
        onNavigateLive={navigate_to_recorder}
        onRangeDragStart={pauseForRangeDrag}
        onRangeDrag={seekDuringRangeDrag}
      />
    </div>
    <div
      class="inspector-shell"
      class:w-[340px]={!rpanel_collapsed}
      class:w-0={rpanel_collapsed}
    >
      {#if rpanel_collapsed}
        <button
          type="button"
          class="open-inspector"
          title="展开素材检查器"
          onclick={() => (rpanel_collapsed = false)}
        >
          <PanelRightOpen size={17} />
        </button>
      {:else}
        <ArchiveInspector
          {archive}
          bind:ranges
          bind:markers
          danmuRecords={danmu_records}
          globalOffset={global_offset}
          danmuPeaks={danmu_peaks}
          peakThreshold={peak_threshold}
          danmuKeywords={danmu_keywords}
          onPeakThresholdChange={handlePeakThresholdChange}
          onDanmuKeywordsChange={handleDanmuKeywordsChange}
          {videos}
          selectedVideo={selected_video}
          bind:clipRunning={clip_running}
          bind:selectedRangeIndex={selected_range_index}
          onCollapse={() => (rpanel_collapsed = true)}
          onSeek={(seconds) => player?.seek(seconds)}
          onAddPeak={add_peak_to_ranges}
          onAddAllPeaks={add_all_peaks_to_ranges}
          onVideoSelect={selectVideo}
          onDeleteVideo={delete_video}
          onDownloadVideo={save_video}
          onOpenVideo={open_clip}
          onGenerated={handleClipGenerated}
        />
      {/if}
    </div>
  </div>
</main>

<style>
  main {
    width: 100vw;
    height: 100vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: #0e1219;
  }

  .preview-workspace {
    position: relative;
    display: flex;
    min-height: 0;
    flex: 1;
    flex-direction: row;
    overflow: hidden;
  }

  .preview-main {
    display: flex;
    min-width: 0;
    min-height: 0;
    flex: 1;
    flex-direction: column;
    overflow: hidden;
  }

  .video-stage {
    position: relative;
    min-height: 0;
    flex: 1;
    overflow: hidden;
    background: #05070a;
  }

  .inspector-shell {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: 510;
    flex: 0 0 auto;
    overflow: visible;
    box-shadow: -12px 0 32px rgb(0 0 0 / 28%);
    transition: width 200ms ease;
  }

  .open-inspector {
    position: absolute;
    top: 12px;
    right: 8px;
    display: inline-flex;
    width: 34px;
    height: 34px;
    align-items: center;
    justify-content: center;
    border: 1px solid #344052;
    border-radius: 9px;
    background: #202733;
    color: #dce5f3;
    box-shadow: 0 6px 18px rgb(0 0 0 / 30%);
  }

  .open-inspector:focus-visible {
    outline: 2px solid #0a84ff;
    outline-offset: 2px;
  }

  @media (min-width: 1100px) {
    .inspector-shell {
      position: relative;
      box-shadow: none;
    }
  }
</style>
