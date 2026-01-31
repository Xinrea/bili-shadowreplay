<script lang="ts">
  import {
    invoke,
    set_title,
    TAURI_ENV,
    listen,
    log,
    get_static_url,
  } from "./lib/invoker";
  import Player from "./lib/components/Player.svelte";
  import type { RecordItem } from "./lib/db";
  import { ChevronRight, ChevronLeft, Play, Pen } from "lucide-svelte";
  import {
    type VideoItem,
    type Config,
    type Marker,
    type DanmuEntry,
    clipRange,
    generateEventId,
    type Range,
  } from "./lib/interface";
  import MarkerPanel from "./lib/components/MarkerPanel.svelte";
  import { onDestroy, onMount } from "svelte";

  const urlParams = new URLSearchParams(window.location.search);
  const room_id = urlParams.get("room_id");
  const platform = urlParams.get("platform");
  const live_id = urlParams.get("live_id");
  const focus_start = parseInt(urlParams.get("start") || "0");
  const focus_end = parseInt(urlParams.get("end") || "0");

  log.info("AppLive loaded", room_id, platform, live_id);

  let config: Config = null;

  invoke("get_config").then((c) => {
    config = c as Config;
  });

  let current_clip_event_id = null;
  let danmu_enabled = false;
  let fix_encoding = false;
  let clip_note: string = "";

  // 弹幕相关变量
  let danmu_records: DanmuEntry[] = [];
  let filtered_danmu: DanmuEntry[] = [];
  let danmu_search_text = "";

  // 弹幕峰值检测相关变量
  interface DanmuPeak {
    start: number; // 秒
    end: number; // 秒
    count: number;
    added: boolean; // 是否已添加为选区
  }
  let danmu_peaks: DanmuPeak[] = [];
  let peak_threshold = 80; // 阈值百分比
  const DENSITY_WINDOW_SEC = 30; // 内部固定密度计算窗口
  let show_peak_panel = false;

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

  // 监听弹幕数据变化、面板状态变化、阈值变化，统一检测峰值
  $: if (show_peak_panel && danmu_records.length > 0) {
    // 引用 peak_threshold 以便在其变化时触发重新计算
    peak_threshold;
    detect_danmu_peaks();
  }

  // 监听 ranges 变化，更新峰值的添加状态
  $: if (ranges && danmu_peaks.length > 0) {
    update_peak_added_status();
  }

  // 虚拟滚动相关变量
  let danmu_container_height = 0;
  let danmu_item_height = 80; // 预估每个弹幕项的高度
  let visible_start_index = 0;
  let visible_end_index = 0;
  let scroll_top = 0;
  let container_ref: HTMLElement;
  let scroll_timeout: ReturnType<typeof setTimeout>;

  // 计算可见区域的弹幕
  function calculate_visible_danmu() {
    if (!container_ref || filtered_danmu.length === 0) return;

    const container_height = container_ref.clientHeight;
    const buffer = 10; // 缓冲区，多渲染几个项目

    visible_start_index = Math.max(
      0,
      Math.floor(scroll_top / danmu_item_height) - buffer
    );
    visible_end_index = Math.min(
      filtered_danmu.length,
      Math.ceil((scroll_top + container_height) / danmu_item_height) + buffer
    );
  }

  // 处理滚动事件（带防抖）
  function handle_scroll(event: Event) {
    const target = event.target as HTMLElement;
    scroll_top = target.scrollTop;

    // 清除之前的定时器
    if (scroll_timeout) {
      clearTimeout(scroll_timeout);
    }

    // 防抖处理，避免频繁计算
    scroll_timeout = setTimeout(() => {
      calculate_visible_danmu();
    }, 16); // 约60fps
  }

  // 监听容器大小变化
  function handle_resize() {
    if (container_ref) {
      danmu_container_height = container_ref.clientHeight;
      calculate_visible_danmu();
    }
  }

  // 监听弹幕数据变化，更新过滤结果
  $: {
    if (danmu_records) {
      // 如果当前有搜索文本，重新过滤
      if (danmu_search_text) {
        filter_danmu();
      } else {
        // 否则直接复制所有弹幕
        filtered_danmu = [...danmu_records];
      }
      // 重新计算可见区域
      calculate_visible_danmu();
    }
  }

  // 监听容器引用变化
  $: if (container_ref) {
    handle_resize();
  }

  // 过滤弹幕
  function filter_danmu() {
    filtered_danmu = danmu_records.filter((danmu) => {
      // 只按内容过滤
      if (
        danmu_search_text &&
        !danmu.content.toLowerCase().includes(danmu_search_text.toLowerCase())
      ) {
        return false;
      }
      return true;
    });
  }



  // 格式化时间(ts 为毫秒)
  function format_time(milliseconds: number): string {
    const seconds = Math.floor(milliseconds / 1000);
    const minutes = Math.floor(seconds / 60);
    const hours = Math.floor(minutes / 60)
      .toString()
      .padStart(2, "0");
    const remaining_seconds = (seconds % 60).toString().padStart(2, "0");
    const remaining_minutes = (minutes % 60).toString().padStart(2, "0");
    return `${hours}:${remaining_minutes}:${remaining_seconds}`;
  }

  // 将时长(单位: 秒)格式化为 "X小时 Y分 Z秒"
  function format_duration_seconds(totalSecondsFloat: number): string {
    const totalSeconds = Math.max(0, Math.floor(totalSecondsFloat));
    const hours = Math.floor(totalSeconds / 3600);
    const minutes = Math.floor((totalSeconds % 3600) / 60);
    const seconds = totalSeconds % 60;
    const parts = [] as string[];
    if (hours > 0) parts.push(`${hours} 小时`);
    if (minutes > 0) parts.push(`${minutes} 分`);
    parts.push(`${seconds} 秒`);
    return parts.join(" ");
  }

  // 跳转到弹幕时间点
  function seek_to_danmu(danmu: DanmuEntry) {
    if (player) {
      const time_in_seconds = danmu.ts / 1000 - global_offset;
      player.seek(time_in_seconds);
    }
  }

  onDestroy(() => {
    // 清理滚动定时器
    if (scroll_timeout) {
      clearTimeout(scroll_timeout);
    }
  });

  let archive: RecordItem = null;

  // load ranges from local storage
  let ranges: Range[] = JSON.parse(
    window.localStorage.getItem(`ranges:${room_id}:${live_id}`) || "[]"
  );
  $: activeRanges = ranges.filter((r) => r.activated !== false);
  // save ranges to local storage when changed
  $: {
    if (ranges) {
      window.localStorage.setItem(
        `ranges:${room_id}:${live_id}`,
        JSON.stringify(ranges)
      );
    }
  }
  let global_offset = 0;

  function handleSelectAll(e: Event) {
    const checked = (e.currentTarget as HTMLInputElement).checked;
    ranges = ranges.map((r) => ({ ...r, activated: checked }));
  }

  function handleRangeChange(e: Event, range: Range) {
    range.activated = (e.currentTarget as HTMLInputElement).checked;
    ranges = ranges; // trigger update
  }

  function deleteActivatedRanges() {
    // 删除选区
    ranges = ranges.filter((r) => r.activated === false);
  }

  function generateCover() {
    const video = document.getElementById("video") as HTMLVideoElement;
    var w = video.videoWidth;
    var h = video.videoHeight;
    var canvas = document.createElement("canvas");
    canvas.width = 1280;
    canvas.height = 720;
    var context = canvas.getContext("2d");
    context.drawImage(video, 0, 0, w, h, 0, 0, 1280, 720);
    return canvas.toDataURL();
  }

  let show_clip_confirm = false;
  let show_selection_list = false;
  let text_style = {
    position: { x: 8, y: 8 },
    fontSize: 24,
    color: "#FF7F00",
  };
  let video_selected = 0;
  let videos = [];

  let selected_video = null;

  let video: HTMLVideoElement;

  function pauseVideo() {
    if (video) {
      video.pause();
    }
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
    console.log(archive);

    // 初始化虚拟滚动
    setTimeout(() => {
      if (container_ref) {
        handle_resize();
      }
    }, 100);
  });

  get_video_list();

  function update_clip_prompt(str: string) {
    // update button text
    const span = document.getElementById("generate-clip-prompt");
    if (span) {
      span.textContent = str;
    }
  }

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
          cover: v.cover,
        };
      })
    );
  }

  async function find_video(e) {
    if (!e.target) {
      selected_video = null;
      return;
    }
    const id = parseInt(e.target.value);
    let target_video = videos.find((v) => {
      return v.value == id;
    });
    if (target_video) {
      target_video.cover = await get_static_url("output", target_video.cover);
    }
    selected_video = target_video;
  }

  async function confirm_generate_clip() {
    show_clip_confirm = false;
    let new_cover = generateCover();
    update_clip_prompt(`切片生成中`);
    let event_id = generateEventId();
    current_clip_event_id = event_id;
    const clear_update_listener = await listen(
      `progress-update:${event_id}`,
      (e) => {
        update_clip_prompt(e.payload.content);
      }
    );
    const clear_finished_listener = await listen(
      `progress-finished:${event_id}`,
      (e) => {
        update_clip_prompt(`生成切片`);
        if (!e.payload.success) {
          alert("请检查 ffmpeg 是否配置正确：" + e.payload.message);
        }
        current_clip_event_id = null;

        clear_update_listener();
        clear_finished_listener();
      }
    );
    let new_video = (await clipRange(event_id, {
      title: archive.title,
      note: clip_note,
      room_id: room_id,
      platform: platform,
      cover: new_cover,
      live_id: live_id,
      ranges: activeRanges,
      danmu: danmu_enabled,
      local_offset:
        parseInt(localStorage.getItem(`local_offset:${live_id}`) || "0", 10) ||
        0,
      fix_encoding,
    })) as VideoItem;
    await get_video_list();
    new_video.cover = await get_static_url("output", new_video.cover);
    video_selected = new_video.id;
    selected_video = videos.find((v) => {
      return v.value == new_video.id;
    });
    if (selected_video) {
      selected_video.cover = new_video.cover;
    }

    // clean up previous input data
    clip_note = "";
  }

  async function cancel_clip() {
    if (!current_clip_event_id) {
      return;
    }
    invoke("cancel", { eventId: current_clip_event_id });
  }

  async function delete_video() {
    if (!selected_video) {
      return;
    }
    await invoke("delete_video", { id: video_selected });
    video_selected = 0;
    selected_video = null;
    await get_video_list();
  }
  let player;
  let lpanel_collapsed = true;
  let rpanel_collapsed = false;
  let markers: Marker[] = [];
  // load markers from local storage
  markers = JSON.parse(
    window.localStorage.getItem(`markers:${room_id}:${live_id}`) || "[]"
  );
  $: {
    // makers changed, save to local storage
    window.localStorage.setItem(
      `markers:${room_id}:${live_id}`,
      JSON.stringify(markers)
    );
  }

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
</script>

<main>
  <div class="flex flex-row overflow-hidden">
    <div
      class="flex relative h-screen border-solid bg-gray-950 border-r-2 border-gray-800 z-[501] transition-all duration-300 ease-in-out"
      class:w-[200px]={!lpanel_collapsed}
      class:w-0={lpanel_collapsed}
    >
      <div class="relative flex w-full overflow-hidden">
        <div
          class="w-[200px] transition-all duration-300 overflow-hidden flex-shrink-0"
          style="margin-left: {lpanel_collapsed ? '-200px' : '0'};"
        >
          <div class="w-full whitespace-nowrap">
            <MarkerPanel
              {archive}
              bind:markers
              on:markerClick={(e) => {
                player.seek(e.detail.offset);
              }}
            />
          </div>
        </div>
      </div>
      <button
        class="collapse-btn lp transition-transform duration-300 absolute"
        on:click={() => {
          lpanel_collapsed = !lpanel_collapsed;
        }}
      >
        {#if lpanel_collapsed}
          <ChevronRight class="text-white" size={20} />
        {:else}
          <ChevronLeft class="text-white" size={20} />
        {/if}
      </button>
    </div>
    <div class="overflow-hidden h-screen w-full relative">
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
        on:markerAdd={(e) => {
          markers.push({
            offset: e.detail.offset,
            realtime: e.detail.realtime,
            content: "[空标记点]",
          });
          markers = markers.sort((a, b) => a.offset - b.offset);
        }}
      />
    </div>
    <div
      class="flex relative h-screen border-solid bg-gray-950 border-l-2 border-gray-800 text-white transition-all duration-300 ease-in-out"
      class:w-[400px]={!rpanel_collapsed}
      class:w-0={rpanel_collapsed}
    >
      <button
        class="collapse-btn rp transition-transform duration-300"
        class:translate-x-[-20px]={!rpanel_collapsed}
        class:translate-x-0={rpanel_collapsed}
        on:click={() => {
          rpanel_collapsed = !rpanel_collapsed;
        }}
      >
        {#if rpanel_collapsed}
          <ChevronLeft class="text-white" size={20} />
        {:else}
          <ChevronRight class="text-white" size={20} />
        {/if}
      </button>
      <div
        id="post-panel"
        class="h-screen bg-[#1c1c1e] text-white w-[400px] flex flex-col transition-opacity duration-300"
        class:opacity-0={rpanel_collapsed}
        class:opacity-100={!rpanel_collapsed}
        class:invisible={rpanel_collapsed}
      >
        <!-- 内容区域 -->
        <div class="flex-1 overflow-hidden flex flex-col">
          <div class="px-6 py-4 space-y-8 flex flex-col h-full">
            <!-- 切片操作区 -->
            <section class="space-y-3 flex-shrink-0">
              <div class="flex items-center justify-between">
                <h3 class="text-sm font-medium text-gray-300">切片列表</h3>
                <div class="flex space-x-2">
                  <button
                    on:click={() => (show_selection_list = true)}
                    class="px-4 py-1.5 bg-[#2c2c2e] text-white text-sm rounded-lg
                           hover:bg-[#3c3c3e]/90 transition-all duration-200
                           disabled:opacity-50 disabled:cursor-not-allowed
                           flex items-center space-x-2"
                  >
                    选区列表
                  </button>
                  <button
                    on:click={() => (show_clip_confirm = true)}
                    disabled={current_clip_event_id != null}
                    class="px-4 py-1.5 bg-[#0A84FF] text-white text-sm rounded-lg
                           transition-all duration-200 hover:bg-[#0A84FF]/90
                           disabled:opacity-50 disabled:cursor-not-allowed
                           flex items-center space-x-2"
                  >
                    {#if current_clip_event_id != null}
                      <div
                        class="w-4 h-4 border-2 border-current border-t-transparent rounded-full animate-spin"
                      />
                    {/if}
                    <span id="generate-clip-prompt">生成切片</span>
                  </button>
                  {#if current_clip_event_id != null}
                    <button
                      on:click={cancel_clip}
                      class="px-4 py-1.5 text-red-500 text-sm rounded-lg
                             transition-all duration-200 hover:bg-red-500/10
                             disabled:opacity-50 disabled:cursor-not-allowed"
                    >
                      取消
                    </button>
                  {/if}
                  {#if selected_video}
                    <button
                      on:click={delete_video}
                      class="px-4 py-1.5 text-red-500 text-sm rounded-lg
                             transition-all duration-200 hover:bg-red-500/10
                             disabled:opacity-50 disabled:cursor-not-allowed"
                    >
                      删除
                    </button>
                  {/if}
                </div>
              </div>

              <div class="flex flex-row items-center justify-between">
                <select
                  bind:value={video_selected}
                  on:change={find_video}
                  class="w-full px-3 py-2 bg-[#2c2c2e] text-white rounded-lg
                       border border-gray-800/50 focus:border-[#0A84FF]
                       transition duration-200 outline-none appearance-none
                       hover:border-gray-700/50"
                >
                  <option value={0}>选择切片</option>
                  {#each videos as video}
                    <option value={video.value}>{video.name}</option>
                  {/each}
                </select>
                {#if !TAURI_ENV && selected_video}
                  <button
                    on:click={save_video}
                    class="w-24 ml-2 px-3 py-2 bg-[#0A84FF] text-white rounded-lg
                     transition-all duration-200 hover:bg-[#0A84FF]/90
                     disabled:opacity-50 disabled:cursor-not-allowed"
                  >
                    下载
                  </button>
                {/if}
              </div>
            </section>

            <!-- 弹幕峰值检索区 -->
            <section class="space-y-3 flex-shrink-0">
              <div class="flex items-center justify-between">
                <h3 class="text-sm font-medium text-gray-300">弹幕峰值</h3>
                {#if show_peak_panel}
                  <button
                    on:click={() => (show_peak_panel = false)}
                    class="text-sm text-gray-400 hover:text-[#0A84FF] transition-colors duration-200"
                  >
                    收起
                  </button>
                {:else}
                  <button
                    on:click={() => (show_peak_panel = true)}
                    class="px-4 py-1.5 bg-[#2c2c2e] text-white text-sm rounded-lg
                           transition-all duration-200 hover:bg-[#3c3c3e]"
                  >
                    峰值检索
                  </button>
                {/if}
              </div>

              {#if show_peak_panel}
                <!-- 设置区域 -->
                <div
                  class="space-y-2 p-3 bg-[#2c2c2e] rounded-lg border border-gray-800/50"
                >
                  <div class="flex items-center justify-between">
                    <span class="text-xs text-gray-400"
                      >阈值: {peak_threshold}%</span
                    >
                    <input
                      type="range"
                      min="50"
                      max="100"
                      bind:value={peak_threshold}
                      class="w-32 h-1 bg-gray-700 rounded-lg appearance-none cursor-pointer"
                    />
                  </div>
                </div>

                <!-- 峰值列表 -->
                {#if danmu_records.length === 0}
                  <div class="text-center py-4 text-gray-500 text-sm">
                    暂无弹幕数据
                  </div>
                {:else if danmu_peaks.length === 0}
                  <div class="text-center py-4 text-gray-500 text-sm">
                    未检测到峰值，请尝试降低阈值
                  </div>
                {:else}
                  <div class="flex items-center justify-between mb-2">
                    <span class="text-xs text-gray-400">
                      检测到 {danmu_peaks.length} 个峰值
                    </span>
                    <button
                      on:click={add_all_peaks_to_ranges}
                      class="text-xs text-gray-400 hover:text-[#0A84FF] transition-colors duration-200 font-medium"
                    >
                      + 全部添加
                    </button>
                  </div>
                  <div
                    class="max-h-48 overflow-y-auto space-y-2 sidebar-scrollbar"
                  >
                    {#each danmu_peaks as peak}
                      <!-- svelte-ignore a11y-click-events-have-key-events -->
                      <div
                        class="flex items-center justify-between p-2 bg-[#2c2c2e] rounded-lg border border-gray-800/50
                               hover:border-[#0A84FF]/50 transition-all duration-200 cursor-pointer"
                        on:click={() => {
                          if (player) {
                            player.seek(peak.start);
                          }
                        }}
                      >
                        <div class="flex-1">
                          <div class="text-xs text-white/90">
                            {format_time(peak.start * 1000)} → {format_time(
                              peak.end * 1000,
                            )}
                          </div>
                          <div class="text-xs text-gray-500">
                            {peak.count} 条弹幕
                          </div>
                        </div>
                        {#if peak.added}
                          <span class="text-xs text-[#0A84FF]/80 font-medium">
                            ✓ 已添加
                          </span>
                        {:else}
                          <button
                            on:click|stopPropagation={() =>
                              add_peak_to_ranges(peak)}
                            class="text-xs text-gray-400 hover:text-[#0A84FF] transition-colors duration-200 font-medium"
                          >
                            + 添加
                          </button>
                        {/if}
                      </div>
                    {/each}
                  </div>
                {/if}
              {/if}
            </section>

            <!-- 封面预览 -->
            {#if selected_video && selected_video.id != -1}
              <section class="flex-shrink-0">
                <div class="group">
                  <!-- svelte-ignore a11y-click-events-have-key-events -->
                  <div
                    id="capture"
                    class="relative rounded-xl overflow-hidden bg-black/20 border border-gray-800/50 cursor-pointer group"
                    on:click={async () => {
                      pauseVideo();
                      await open_clip(selected_video.id);
                    }}
                  >
                    <div
                      class="absolute inset-0 bg-black/40 opacity-0 group-hover:opacity-100
                              transition duration-200 flex items-center justify-center backdrop-blur-[2px]"
                    >
                      <div
                        class="bg-white/10 backdrop-blur p-3 rounded-full opacity-0 group-hover:opacity-50"
                      >
                        <Play class="w-6 h-6 text-white" />
                      </div>
                    </div>
                    <img
                      src={selected_video.cover}
                      alt="视频封面"
                      class="w-full"
                    />
                  </div>
                </div>
              </section>
            {/if}

            <!-- 弹幕列表区 -->
            <section class="space-y-3 flex flex-col flex-1 min-h-0">
              <div class="flex items-center justify-between flex-shrink-0">
                <h3 class="text-sm font-medium text-gray-300">弹幕列表</h3>
              </div>

              <div class="space-y-3 flex flex-col flex-1 min-h-0">
                <!-- 搜索 -->
                <div class="space-y-2 flex-shrink-0">
                  <input
                    type="text"
                    bind:value={danmu_search_text}
                    placeholder="搜索弹幕内容..."
                    class="w-full px-3 py-2 bg-[#2c2c2e] text-white rounded-lg
                           border border-gray-800/50 focus:border-[#0A84FF]
                           transition duration-200 outline-none
                           placeholder-gray-500"
                  />
                </div>

                <!-- 弹幕统计 -->
                <div class="text-xs text-gray-400 flex-shrink-0">
                  共 {danmu_records.length} 条弹幕，显示 {filtered_danmu.length}
                  条
                </div>

                <!-- 弹幕列表 -->
                <div
                  bind:this={container_ref}
                  on:scroll={handle_scroll}
                  class="flex-1 overflow-y-auto space-y-2 sidebar-scrollbar min-h-0 danmu-container"
                >
                  <!-- 顶部占位符 -->
                  <div
                    style="height: {visible_start_index * danmu_item_height}px;"
                  />

                  <!-- 可见的弹幕项 -->
                  {#each filtered_danmu.slice(visible_start_index, visible_end_index) as danmu, index (visible_start_index + index)}
                    <!-- svelte-ignore a11y-click-events-have-key-events -->
                    <div
                      class="p-3 bg-[#2c2c2e] rounded-lg border border-gray-800/50
                             hover:border-[#0A84FF]/50 transition-all duration-200
                             cursor-pointer group danmu-item"
                      style="content-visibility: auto; contain-intrinsic-size: {danmu_item_height}px;"
                      on:click={() => seek_to_danmu(danmu)}
                    >
                      <div class="flex items-start justify-between">
                        <div class="flex-1 min-w-0">
                          <p
                            class="text-sm text-white break-words leading-relaxed"
                          >
                            {danmu.content}
                          </p>
                        </div>
                        <div class="ml-3 flex-shrink-0">
                          <span
                            class="text-xs text-gray-400 bg-[#1c1c1e] px-2 py-1 rounded
                                     group-hover:text-[#0A84FF] transition-colors duration-200"
                          >
                            {format_time(danmu.ts - global_offset * 1000)}
                          </span>
                        </div>
                      </div>
                    </div>
                  {/each}

                  <!-- 底部占位符 -->
                  <div
                    style="height: {(filtered_danmu.length -
                      visible_end_index) *
                      danmu_item_height}px;"
                  />

                  {#if filtered_danmu.length === 0}
                    <div class="text-center py-8 text-gray-500">
                      {danmu_records.length === 0
                        ? "暂无弹幕数据"
                        : "没有匹配的弹幕"}
                    </div>
                  {/if}
                </div>
              </div>
            </section>
          </div>
        </div>
      </div>
    </div>
  </div>
</main>

<!-- Clip Confirmation Dialog -->
{#if show_clip_confirm}
  <div class="fixed inset-0 z-[100] flex items-center justify-center">
    <div
      class="absolute inset-0 bg-black/60 backdrop-blur-md"
      role="button"
      tabindex="0"
      aria-label="关闭对话框"
      on:click={() => (show_clip_confirm = false)}
      on:keydown={(e) => {
        if (e.key === "Escape" || e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          show_clip_confirm = false;
        }
      }}
    />

    <div
      role="dialog"
      aria-modal="true"
      class="relative mx-4 w-full max-w-md rounded-2xl bg-[#1c1c1e] border border-white/10 shadow-2xl ring-1 ring-black/5"
    >
      <div class="p-5">
        <h3 class="text-[17px] font-semibold text-white">确认生成切片</h3>
        <p class="mt-1 text-[13px] text-white/70">请确认以下设置后继续</p>

        <div class="mt-3 space-y-3">
          <div class="text-[13px] font-medium text-white/90">
            待合并选区列表
          </div>
          <div
            class="max-h-48 overflow-y-auto space-y-2 custom-scrollbar-light"
          >
            {#each activeRanges as range, index}
              <div
                class="flex items-center justify-between px-3 py-2 bg-[#2c2c2e] rounded-lg border border-white/5 hover:border-white/10 transition-colors"
              >
                <div class="flex items-center space-x-3">
                  <div
                    class="flex items-center justify-center w-6 h-6 rounded-full bg-[#0A84FF]/20 text-[#0A84FF] text-[11px] font-semibold"
                  >
                    {index + 1}
                  </div>
                  <div class="flex flex-col space-y-0.5">
                    <div class="text-[12px] text-white/90">
                      {format_time(range.start * 1000)} → {format_time(
                        range.end * 1000
                      )}
                    </div>
                    <div class="text-[11px] text-white/60">
                      时长: {format_duration_seconds(range.end - range.start)}
                    </div>
                  </div>
                </div>
              </div>
            {/each}
          </div>
          <div
            class="mt-2 pt-2 border-t border-white/10 text-[15px] font-semibold text-white"
          >
            总时长: {format_duration_seconds(
              activeRanges.reduce(
                (acc, range) => acc + range.end - range.start,
                0
              )
            )}
          </div>
        </div>

        <div class="mt-3 space-y-3">
          <div class="mt-1 text-[13px] text-white/80">> 切片备注（可选）</div>
          <input
            type="text"
            id="confirm-clip-note-input"
            bind:value={clip_note}
            class="w-full px-3 py-2 bg-[#2c2c2e] text-white rounded-lg
                   border border-gray-800/50 focus:border-[#0A84FF]
                   transition duration-200 outline-none
                   placeholder-gray-500"
          />
        </div>

        <div class="mt-3 space-y-3">
          <label class="flex items-center gap-2.5">
            <input
              type="checkbox"
              id="confirm-danmu-checkbox"
              bind:checked={danmu_enabled}
              class="h-4 w-4 rounded border-white/30 bg-[#2c2c2e] text-[#0A84FF] accent-[#0A84FF] focus:outline-none focus:ring-2 focus:ring-[#0A84FF]/40"
            />
            <span class="text-[13px] text-white/80">压制弹幕</span>
          </label>

          <label class="flex items-center gap-2.5">
            <input
              type="checkbox"
              id="confirm-fix-encoding-checkbox"
              bind:checked={fix_encoding}
              class="h-4 w-4 rounded border-white/30 bg-[#2c2c2e] text-[#0A84FF] accent-[#0A84FF] focus:outline-none focus:ring-2 focus:ring-[#0A84FF]/40"
            />
            <span class="text-[13px] text-white/80"
              >修复编码（切片异常时使用）</span
            >
          </label>
        </div>
      </div>

      <div
        class="flex items-center justify-end gap-2 rounded-b-2xl border-t border-white/10 bg-[#111113] px-5 py-3"
      >
        <button
          on:click={() => (show_clip_confirm = false)}
          class="px-3.5 py-2 text-[13px] rounded-lg border border-white/20 text-white/90 hover:bg-white/10 transition-colors"
        >
          取消
        </button>
        <button
          on:click={confirm_generate_clip}
          class="px-3.5 py-2 text-[13px] rounded-lg bg-[#0A84FF] text-white shadow-[inset_0_1px_0_rgba(255,255,255,.15)] hover:bg-[#0A84FF]/90 transition-colors"
        >
          确认生成
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- Selection List Dialog -->
{#if show_selection_list}
  <div class="fixed inset-0 z-[100] flex items-center justify-center">
    <div
      class="absolute inset-0 bg-black/60 backdrop-blur-md"
      role="button"
      tabindex="0"
      aria-label="关闭对话框"
      on:click={() => (show_selection_list = false)}
      on:keydown={(e) => {
        if (e.key === "Escape" || e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          show_selection_list = false;
        }
      }}
    />

    <div
      role="dialog"
      aria-modal="true"
      class="relative mx-4 w-full max-w-md rounded-2xl bg-[#1c1c1e] border border-white/10 shadow-2xl ring-1 ring-black/5"
    >
      <div class="p-5">
        <div class="flex items-center justify-between mb-4">
          <h3 class="text-[17px] font-semibold text-white">选区管理</h3>
          <div class="flex items-center gap-3">
            <div class="text-[13px] text-white/60">
              共 {ranges.length} 个选区，已激活 {activeRanges.length} 个
            </div>
            <label class="flex items-center cursor-pointer select-none">
              <input
                type="checkbox"
                checked={ranges.length > 0 &&
                  ranges.every((r) => r.activated !== false)}
                on:change={handleSelectAll}
                class="h-4 w-4 rounded border-white/30 bg-[#1c1c1e] text-[#0A84FF] accent-[#0A84FF] focus:outline-none focus:ring-2 focus:ring-[#0A84FF]/40 cursor-pointer"
              />
            </label>
          </div>
        </div>

        <div class="space-y-3">
          <div
            class="max-h-[60vh] overflow-y-auto space-y-2 custom-scrollbar-light pr-1"
          >
            {#each ranges as range, index}
              <div
                class="flex items-center justify-between px-3 py-2 bg-[#2c2c2e] rounded-lg border border-white/5 hover:border-white/10 transition-colors"
                class:opacity-50={range.activated === false}
              >
                <div class="flex items-center space-x-3">
                  <div
                    class="flex items-center justify-center w-6 h-6 rounded-full bg-[#0A84FF]/20 text-[#0A84FF] text-[11px] font-semibold"
                  >
                    {index + 1}
                  </div>
                  <div class="flex flex-col space-y-0.5">
                    <div class="text-[12px] text-white/90">
                      {format_time(range.start * 1000)} → {format_time(
                        range.end * 1000
                      )}
                    </div>
                    <div class="text-[11px] text-white/60">
                      时长: {format_duration_seconds(range.end - range.start)}
                    </div>
                  </div>
                </div>
                <label class="flex items-center cursor-pointer p-1">
                  <input
                    type="checkbox"
                    checked={range.activated !== false}
                    on:change={(e) => handleRangeChange(e, range)}
                    class="h-5 w-5 rounded border-white/30 bg-[#1c1c1e] text-[#0A84FF] accent-[#0A84FF] focus:outline-none focus:ring-2 focus:ring-[#0A84FF]/40 cursor-pointer"
                  />
                </label>
              </div>
            {/each}
            {#if ranges.length === 0}
              <div class="text-center py-8 text-white/40 text-[13px]">
                暂无选区
              </div>
            {/if}
          </div>
        </div>
      </div>

      <div
        class="flex items-center justify-end gap-2 rounded-b-2xl border-t border-white/10 bg-[#111113] px-5 py-3"
      >
        <button
          on:click={deleteActivatedRanges}
          class="px-3.5 py-2 text-[13px] rounded-lg border border-red-500/20 text-red-500 hover:bg-red-500/10 transition-colors"
        >
          删除选区
        </button>
        <button
          on:click={() => (show_selection_list = false)}
          class="px-3.5 py-2 text-[13px] rounded-lg bg-[#0A84FF] text-white shadow-[inset_0_1px_0_rgba(255,255,255,.15)] hover:bg-[#0A84FF]/90 transition-colors"
        >
          完成
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  main {
    width: 100vw;
    height: 100vh;
  }

  .collapse-btn {
    position: absolute;
    z-index: 50;
    top: 50%;
    width: 20px;
    height: 40px;
  }
  .collapse-btn.rp {
    left: -20px;
    border-radius: 4px 0 0 4px;
    border: 2px solid rgb(31 41 55 / var(--tw-border-opacity));
    border-right: none;
    background-color: rgb(3 7 18 / var(--tw-bg-opacity));
    transform: translateY(-50%);
  }
  .collapse-btn.lp {
    right: -20px;
    border-radius: 0 4px 4px 0;
    border: 2px solid rgb(31 41 55 / var(--tw-border-opacity));
    border-left: none;
    background-color: rgb(3 7 18 / var(--tw-bg-opacity));
    transform: translateY(-50%);
  }

  /* 弹幕列表滚动条样式 */
  .sidebar-scrollbar::-webkit-scrollbar {
    width: 6px;
  }

  .sidebar-scrollbar::-webkit-scrollbar-track {
    background: rgba(44, 44, 46, 0.3);
    border-radius: 3px;
  }

  .sidebar-scrollbar::-webkit-scrollbar-thumb {
    background: rgba(10, 132, 255, 0.5);
    border-radius: 3px;
  }

  .sidebar-scrollbar::-webkit-scrollbar-thumb:hover {
    background: rgba(10, 132, 255, 0.7);
  }

  /* 虚拟滚动优化 */
  .danmu-container {
    will-change: scroll-position;
    contain: layout style paint;
  }

  .danmu-item {
    contain: layout style paint;
    will-change: transform;
  }
</style>
