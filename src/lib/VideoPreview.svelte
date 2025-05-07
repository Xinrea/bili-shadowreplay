<script lang="ts">
  import {
    Play,
    ArrowLeft,
    Plus,
    Minus,
    Pause,
    Film,
    Settings,
    Trash2,
    BrainCircuit,
    Eraser,
    Download,
  } from "lucide-svelte";
  import {
    generateEventId,
    parseSubtitleStyle,
    type ProgressFinished,
    type ProgressUpdate,
    type SubtitleStyle,
    type VideoItem,
  } from "./interface";
  import SubtitleStyleEditor from "./SubtitleStyleEditor.svelte";
  import { invoke, TAURI_ENV, listen } from "../lib/invoker";
  import { onDestroy } from "svelte";

  export let show = false;
  export let video: VideoItem;
  export let onClose: () => void;
  export let roomId: number;
  export let videos: any[] = [];
  export let onVideoChange: ((video: VideoItem) => void) | undefined =
    undefined;
  export let onVideoListUpdate: (() => Promise<void>) | undefined = undefined;

  interface Subtitle {
    startTime: number;
    endTime: number;
    text: string;
  }

  let subtitles: Subtitle[] = [];
  let currentTime = 0;
  let currentSubtitle = "";
  let videoElement: HTMLVideoElement;
  let timelineWidth = 0;
  let timelineElement: HTMLElement;
  let draggingSubtitle: { index: number; isStart: boolean } | null = null;
  let draggingBlock: number | null = null;
  let timelineScale = 1; // 时间轴缩放比例，1 表示正常大小
  let isPlaying = false;
  let timeMarkers: number[] = [];
  let dragOffset: number = 0; // 添加拖动偏移量
  let isVideoLoaded = false;
  let showStyleEditor = false;
  let volume = 1;
  let previousVolume = 1;
  let isMuted = false;
  let currentSubtitleIndex = -1;
  let subtitleElements: HTMLElement[] = [];
  let timelineContainer: HTMLElement;
  let showEncodeModal = false;
  let videoWidth = 0;
  let videoHeight = 0;
  let subtitleStyle: SubtitleStyle = {
    fontName: "Arial",
    fontSize: 18,
    fontColor: "#FFFFFF",
    outlineColor: "#000000",
    outlineWidth: 2,
    alignment: 2,
    marginV: 20,
    marginL: 20,
    marginR: 20,
  };

  let current_encode_event_id = null;
  let current_generate_event_id = null;

  const update_listener = listen<ProgressUpdate>(`progress-update`, (e) => {
    let event_id = e.payload.id;
    console.log(e.payload);
    if (event_id == current_encode_event_id) {
      update_encode_prompt(e.payload.content);
    } else if (event_id == current_generate_event_id) {
      update_generate_prompt(e.payload.content);
    }
  });

  const finish_listener = listen<ProgressFinished>(`progress-finished`, (e) => {
    let event_id = e.payload.id;
    if (event_id == current_encode_event_id) {
      update_encode_prompt(`压制字幕`);
      if (!e.payload.success) {
        alert("压制失败: " + e.payload.message);
      }
      current_encode_event_id = null;
    } else if (event_id == current_generate_event_id) {
      update_generate_prompt(`AI 生成字幕`);
      if (!e.payload.success) {
        alert("生成字幕失败: " + e.payload.message);
      }
      current_generate_event_id = null;
    }
  });

  onDestroy(() => {
    update_listener?.then((fn) => fn());
    finish_listener?.then((fn) => fn());
  });

  function update_encode_prompt(content: string) {
    const encode_prompt = document.getElementById("encode-prompt");
    if (encode_prompt) {
      encode_prompt.textContent = content;
    }
  }

  function update_generate_prompt(content: string) {
    const generate_prompt = document.getElementById("generate-prompt");
    if (generate_prompt) {
      generate_prompt.textContent = content;
    }
  }

  // 监听当前字幕索引变化
  $: if (currentSubtitleIndex >= 0 && subtitleElements[currentSubtitleIndex]) {
    subtitleElements[currentSubtitleIndex].scrollIntoView({
      behavior: "smooth",
      block: "nearest",
    });
  }

  function parseSrtTime(time: string): number {
    // hours:minutes:seconds,milliseconds
    time = time.replace(",", ".");
    const [hours, minutes, seconds] = time.split(":").map(Number);
    return hours * 3600 + minutes * 60 + seconds;
  }

  function formatSrtTime(time: number): string {
    const hours = Math.floor(time / 3600);
    const minutes = Math.floor((time % 3600) / 60);
    const seconds = Math.floor(time % 60);
    const milliseconds = Math.floor((time % 1) * 1000);
    return `${hours.toString().padStart(2, "0")}:${minutes.toString().padStart(2, "0")}:${seconds.toString().padStart(2, "0")},${milliseconds.toString().padStart(3, "0")}`;
  }

  function srtToSubtitles(srt: string): Subtitle[] {
    if (!srt.trim()) return [];

    // Split by double newlines to separate subtitle blocks
    const blocks = srt.split(/\n\s*\n/);

    return blocks
      .map((block) => {
        // Split block into lines and filter out empty lines
        const lines = block.split("\n").filter((line) => line.trim());

        // Skip if block doesn't have enough lines
        if (lines.length < 3) return null;

        // Skip the first line (subtitle number)
        const timeLine = lines[1];
        const text = lines.slice(2).join("\n");

        // Parse time line (format: "00:00:00,000 --> 00:00:00,000")
        const [startTime, endTime] = timeLine.split(" --> ").map(parseSrtTime);

        return {
          startTime,
          endTime,
          text,
        };
      })
      .filter((subtitle): subtitle is Subtitle => subtitle !== null)
      .sort((a, b) => a.startTime - b.startTime);
  }

  function subtitlesToSrt(subtitles: Subtitle[]): string {
    return subtitles
      .map((subtitle, index) => {
        return `${index + 1}\n${formatSrtTime(subtitle.startTime)} --> ${formatSrtTime(subtitle.endTime)}\n${subtitle.text}\n`;
      })
      .join("\n");
  }

  // 保存字幕到 localStorage
  async function saveSubtitles() {
    if (video?.file) {
      try {
        console.log("update video subtitle");
        await invoke("update_video_subtitle", {
          id: video.id,
          subtitle: subtitlesToSrt(subtitles),
        });
      } catch (error) {
        console.warn(error);
      }
    }
  }

  async function generateSubtitles() {
    if (video?.file) {
      current_generate_event_id = generateEventId();
      const savedSubtitles = (await invoke("generate_video_subtitle", {
        eventId: current_generate_event_id,
        id: video.id,
      })) as string;
      subtitles = srtToSubtitles(savedSubtitles);
    }
  }

  // 从 localStorage 加载字幕
  async function loadSubtitles() {
    if (video?.file) {
      const savedSubtitles = (await invoke("get_video_subtitle", {
        id: video.id,
      })) as string;
      if (savedSubtitles) {
        subtitles = srtToSubtitles(savedSubtitles);
      }
    }
  }

  // 加载字幕样式
  function loadSubtitleStyle() {
    const savedStyle = localStorage.getItem(`subtitle_style_${roomId}`);
    if (savedStyle) {
      subtitleStyle = JSON.parse(savedStyle);
    }
  }

  $: if (show) {
    isVideoLoaded = false;
    subtitles = []; // 清空字幕列表
    currentSubtitleIndex = -1;
    subtitleElements = [];
    loadSubtitleStyle(); // 加载字幕样式
  }

  // 监听样式编辑器关闭，重新加载样式
  $: if (!showStyleEditor) {
    loadSubtitleStyle();
  }

  async function handleClose() {
    if (videoElement) {
      videoElement.pause();
      videoElement.currentTime = 0;
    }
    isPlaying = false;
    currentTime = 0;
    currentSubtitle = "";
    currentSubtitleIndex = -1;
    subtitleElements = [];
    isVideoLoaded = false;
    await saveSubtitles(); // 保存字幕
    subtitles = []; // 清空字幕列表
    onClose();
  }

  async function handleVideoLoaded() {
    isVideoLoaded = true;
    if (videoElement) {
      videoElement.currentTime = 0;
      videoElement.pause();
      videoElement.volume = volume;
      isPlaying = false;
      currentTime = 0;
      currentSubtitle = "";
      currentSubtitleIndex = -1;
      // 获取视频实际尺寸
      videoWidth = videoElement.videoWidth;
      videoHeight = videoElement.videoHeight;
    }
    await loadSubtitles(); // 加载保存的字幕
  }

  function updateTimeMarkers() {
    if (!isVideoLoaded || !videoElement?.duration || !timelineWidth) {
      timeMarkers = [];
      return;
    }

    const duration = videoElement.duration;
    const minMarkerWidth = 100; // 最小标记宽度（像素）
    const maxMarkers = Math.floor(timelineWidth / minMarkerWidth);
    const interval = Math.ceil(duration / maxMarkers);

    timeMarkers = Array.from(
      { length: Math.min(Math.ceil(duration / interval) + 1, maxMarkers) },
      (_, i) => Math.min(i * interval, duration),
    );
  }

  function formatTime(seconds: number): string {
    const minutes = Math.floor(seconds / 60);
    const remainingSeconds = seconds % 60;
    return `${minutes}:${remainingSeconds.toFixed(1).padStart(4, "0")}`;
  }

  function togglePlay() {
    if (isPlaying) {
      videoElement.pause();
    } else {
      videoElement.play();
    }
    isPlaying = !isPlaying;
  }

  function handleTimeUpdate() {
    currentTime = videoElement.currentTime;
    // Find current subtitle
    currentSubtitleIndex = getCurrentSubtitleIndex();
    const currentSub = subtitles[currentSubtitleIndex];
    currentSubtitle = currentSub?.text || "";
  }

  function handleVideoEnded() {
    isPlaying = false;
  }

  function handleTimelineClick(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();

    if (!timelineElement || !videoElement) return;
    const rect = timelineElement.getBoundingClientRect();
    const x = Math.max(0, Math.min(e.clientX - rect.left, rect.width));
    const time = (x / rect.width) * videoElement.duration;
    videoElement.currentTime = time;
  }

  function addSubtitle() {
    const newStartTime = currentTime;
    const newEndTime = Math.min(currentTime + 5, videoElement.duration);
    subtitles = [
      ...subtitles,
      {
        startTime: newStartTime,
        endTime: newEndTime,
        text: "",
      },
    ];
    subtitles.sort((a, b) => a.startTime - b.startTime);
  }

  function updateSubtitleTime(index: number, isStart: boolean, time: number) {
    subtitles = subtitles.map((sub, i) => {
      if (i !== index) return sub;
      if (isStart) {
        return { ...sub, startTime: Math.min(time, sub.endTime - 0.1) };
      } else {
        return { ...sub, endTime: Math.max(time, sub.startTime + 0.1) };
      }
    });
    subtitles = subtitles.sort((a, b) => a.startTime - b.startTime);
  }

  function moveSubtitle(index: number, newStartTime: number) {
    const sub = subtitles[index];
    const duration = sub.endTime - sub.startTime;
    const newEndTime = Math.min(newStartTime + duration, videoElement.duration);
    const newStartTimeFinal = Math.max(0, newStartTime);

    subtitles = subtitles.map((s, i) =>
      i === index
        ? { ...s, startTime: newStartTimeFinal, endTime: newEndTime }
        : s,
    );
    subtitles = subtitles.sort((a, b) => a.startTime - b.startTime);
  }

  async function removeSubtitle(index: number) {
    subtitles = subtitles.filter((_, i) => i !== index);
    await saveSubtitles(); // 删除字幕时保存
  }

  async function clearSubtitles() {
    subtitles = [];
    await saveSubtitles(); // 清空字幕时保存
  }

  function seekToTime(time: number) {
    videoElement.currentTime = time;
  }

  function adjustTime(index: number, isStart: boolean, delta: number) {
    const sub = subtitles[index];
    if (isStart) {
      const newTime = Math.max(0, sub.startTime + delta);
      if (newTime < sub.endTime - 0.1) {
        subtitles = subtitles.map((s, i) =>
          i === index ? { ...s, startTime: newTime } : s,
        );
        subtitles = subtitles.sort((a, b) => a.startTime - b.startTime);
      }
    } else {
      const newTime = Math.min(videoElement.duration, sub.endTime + delta);
      if (newTime > sub.startTime + 0.1) {
        subtitles = subtitles.map((s, i) =>
          i === index ? { ...s, endTime: newTime } : s,
        );
        subtitles = subtitles.sort((a, b) => a.startTime - b.startTime);
      }
    }
  }

  function handleTimelineMouseDown(
    e: MouseEvent,
    index: number,
    isStart: boolean,
  ) {
    draggingSubtitle = { index, isStart };
    document.addEventListener("mousemove", handleTimelineMouseMove);
    document.addEventListener("mouseup", handleTimelineMouseUp);
  }

  function handleBlockMouseDown(e: MouseEvent, index: number) {
    draggingBlock = index;
    const sub = subtitles[index];
    const rect = timelineElement.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const mouseTime = (x / rect.width) * videoElement.duration;

    // 检查是否点击了边缘
    const blockWidth =
      rect.width * ((sub.endTime - sub.startTime) / videoElement.duration);
    const blockLeft = rect.width * (sub.startTime / videoElement.duration);
    const relativeX = x - blockLeft;

    if (relativeX < 5) {
      // 开始边缘
      draggingSubtitle = { index, isStart: true };
      document.addEventListener("mousemove", handleTimelineMouseMove);
      document.addEventListener("mouseup", handleTimelineMouseUp);
    } else if (relativeX > blockWidth - 5) {
      // 结束边缘
      draggingSubtitle = { index, isStart: false };
      document.addEventListener("mousemove", handleTimelineMouseMove);
      document.addEventListener("mouseup", handleTimelineMouseUp);
    } else {
      // 中间部分
      dragOffset = mouseTime - sub.startTime;
      document.addEventListener("mousemove", handleBlockMouseMove);
      document.addEventListener("mouseup", handleBlockMouseUp);
    }
  }

  function handleTimelineMouseMove(e: MouseEvent) {
    if (!draggingSubtitle || !timelineElement) return;

    const rect = timelineElement.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const time = (x / rect.width) * videoElement.duration;

    updateSubtitleTime(draggingSubtitle.index, draggingSubtitle.isStart, time);
  }

  function handleBlockMouseMove(e: MouseEvent) {
    if (draggingBlock === null || !timelineElement) return;

    const rect = timelineElement.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const mouseTime = (x / rect.width) * videoElement.duration;
    const newStartTime = mouseTime - dragOffset; // 使用偏移量计算新的开始时间

    moveSubtitle(draggingBlock, newStartTime);
  }

  function handleTimelineMouseUp() {
    draggingSubtitle = null;
    document.removeEventListener("mousemove", handleTimelineMouseMove);
    document.removeEventListener("mouseup", handleTimelineMouseUp);
  }

  function handleBlockMouseUp() {
    draggingBlock = null;
    document.removeEventListener("mousemove", handleBlockMouseMove);
    document.removeEventListener("mouseup", handleBlockMouseUp);
  }

  function getSubtitleStyle(subtitle: Subtitle) {
    if (!isVideoLoaded || !videoElement?.duration) return "";
    const start =
      (subtitle.startTime / videoElement.duration) * 100 * timelineScale;
    const width =
      ((subtitle.endTime - subtitle.startTime) / videoElement.duration) *
      100 *
      timelineScale;
    return `left: ${start}%; width: ${width}%;`;
  }

  function handleVolumeChange(e: Event) {
    const input = e.target as HTMLInputElement;
    volume = parseFloat(input.value);
    if (videoElement) {
      videoElement.volume = volume;
    }
  }

  function toggleMute() {
    if (videoElement) {
      if (isMuted) {
        videoElement.volume = previousVolume;
      } else {
        previousVolume = videoElement.volume;
        videoElement.volume = 0;
      }
      isMuted = !isMuted;
    }
  }

  function getCurrentSubtitleIndex(): number {
    return subtitles.findIndex(
      (sub) => currentTime >= sub.startTime && currentTime < sub.endTime,
    );
  }

  function handleScaleChange(e: Event) {
    const input = e.target as HTMLInputElement;
    timelineScale = parseFloat(input.value);
    const rect = timelineElement.getBoundingClientRect();
    timelineWidth = rect.width;
    updateTimeMarkers();
  }

  function handleWheel(e: WheelEvent) {
    e.preventDefault();
    if (timelineContainer) {
      timelineContainer.scrollLeft += e.deltaY;
    }
  }

  async function encodeVideoSubtitle() {
    await saveSubtitles();
    const event_id = generateEventId();
    current_encode_event_id = event_id;
    const result = await invoke("encode_video_subtitle", {
      eventId: event_id,
      id: video.id,
      srtStyle: parseSubtitleStyle(subtitleStyle),
    });
    console.log(result);
    // 压制成功后更新视频列表
    await onVideoListUpdate?.();
  }

  function handleVideoSelect(e: Event) {
    const selectedVideo = videos.find(
      (v) => v.id === Number((e.target as HTMLSelectElement).value),
    );
    if (selectedVideo) {
      // 清空字幕列表
      subtitles = [];
      currentSubtitleIndex = -1;
      currentSubtitle = "";
      // 重置视频状态
      if (videoElement) {
        videoElement.currentTime = 0;
        videoElement.pause();
        isPlaying = false;
        currentTime = 0;
      }
      // 调用父组件的回调
      onVideoChange?.(selectedVideo);
    }
  }

  async function saveVideo() {
    if (!video) return;
    const video_url = video.file;
    const video_name = video.file;
    const a = document.createElement("a");
    a.href = video_url;
    a.download = video_name;
    a.click();
  }
</script>

{#if show}
  <div
    class="fixed inset-0 bg-[#1c1c1e] z-[1000] transition-opacity duration-200"
    class:opacity-0={!show}
    class:opacity-100={show}
  >
    <!-- 顶部导航栏 -->
    <div
      class="h-14 border-b border-gray-800/50 bg-[#2c2c2e] flex items-center px-4 justify-between"
    >
      <div class="flex items-center space-x-4">
        <button
          class="flex items-center space-x-2 text-gray-300 hover:text-white transition-colors duration-200 px-3 py-1.5 rounded-md hover:bg-gray-700/50"
          on:click={handleClose}
        >
          <ArrowLeft class="w-4 h-4" />
          <span class="text-sm">返回</span>
        </button>
        <!-- 视频选择器 -->
        <div class="relative flex items-center space-x-2">
          <select
            class="bg-[#1c1c1e] text-gray-300 text-sm rounded-md px-3 py-1.5 border border-gray-700 focus:border-[#0A84FF] outline-none appearance-none cursor-pointer hover:bg-[#2c2c2e] transition-colors duration-200"
            value={video.id}
            on:change={handleVideoSelect}
          >
            {#each videos as v}
              <option value={v.id}>{v.name}</option>
            {/each}
          </select>
          <!-- 保存按钮 -->
          {#if !TAURI_ENV}
            <button
              class="text-blue-500 hover:text-blue-400 transition-colors duration-200 px-2 py-1.5 rounded-md hover:bg-blue-500/10"
              on:click={saveVideo}
            >
              <Download class="w-4 h-4" />
            </button>
          {/if}
          <!-- 删除按钮 -->
          <button
            class="text-red-500 hover:text-red-400 transition-colors duration-200 px-2 py-1.5 rounded-md hover:bg-red-500/10"
            on:click={async () => {
              if (!video) return;
              try {
                await invoke("delete_video", { id: video.id });
                // 更新视频列表
                await onVideoListUpdate?.();
                // 如果列表不为空，选择新的视频
                if (videos.length > 0) {
                  const newVideo = videos[0];
                  onVideoChange?.(newVideo);
                } else {
                  // 如果列表为空，关闭预览
                  await handleClose();
                }
              } catch (error) {
                console.error(error);
                alert("删除失败：" + error);
              }
            }}
          >
            <Trash2 class="w-4 h-4" />
          </button>
        </div>
      </div>

      <div class="flex items-center space-x-2">
        <button
          class="px-4 py-1.5 text-sm bg-[#0A84FF] text-white rounded-md hover:bg-[#0A84FF]/90 transition-colors duration-200 border border-gray-600/50 flex items-center space-x-2 disabled:opacity-50 disabled:cursor-not-allowed"
          on:click={() => (showEncodeModal = true)}
          disabled={current_encode_event_id != null}
        >
          {#if current_encode_event_id != null}
            <svg
              class="animate-spin h-4 w-4"
              xmlns="http://www.w3.org/2000/svg"
              fill="none"
              viewBox="0 0 24 24"
            >
              <circle
                class="opacity-25"
                cx="12"
                cy="12"
                r="10"
                stroke="currentColor"
                stroke-width="4"
              ></circle>
              <path
                class="opacity-75"
                fill="currentColor"
                d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
              ></path>
            </svg>
          {:else}
            <Film class="w-4 h-4" />
          {/if}
          <span id="encode-prompt">压制字幕</span>
        </button>
      </div>
    </div>

    <!-- 编码确认 Modal -->
    {#if showEncodeModal}
      <div
        class="fixed inset-0 bg-black/50 z-[1100] flex items-center justify-center"
      >
        <div class="bg-[#2c2c2e] rounded-lg shadow-xl w-[480px] max-w-[90vw]">
          <!-- Modal 头部 -->
          <div
            class="px-4 py-3 border-b border-gray-800/50 flex items-center justify-between"
          >
            <h3 class="text-sm font-medium text-gray-200">确认压制</h3>
            <button
              class="text-gray-400 hover:text-white transition-colors duration-200"
              on:click={() => (showEncodeModal = false)}
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                class="h-4 w-4"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <line x1="18" y1="6" x2="6" y2="18"></line>
                <line x1="6" y1="6" x2="18" y2="18"></line>
              </svg>
            </button>
          </div>

          <!-- Modal 内容 -->
          <div class="p-4 text-gray-300">
            压制需要耗费一定的时间，请耐心等待；压制完成后，切片列表中会出现新的带有字幕的切片。确定要进行压制吗？
          </div>

          <!-- Modal 底部按钮 -->
          <div
            class="px-4 py-3 border-t border-gray-800/50 flex justify-end space-x-2"
          >
            <button
              class="px-4 py-1.5 text-sm bg-gray-700/50 text-gray-200 rounded-md hover:bg-gray-700/70 transition-colors duration-200 border border-gray-600/50"
              on:click={() => (showEncodeModal = false)}
            >
              取消
            </button>
            <button
              class="px-4 py-1.5 text-sm bg-[#0A84FF] text-white rounded-md hover:bg-[#0A84FF]/90 transition-colors duration-200 disabled:opacity-50 disabled:cursor-not-allowed flex items-center space-x-1"
              on:click={() => {
                showEncodeModal = false;
                encodeVideoSubtitle();
              }}
            >
              <span>确认</span>
            </button>
          </div>
        </div>
      </div>
    {/if}

    <div class="flex h-[calc(100vh-3.5rem)]">
      <!-- 视频区域 -->
      <div class="flex-1 flex flex-col">
        <!-- 视频容器 -->
        <div class="flex-1 bg-black relative">
          <div class="absolute inset-0 flex items-center">
            <!-- svelte-ignore a11y-media-has-caption -->
            <video
              bind:this={videoElement}
              src={video?.file}
              class="w-full h-auto max-h-full cursor-pointer"
              on:timeupdate={handleTimeUpdate}
              on:ended={handleVideoEnded}
              on:loadedmetadata={handleVideoLoaded}
              on:click={togglePlay}
            />
            <!-- 字幕显示 -->
            {#if currentSubtitle}
              <div
                class="absolute bottom-8 left-1/2 -translate-x-1/2 px-4 py-2 rounded-lg text-white"
                style="
                  font-family: {subtitleStyle.fontName};
                  font-size: {videoHeight * (subtitleStyle.fontSize / 720)}px;
                  color: {subtitleStyle.fontColor};
                  text-shadow: {`
                    ${subtitleStyle.outlineWidth}px ${subtitleStyle.outlineWidth}px 0 ${subtitleStyle.outlineColor},
                    -${subtitleStyle.outlineWidth}px ${subtitleStyle.outlineWidth}px 0 ${subtitleStyle.outlineColor},
                    ${subtitleStyle.outlineWidth}px -${subtitleStyle.outlineWidth}px 0 ${subtitleStyle.outlineColor},
                    -${subtitleStyle.outlineWidth}px -${subtitleStyle.outlineWidth}px 0 ${subtitleStyle.outlineColor}
                  `};
                  text-align: {subtitleStyle.alignment === 1
                  ? 'left'
                  : subtitleStyle.alignment === 2
                    ? 'center'
                    : 'right'};
                  margin-bottom: {videoHeight *
                  (subtitleStyle.marginV / 720)}px;
                "
              >
                {currentSubtitle}
              </div>
            {/if}
          </div>
        </div>

        <!-- 时间轴和控制条 -->
        <div class="h-32 bg-[#1c1c1e] border-t border-gray-800/50">
          <!-- 控制栏 -->
          <div
            class="h-8 px-4 flex items-center justify-between border-b border-gray-800/50"
          >
            <!-- 左侧控制 -->
            <div class="flex items-center space-x-2">
              <!-- 缩放控制 -->
              <span class="text-xs text-gray-400">缩放</span>
              <input
                type="range"
                min="1"
                max="10"
                step="0.1"
                value={timelineScale}
                on:input={handleScaleChange}
                class="w-32 h-1 bg-gray-600 rounded-lg appearance-none cursor-pointer"
              />
            </div>

            <!-- 中间播放控制 -->
            <div class="flex-1 flex justify-center">
              <button
                class="p-1.5 rounded-lg bg-[#2c2c2e] hover:bg-[#3c3c3e] transition-colors duration-200"
                on:click={togglePlay}
              >
                {#if isPlaying}
                  <Pause class="w-4 h-4 text-white" />
                {:else}
                  <Play class="w-4 h-4 text-white" />
                {/if}
              </button>
            </div>

            <!-- 音量控制 -->
            <div class="flex items-center space-x-2">
              <button
                class="text-white hover:text-gray-300 transition-colors duration-200"
                on:click={toggleMute}
              >
                {#if isMuted || volume === 0}
                  <svg
                    xmlns="http://www.w3.org/2000/svg"
                    class="h-4 w-4"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  >
                    <path d="M11 5L6 9H2v6h4l5 4V5z" />
                    <line x1="23" y1="9" x2="17" y2="15" />
                    <line x1="17" y1="9" x2="23" y2="15" />
                  </svg>
                {:else if volume < 0.5}
                  <svg
                    xmlns="http://www.w3.org/2000/svg"
                    class="h-4 w-4"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  >
                    <path d="M11 5L6 9H2v6h4l5 4V5z" />
                  </svg>
                {:else}
                  <svg
                    xmlns="http://www.w3.org/2000/svg"
                    class="h-4 w-4"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  >
                    <path d="M11 5L6 9H2v6h4l5 4V5z" />
                  </svg>
                {/if}
              </button>
              <input
                type="range"
                min="0"
                max="1"
                step="0.1"
                bind:value={volume}
                on:input={handleVolumeChange}
                class="w-20 h-1 bg-gray-600 rounded-lg appearance-none cursor-pointer"
              />
            </div>
          </div>

          <!-- 时间轴容器 -->
          <div
            class="h-24 overflow-x-auto overflow-y-hidden"
            bind:this={timelineContainer}
            on:wheel|preventDefault={handleWheel}
          >
            <!-- svelte-ignore a11y-click-events-have-key-events -->
            <div
              bind:this={timelineElement}
              class="relative h-full"
              style="width: {100 * timelineScale}%"
              on:mousemove={(e) => {
                if (!timelineElement) return;
                const rect = timelineElement.getBoundingClientRect();
                timelineWidth = rect.width;
                updateTimeMarkers();
              }}
              on:click|preventDefault|stopPropagation={handleTimelineClick}
            >
              <!-- 播放进度条 -->
              <div class="absolute top-0 left-0 right-0 h-1 bg-gray-700">
                <div
                  class="h-full bg-[#0A84FF]"
                  style="width: {(currentTime / (videoElement?.duration || 1)) *
                    100}%"
                />
              </div>

              <!-- 时间刻度 -->
              {#each timeMarkers as time}
                <div
                  class="absolute top-1 bottom-0 border-l border-gray-700"
                  style="left: {(time / (videoElement?.duration || 1)) * 100}%"
                >
                  <div
                    class="absolute top-2 left-1/2 -translate-x-1/2 text-xs text-gray-400 whitespace-nowrap"
                  >
                    {formatTime(time)}
                  </div>
                </div>
              {/each}

              <!-- 字幕块 -->
              {#each subtitles as subtitle, index}
                <div
                  bind:this={subtitleElements[index]}
                  class="absolute top-4 bottom-4 bg-[#0A84FF]/30 rounded-lg cursor-move"
                  style={getSubtitleStyle(subtitle)}
                  on:mousedown={(e) => handleBlockMouseDown(e, index)}
                >
                  <!-- 开始时间手柄 -->
                  <div
                    class="absolute left-0 top-0 bottom-0 w-1 bg-[#0A84FF] rounded-l cursor-ew-resize"
                    on:mousedown|stopPropagation={(e) =>
                      handleTimelineMouseDown(e, index, true)}
                  />
                  <!-- 结束时间手柄 -->
                  <div
                    class="absolute right-0 top-0 bottom-0 w-1 bg-[#0A84FF] rounded-r cursor-ew-resize"
                    on:mousedown|stopPropagation={(e) =>
                      handleTimelineMouseDown(e, index, false)}
                  />
                  <!-- 字幕文本预览 -->
                  <div
                    class="absolute inset-x-2 inset-y-1 flex items-center justify-center text-xs text-white text-center line-clamp-3 rounded"
                  >
                    {subtitle.text || "空字幕"}
                  </div>
                </div>
              {/each}
            </div>
          </div>
        </div>
      </div>

      <!-- 字幕编辑面板 -->
      <div
        class="w-80 border-l border-gray-800/50 bg-[#2c2c2e] overflow-y-auto"
      >
        <div class="p-4 space-y-4">
          <div class="w-full sticky top-0 bg-[#2c2c2e] z-10 pb-4">
            <div class="flex flex-col space-y-2">
              <div class="flex space-x-2">
                <button
                  class="flex-1 px-3 py-1.5 text-sm bg-[#1c1c1e] text-gray-300 rounded-lg hover:bg-[#2c2c2e] transition-colors duration-200 flex items-center justify-center space-x-1 border border-gray-700"
                  on:click={() => (showStyleEditor = true)}
                >
                  <Settings class="w-4 h-4" />
                  <span>字幕样式</span>
                </button>
                <button
                  class="flex-1 px-3 py-1.5 text-sm bg-[#1c1c1e] text-gray-300 rounded-lg hover:bg-[#2c2c2e] transition-colors duration-200 flex items-center justify-center space-x-1 border border-gray-700"
                  on:click={clearSubtitles}
                >
                  <Eraser class="w-4 h-4" />
                  <span>清空列表</span>
                </button>
              </div>
              <div class="flex space-x-2">
                <button
                  class="flex-1 px-3 py-1.5 text-sm bg-[#1c1c1e] text-gray-300 rounded-lg hover:bg-[#2c2c2e] transition-colors duration-200 disabled:opacity-50 disabled:cursor-not-allowed flex items-center justify-center space-x-1 border border-gray-700"
                  on:click={generateSubtitles}
                  disabled={current_generate_event_id !== null}
                >
                  {#if current_generate_event_id !== null}
                    <svg
                      class="animate-spin h-4 w-4"
                      xmlns="http://www.w3.org/2000/svg"
                      fill="none"
                      viewBox="0 0 24 24"
                    >
                      <circle
                        class="opacity-25"
                        cx="12"
                        cy="12"
                        r="10"
                        stroke="currentColor"
                        stroke-width="4"
                      ></circle>
                      <path
                        class="opacity-75"
                        fill="currentColor"
                        d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
                      ></path>
                    </svg>
                  {:else}
                    <BrainCircuit class="w-4 h-4" />
                  {/if}
                  <span id="generate-prompt">AI 生成字幕</span>
                </button>
                <button
                  class="flex-1 px-3 py-1.5 text-sm bg-[#1c1c1e] text-gray-300 rounded-lg hover:bg-[#2c2c2e] transition-colors duration-200 flex items-center justify-center space-x-1 border border-gray-700"
                  on:click={addSubtitle}
                >
                  <Plus class="w-4 h-4" />
                  <span>手动添加</span>
                </button>
              </div>
            </div>
          </div>

          <!-- 字幕列表 -->
          <div class="space-y-2">
            {#each subtitles as subtitle, index}
              <div
                bind:this={subtitleElements[index]}
                class="p-3 bg-[#1c1c1e] rounded-lg space-y-2 transition-colors duration-200"
                class:bg-[#2c2c2e]={currentSubtitleIndex === index}
                class:border={currentSubtitleIndex === index}
                class:border-[#0A84FF]={currentSubtitleIndex === index}
              >
                <div class="flex justify-between items-center">
                  <div class="flex items-center space-x-4">
                    <div class="flex items-center space-x-1">
                      <button
                        class="text-sm text-[#0A84FF] hover:text-[#0A84FF]/80"
                        on:click={() => seekToTime(subtitle.startTime)}
                      >
                        {formatTime(subtitle.startTime)}
                      </button>
                      <button
                        class="p-0.5 text-gray-400 hover:text-white"
                        on:click={() => adjustTime(index, true, -0.1)}
                      >
                        <Minus class="w-3 h-3" />
                      </button>
                      <button
                        class="p-0.5 text-gray-400 hover:text-white"
                        on:click={() => adjustTime(index, true, 0.1)}
                      >
                        <Plus class="w-3 h-3" />
                      </button>
                    </div>
                    <span class="text-gray-400">→</span>
                    <div class="flex items-center space-x-1">
                      <button
                        class="text-sm text-[#0A84FF] hover:text-[#0A84FF]/80"
                        on:click={() => seekToTime(subtitle.endTime)}
                      >
                        {formatTime(subtitle.endTime)}
                      </button>
                      <button
                        class="p-0.5 text-gray-400 hover:text-white"
                        on:click={() => adjustTime(index, false, -0.1)}
                      >
                        <Minus class="w-3 h-3" />
                      </button>
                      <button
                        class="p-0.5 text-gray-400 hover:text-white"
                        on:click={() => adjustTime(index, false, 0.1)}
                      >
                        <Plus class="w-3 h-3" />
                      </button>
                    </div>
                  </div>
                  <button
                    class="text-sm text-red-500 hover:text-red-400"
                    on:click={async () => await removeSubtitle(index)}
                  >
                    删除
                  </button>
                </div>
                <input
                  type="text"
                  bind:value={subtitle.text}
                  class="w-full px-2 py-1 bg-[#2c2c2e] text-white rounded border border-gray-700 focus:border-[#0A84FF] outline-none"
                  placeholder="输入字幕文本"
                />
              </div>
            {/each}
          </div>
        </div>
      </div>
    </div>
  </div>
{/if}

<SubtitleStyleEditor
  bind:show={showStyleEditor}
  {roomId}
  onClose={() => (showStyleEditor = false)}
/>
