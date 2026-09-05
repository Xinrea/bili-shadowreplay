<script lang="ts">

  import {
    invoke,
    set_title,
    log,
    get_static_url,
  } from "./lib/invoker";
  import Player from "./lib/components/Player.svelte";
  import type { RecordItem } from "./lib/db";
  import { PanelRightOpen } from "lucide-svelte";
  import {
    type VideoItem,
    type Marker,
    type DanmuEntry,
    type Range,
  } from "./lib/interface";
  import ArchivePreviewHeader from "./lib/components/ArchivePreviewHeader.svelte";
  import ArchiveInspector from "./lib/components/ArchiveInspector.svelte";
  import ArchiveTimeline from "./lib/components/ArchiveTimeline.svelte";
  import { onDestroy, onMount } from "svelte";

  interface PlayerHandle {
    seek(offset: number): void;
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
  const focus_start = parseInt(urlParams.get("start") || "0");
  const focus_end = parseInt(urlParams.get("end") || "0");

  log.info("AppLive loaded", room_id, platform, live_id);

  // 弹幕相关变量
  let danmu_records: DanmuEntry[] = $state([]);

  // 弹幕峰值检测相关变量
  interface DanmuPeak {
    start: number; // 秒
    end: number; // 秒
    count: number;
    added: boolean; // 是否已添加为选区
  }
  let danmu_peaks: DanmuPeak[] = $state([]);
  let peak_threshold = $state(80); // 阈值百分比
  const DENSITY_WINDOW_SEC = 30; // 内部固定密度计算窗口

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

  // 检测弹幕峰值区间
  function detect_danmu_peaks() {
    if (danmu_records.length === 0) {
      danmu_peaks = [];
      return;
    }

    const window_ms = DENSITY_WINDOW_SEC * 1000;
    const half_window_ms = window_ms / 2;
    const step_ms = 5000; // 5秒滑动步长
    const bucket_ms = step_ms; // 桶大小与步长一致

    // 找出时间范围
    const min_ts = Math.min(...danmu_records.map((d) => d.ts));
    const max_ts = Math.max(...danmu_records.map((d) => d.ts));

    // 1. 构建弹幕直方图 (O(N))
    const total_buckets = Math.ceil((max_ts - min_ts) / bucket_ms) + 1;
    const histogram = new Array(total_buckets).fill(0);
    for (const d of danmu_records) {
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
      return;
    }

    // 2. 计算统计特征 (Mean & StdDev)
    const n = counts.length;
    const mean = counts.reduce((a, b) => a + b, 0) / n;
    const variance = counts.reduce((a, b) => a + Math.pow(b - mean, 2), 0) / n;
    const stdDev = Math.sqrt(variance);

    // 3. 计算动态阈值
    // peak_threshold (50-100) 映射为 k (1.0 - 4.0)
    const k = 1.0 + ((peak_threshold - 50) / 50) * 3.0;
    const z_threshold = mean + k * stdDev;

    // 至少要有一定的弹幕量 (例如平均值的 1.5 倍，或者固定值如 15/30s)
    const abs_min_count = Math.max(15, mean * 1.2);

    // 动态边界的基准线 (Baseline)
    const expansion_baseline = mean + 0.5 * stdDev;

    // 4. 寻找候选峰值 (局部极值)
    let candidates: { center: number; count: number; index: number }[] = [];
    for (let i = 1; i < density.length - 1; i++) {
      const curr = density[i];
      const prev = density[i - 1];
      const next = density[i + 1];

      if (
        curr.count >= z_threshold &&
        curr.count >= abs_min_count && // 增加绝对门槛判断
        curr.count > 0 &&
        curr.count >= prev.count &&
        curr.count >= next.count
      ) {
        candidates.push({ ...curr, index: i });
      }
    }

    // 按强度降序排列
    candidates.sort((a, b) => b.count - a.count);

    const final_peaks: DanmuPeak[] = [];
    while (candidates.length > 0) {
      const best = candidates.shift();
      const best_idx = best.index;

      // 动态向左扩展
      let left_idx = best_idx;
      while (left_idx > 0 && density[left_idx].count > expansion_baseline) {
        left_idx--;
      }

      // 动态向右扩展
      let right_idx = best_idx;
      while (
        right_idx < density.length - 1 &&
        density[right_idx].count > expansion_baseline
      ) {
        right_idx++;
      }

      // 计算时间 (秒)
      const start_time = (density[left_idx].center - min_ts) / 1000 - 5; // 再多给5s缓冲
      const end_time = (density[right_idx].center - min_ts) / 1000 + 5;

      // 限制最小和最大时长
      const min_duration = 15;
      const max_duration = 120;
      let duration = end_time - start_time;

      let final_start = Math.max(0, start_time);
      let final_end = end_time;

      if (duration < min_duration) {
        const padding = (min_duration - duration) / 2;
        final_start = Math.max(0, final_start - padding);
        final_end = final_end + padding;
      } else if (duration > max_duration) {
        // 如果太长，就只取峰值附近的 max_duration
        const center_sec = (best.center - min_ts) / 1000;
        final_start = Math.max(0, center_sec - max_duration / 2);
        final_end = center_sec + max_duration / 2;
      }

      const is_added = ranges.some((r) =>
        is_range_similar(r, { start: final_start, end: final_end }),
      );

      final_peaks.push({
        start: final_start,
        end: final_end,
        count: best.count,
        added: is_added,
      });

      // 抑制相邻的较弱峰值 (基于实际生成的区间进行抑制)
      // 如果候选点落在我们刚刚生成的区间内，就剔除
      const current_peak_center_ms = best.center;
      candidates = candidates.filter(
        (c) =>
          Math.abs(c.center - current_peak_center_ms) >=
          ((final_end - final_start) * 1000) / 2, // 简单起见，只要距离峰值中心超过半个区间长度就算不重叠
      );
    }

    // 按弹幕数量降序排列
    danmu_peaks = final_peaks.sort((a, b) => b.count - a.count);
  }

  // 将峰值添加到选区
  function add_peak_to_ranges(peak: DanmuPeak) {
    // 检查是否已存在类似选区
    const exists = ranges.some((r) => is_range_similar(r, peak));
    if (exists) {
      return;
    }

    ranges = [...ranges, { start: peak.start, end: peak.end, activated: true }];
    peak.added = true;
    danmu_peaks = [...danmu_peaks]; // 触发响应式更新
  }

  // 一键添加所有峰值
  function add_all_peaks_to_ranges() {
    for (const peak of danmu_peaks) {
      if (!peak.added) {
        add_peak_to_ranges(peak);
      }
    }
  }

  // 更新峰值的添加状态（根据当前 ranges）
  function update_peak_added_status() {
    let changed = false;
    for (const peak of danmu_peaks) {
      const is_added = ranges.some((r) => is_range_similar(r, peak));
      if (peak.added !== is_added) {
        peak.added = is_added;
        changed = true;
      }
    }
    if (changed) {
      danmu_peaks = [...danmu_peaks]; // 触发响应式更新
    }
  }
  onDestroy(() => {
    if (video) {
      video.removeEventListener("timeupdate", syncPlaybackState);
      video.removeEventListener("durationchange", syncPlaybackState);
    }
  });

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

  function syncPlaybackState() {
    if (!video) return;
    current_time = Number.isFinite(video.currentTime) ? video.currentTime : 0;
    player_duration = Number.isFinite(video.duration) ? video.duration : 0;
  }

  // Initialize video element when component is mounted
  onMount(() => {
    video = document.getElementById("video") as HTMLVideoElement;
    video?.addEventListener("timeupdate", syncPlaybackState);
    video?.addEventListener("durationchange", syncPlaybackState);
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

  async function delete_video() {
    if (!selected_video) {
      return;
    }
    await invoke("delete_video", { id: selected_video.id });
    selected_video = null;
    await get_video_list();
  }
  let player: PlayerHandle = $state();
  let rpanel_collapsed = $state(false);
  let markers: Marker[] = $state([]);
  // load markers from local storage
  markers = JSON.parse(
    window.localStorage.getItem(`markers:${room_id}:${live_id}`) || "[]"
  );

  async function save_video() {
    if (!selected_video) {
      return;
    }
    // download video
    const video_url = selected_video.file;
    const video_name = selected_video.name;
    const a = document.createElement("a");
    a.href = video_url;
    a.download = video_name;
    a.click();
  }

  async function open_clip(video_id: number) {
    await invoke("open_clip", { videoId: video_id });
  }
  // 弹幕数据或阈值变化时刷新智能推荐
  $effect(() => {
    if (danmu_records.length > 0) {
      // 引用 peak_threshold 以便在其变化时触发重新计算
      peak_threshold;
      detect_danmu_peaks();
    }
  });
  // 监听 ranges 变化，更新峰值的添加状态
  $effect(() => {
    if (ranges && danmu_peaks.length > 0) {
      update_peak_added_status();
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
</script>

<main>
  <ArchivePreviewHeader {archive} {platform} roomId={room_id} />
  <div class="preview-workspace">
    <div class="preview-main">
      <div class="video-stage">
        <Player
          bind:ranges
          bind:global_offset
          bind:this={player}
          bind:danmu_records
          {focus_start}
          {focus_end}
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
        {ranges}
        {markers}
        danmuRecords={danmu_records}
        currentTime={current_time}
        duration={player_duration || archive?.length || 0}
        globalOffset={global_offset}
        bind:selectedRangeIndex={selected_range_index}
        onSeek={(seconds) => player?.seek(seconds)}
        onAddRange={addRangeAtCurrentTime}
        onAddMarker={addMarkerAtCurrentTime}
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
          bind:peakThreshold={peak_threshold}
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
