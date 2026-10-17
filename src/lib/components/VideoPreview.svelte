<script lang="ts">
  import {
    Play,
    ArrowLeft,
    Plus,
    Pause,
    Film,
    Settings,
    Trash2,
    BrainCircuit,
    Eraser,
    Download,
    Pen,
    Scissors,
    CircleHelp,
    Loader2,
  } from "lucide-svelte";
  import {
    generateEventId,
    type SubtitleStyle,
    type VideoItem,
    type Profile,
    type Config,
    default_profile,
  } from "../interface";
  import SubtitleStyleEditor from "./SubtitleStyleEditor.svelte";
  import AssSubtitlePreview from "./AssSubtitlePreview.svelte";
  import ShortcutHelpModal from "./ShortcutHelpModal.svelte";
  import { buildSubtitleAss, DEFAULT_SUBTITLE_STYLE, loadSubtitleStyle as readSubtitleStyle } from "../subtitle-ass";
  import CoverEditor from "./CoverEditor.svelte";
  import TypeSelect from "./TypeSelect.svelte";
  import InspectorPanel from "./InspectorPanel.svelte";
  import { announceTask, openTaskCenter, settleTask, wasCancelled } from "../stores/tasks";
  import {
    invoke,
    TAURI_ENV,
    log,
    close_window,
    get_static_url,
  } from "../invoker";
  import { onDestroy, onMount } from "svelte";
  import { createStateRefresh, watchStateChanges } from "../state-refresh";
  import { listen as tauriListen } from "@tauri-apps/api/event";
  import type { AccountInfo } from "../db";
  import WaveSurfer from "wavesurfer.js";

  interface Props {
    show?: boolean;
    video: VideoItem;
    roomId: string;
    videos?: any[];
    onVideoChange?: ((video: VideoItem) => void) | undefined;
    onVideoListUpdate?: (() => Promise<void>) | undefined;
  }

  let {
    show = $bindable(false),
    video = $bindable(),
    roomId,
    videos = [],
    onVideoChange = undefined,
    onVideoListUpdate = undefined
  }: Props = $props();

  interface Subtitle {
    startTime: number;
    endTime: number;
    text: string;
    layerOrder: number;
  }

  let subtitles: Subtitle[] = $state([]);
  let currentTime = $state(0);
  let videoElement: HTMLVideoElement = $state();
  let showDefaultCoverIcon = $state(false);
  let timelineWidth = $state(0);

  let timelineElement: HTMLElement = $state();
  let draggingSubtitle: { index: number; isStart: boolean } | null = $state(null);
  let draggingBlock: number | null = $state(null);
  let timelineScale = $state(1); // 时间轴缩放比例，1 表示正常大小
  let isPlaying = $state(false);
  let timeMarkers: number[] = $state([]);
  let dragOffset: number = 0; // 添加拖动偏移量
  let isVideoLoaded = $state(false);
  let showStyleEditor = $state(false);
  let volume = $state(1);
  let previousVolume = 1;
  let isMuted = $state(false);
  let currentSubtitleIndex = $state(-1);
  let currentSubtitleIndices: number[] = $state([]);
  let selectedSubtitleIndex = $state(-1); // 快捷键操作的字幕，与播放位置无关
  let subtitleRowElements: HTMLElement[] = $state([]);
  let subtitleBlockElements: HTMLElement[] = $state([]);
  let snapTime: number | null = $state(null); // 拖动字幕时当前吸附到的时间点
  let subtitleLanes: number[] = $state([]);
  let subtitleLaneCount = $state(1);
  let subtitleTrackHeight = $state(48);
  let timelineTotalHeight = $state(156);
  let nextSubtitleLayerOrder = 0;
  let timelineContainer: HTMLElement = $state();
  let showEncodeModal = $state(false);
  let previewContainer: HTMLElement = $state();
  let previewFrameWidth = $state(0);
  let subtitleStyle: SubtitleStyle = $state({ ...DEFAULT_SUBTITLE_STYLE });
  let subtitleAss = $derived(buildSubtitleAss(subtitles, subtitleStyle));

  let submittingEncode = $state(false);
  let current_generate_event_id = $state(null);
  let windowCloseUnlisten: (() => void) | null = null;
  let activeTab = $state("subtitle"); // 添加当前激活的 tab
  let inspectorCollapsed = $state(false);

  // 切片功能相关变量
  let clipStartTime = $state(0);
  let clipEndTime = $state(0);
  let clipTitle = $state("");
  let showShortcutHelp = $state(false);
  let lastVideoId = $state(-1); // 记录上一个视频ID，避免重复初始化
  let clipTimesSet = $state(false); // 标记用户是否主动设置过切片时间

  // 进度条拖动相关变量
  let isDraggingSeekbar = $state(false);
  let seekbarElement: HTMLElement = $state();
  let previewTime = $state(0); // 拖动时预览的时间
  let wasPlayingBeforeDrag = false; // 拖动前的播放状态

  // 投稿相关变量
  let current_post_event_id = $state(null);
  let config: Config = null;
  let accounts: any[] = $state([]);
  let uid_selected = $state(0);
  let show_cover_editor = $state(false);

  // WaveSurfer.js 相关变量
  let wavesurfer: any = null;
  let waveformContainer: HTMLElement = $state();
  let isWaveformLoaded = false;
  let isWaveformLoading = $state(false);
  let syncTimeout: ReturnType<typeof setTimeout> | null = null;

  // 字幕打轴相关
  const PREVIEW_SPAN = 0.5; // Q/W/E/D 试听片段长度，与 Aegisub 一致
  const MIN_SUBTITLE_DURATION = 0.1;
  const SNAP_THRESHOLD_PX = 8; // 拖动吸附距离（像素）
  let stopRangePlayback: (() => void) | null = null; // 结束当前的区间试听



  // 获取 profile 从 localStorage
  function get_profile(): Profile {
    const profile_str = window.localStorage.getItem("profile-" + roomId);
    if (profile_str && profile_str.includes("videos")) {
      return JSON.parse(profile_str);
    }
    return default_profile();
  }

  let profile: Profile = $state(get_profile());


  // 初始化 WaveSurfer.js
  async function initWaveSurfer() {
    if (typeof window === "undefined" || !video?.file) return;

    isWaveformLoading = true;

    try {
      await createWaveSurfer();
    } catch (error) {
      console.error("Failed to initialize WaveSurfer.js:", error);
      isWaveformLoading = false;
    }
  }

  async function createWaveSurfer() {
    // 使用更稳定的容器查找方式
    const container = document.querySelector(
      "[data-waveform-container]"
    ) as HTMLElement;

    if (!container || !video?.file) {
      console.log("Missing container or video file:", {
        container,
        videoFile: video?.file,
      });
      return;
    }

    // 波形容器跟随时间轴内容宽度，缩放由外层时间轴统一控制
    container.style.width = "100%";
    container.style.height = "46px";
    container.style.minHeight = "46px";
    container.style.display = "block";

    console.log("Creating WaveSurfer with:", {
      container: container,
      file: video.file,
      containerDimensions: {
        width: container.offsetWidth,
        height: container.offsetHeight,
      },
    });

    try {
      wavesurfer = WaveSurfer.create({
        container: container,
        waveColor: "rgba(196, 181, 253, 0.4)",
        progressColor: "#a78bfa",
        cursorWidth: 0, // 播放头由时间轴统一绘制
        barWidth: 2,
        barRadius: 1,
        height: 46,
        normalize: true,
        interact: true, // 启用交互，允许点击切换进度
        plugins: [],
      });

      // 加载音频
      await invoke("generate_audio_sample", {
        videoId: video.id,
      });
      let opus_file = video.file.replace(".mp4", ".opus");
      console.log("WaveSurfer created, loading file:", opus_file);
      wavesurfer.load(opus_file);

      // 监听加载完成
      wavesurfer.on("ready", () => {
        isWaveformLoaded = true;
        isWaveformLoading = false;
        console.log("Waveform loaded successfully");
        console.log("WaveSurfer instance:", wavesurfer);
      });

      // 监听点击事件，同步视频进度
      wavesurfer.on("interaction", (newTime: number) => {
        if (videoElement && videoElement.duration) {
          videoElement.currentTime = newTime;
          currentTime = newTime;
        }
      });

      // 监听错误
      wavesurfer.on("error", (e: any) => {
        console.error("WaveSurfer error:", e);
        isWaveformLoading = false;
      });

      // 监听加载进度
      wavesurfer.on("loading", (percent: number) => {
        console.log("WaveSurfer loading:", percent + "%");
      });
    } catch (error) {
      console.error("Failed to create WaveSurfer:", error);
    }
  }

  // 同步波形图与视频进度
  function syncWaveformWithVideo() {
    if (
      !wavesurfer ||
      !videoElement ||
      !isWaveformLoaded ||
      !videoElement.duration
    )
      return;

    try {
      const progress = videoElement.currentTime / videoElement.duration;
      wavesurfer.seekTo(progress);
    } catch (error) {
      console.warn("Failed to sync waveform:", error);
    }
  }

  // 销毁 WaveSurfer 实例
  function destroyWaveSurfer() {
    if (wavesurfer) {
      wavesurfer.destroy();
      wavesurfer = null;
      isWaveformLoaded = false;
    }
  }

  const videoListRefresh = createStateRefresh(async () => {
    await onVideoListUpdate?.();
  });
  onMount(() => watchStateChanges(videoListRefresh, ["videos-changed"]));

  // on window close, save subtitles
  onMount(async () => {
    if (TAURI_ENV) {
      // 使用 Tauri 的全局事件监听器
      try {
        windowCloseUnlisten = await tauriListen(
          "tauri://close-requested",
          async () => {
            await saveSubtitles();
          }
        );
      } catch (error) {
        log.warn("Failed to listen to window close event:", error);
      }
    } else {
      // 在非 Tauri 环境中使用 beforeunload
      window.addEventListener("beforeunload", async () => {
        await saveSubtitles();
      });
    }

    // 初始化投稿相关数据
    try {
      // 获取配置
      config = (await invoke("get_config")) as Config;

      // 获取账号列表
      const account_info: AccountInfo = await invoke("get_accounts");
      accounts = account_info.accounts
        .filter((a) => a.platform === "bilibili")
        .map((a) => ({
          value: a.uid,
          name: a.name,
          platform: a.platform,
        }));
    } catch (error) {
      console.error("Failed to initialize upload data:", error);
    }
  });

  onDestroy(() => {
    // 清理窗口关闭事件监听器
    if (windowCloseUnlisten) {
      windowCloseUnlisten();
    }
    stopRangePlayback?.();
    // 清理 WaveSurfer 实例
    destroyWaveSurfer();
  });

  // 投稿相关函数
  async function do_post() {
    if (!video || current_post_event_id) {
      return;
    }

    const eventId = generateEventId();
    current_post_event_id = eventId;
    // update profile in local storage
    window.localStorage.setItem("profile-" + roomId, JSON.stringify(profile));
    announceTask({
      id: eventId,
      task_type: "upload_procedure",
      metadata: JSON.stringify({ video_id: video.id, profile: { title: profile.title } }),
    });
    openTaskCenter(eventId);
    try {
      await invoke("upload_procedure", {
        uid: uid_selected,
        eventId,
        roomId: roomId,
        videoId: video.id,
        profile: profile,
      });
      uid_selected = 0;
      await onVideoListUpdate?.();
    } catch (error) {
      alert("投稿失败：" + error);
    } finally {
      current_post_event_id = null;
      settleTask(eventId);
    }
  }

  function pauseVideo() {
    if (videoElement) {
      videoElement.pause();
    }
  }


  function parseSrtTime(time: string): number {
    // hours:minutes:seconds,milliseconds
    // Only replace the comma that separates seconds and milliseconds, not the arrow separator
    const timeParts = time.split(",");
    if (timeParts.length !== 2) {
      console.warn("Invalid time format (missing comma):", time);
      return 0;
    }

    const timeWithoutMs = timeParts[0];
    const millisecondsStr = timeParts[1];

    const parts = timeWithoutMs.split(":");
    if (parts.length !== 3) {
      console.warn("Invalid time format:", time);
      return 0;
    }

    const [hours, minutes, seconds] = parts;
    const hoursNum = parseInt(hours, 10);
    const minutesNum = parseInt(minutes, 10);
    const secondsNum = parseInt(seconds, 10);

    // Pad milliseconds to 3 digits if needed
    const millisecondsNum = parseInt(millisecondsStr.padEnd(3, "0"), 10);

    if (
      isNaN(hoursNum) ||
      isNaN(minutesNum) ||
      isNaN(secondsNum) ||
      isNaN(millisecondsNum)
    ) {
      console.warn("Invalid time values:", time);
      return 0;
    }

    return (
      hoursNum * 3600 + minutesNum * 60 + secondsNum + millisecondsNum / 1000
    );
  }

  function formatSrtTime(time: number): string {
    // 先取整到毫秒：直接对浮点数取余会得到 5.6 % 1 = 0.5999…，每保存一次少 1ms
    const totalMs = Math.round(time * 1000);
    const hours = Math.floor(totalMs / 3600000);
    const minutes = Math.floor(totalMs / 60000) % 60;
    const seconds = Math.floor(totalMs / 1000) % 60;
    const milliseconds = totalMs % 1000;
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

        // Parse time line (format: "00:00:00,000 --> 00:00:00,000" or "00:00:00,000-->00:00:00,000")
        const timeParts = timeLine.split(/\s*-->\s*/);
        if (timeParts.length !== 2) {
          console.warn("Invalid time line format:", timeLine);
          return null;
        }

        const startTime = parseSrtTime(timeParts[0].trim());
        const endTime = parseSrtTime(timeParts[1].trim());

        if (isNaN(startTime) || isNaN(endTime)) {
          console.warn("Failed to parse time values:", timeLine);
          return null;
        }

        return {
          startTime,
          endTime,
          text,
          layerOrder: nextSubtitleLayerOrder++,
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
    if (!video?.file || current_generate_event_id) return;
    const eventId = generateEventId();
    current_generate_event_id = eventId;
    announceTask({
      id: eventId,
      task_type: "generate_video_subtitle",
      metadata: JSON.stringify({ video_id: video.id }),
    });
    openTaskCenter(eventId);
    try {
      const savedSubtitles = (await invoke("generate_video_subtitle", {
        eventId,
        id: video.id,
      })) as string;
      subtitles = srtToSubtitles(savedSubtitles);
      selectedSubtitleIndex = -1;
    } catch (error) {
      alert("生成字幕失败：" + error);
    } finally {
      current_generate_event_id = null;
      settleTask(eventId);
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
        selectedSubtitleIndex = -1;
      }
    }
  }

  // 加载字幕样式
  function loadSubtitleStyle() {
    subtitleStyle = readSubtitleStyle(roomId);
  }




  async function handleVideoLoaded() {
    isVideoLoaded = true;
    if (videoElement) {
      videoElement.currentTime = 0;
      videoElement.pause();
      videoElement.volume = volume;
      isPlaying = false;
      currentTime = 0;
      currentSubtitleIndex = -1;
      selectedSubtitleIndex = -1;
    }
    await loadSubtitles(); // 加载保存的字幕
    initClipTimes(); // 初始化切片时间

    // 初始化波形图
    setTimeout(() => {
      if (timelineElement) {
        timelineWidth = timelineElement.getBoundingClientRect().width;
        updateTimeMarkers();
      }
      initWaveSurfer();
    }, 100);
  }

  function updateTimeMarkers() {
    if (!isVideoLoaded || !videoElement?.duration || !timelineWidth) {
      timeMarkers = [];
      return;
    }

    const duration = videoElement.duration;
    const minMarkerWidth = 100; // 最小标记宽度（像素）
    const maxMarkers = Math.max(1, Math.floor(timelineWidth / minMarkerWidth));
    const interval = Math.max(1, Math.ceil(duration / maxMarkers));

    timeMarkers = Array.from(
      { length: Math.min(Math.ceil(duration / interval) + 1, maxMarkers) },
      (_, i) => Math.min(i * interval, duration)
    );
  }

  function formatTime(seconds: number): string {
    const minutes = Math.floor(seconds / 60);
    const remainingSeconds = seconds % 60;
    return `${minutes}:${remainingSeconds.toFixed(1).padStart(4, "0")}`;
  }

  function getVideoDisplayName(): string {
    const item = videos.find((candidate) => candidate.id === video?.id);
    return item?.name || video?.title || "当前视频";
  }

  // 切片功能相关函数
  function initClipTimes() {
    // 不做任何自动初始化，完全等待用户输入
    // 只初始化标题
    if (!clipTitle) {
      clipTitle = "";
    }
  }

  function setClipStartTime() {
    if (videoElement) {
      const newStartTime = videoElement.currentTime;

      // 如果没有选区（首次设置起点），自动将终点设置为视频结尾
      if (!clipTimesSet || clipEndTime === 0) {
        clipStartTime = newStartTime;
        clipEndTime = videoElement.duration; // 自动设置为视频结尾
      }
      // 如果新的开始时间在现有结束时间之后，自动设置终点为视频结尾
      else if (clipTimesSet && clipEndTime > 0 && newStartTime >= clipEndTime) {
        clipStartTime = newStartTime;
        clipEndTime = videoElement.duration; // 自动设置为视频结尾
      } else {
        clipStartTime = newStartTime;
      }

      clipTimesSet = true; // 标记用户已设置切片时间
    }
  }

  function setClipEndTime() {
    if (videoElement) {
      const newEndTime = videoElement.currentTime;

      // 如果没有选区（首次设置终点），自动将起点设置为视频开头
      if (!clipTimesSet || clipStartTime === 0) {
        clipStartTime = 0; // 自动设置为视频开头
        clipEndTime = newEndTime;
      }
      // 如果新的结束时间在现有开始时间之前，清空选区重新开始
      else if (
        clipTimesSet &&
        clipStartTime > 0 &&
        newEndTime <= clipStartTime
      ) {
        clipStartTime = 0; // 清空开始时间
        clipEndTime = newEndTime;
      } else {
        clipEndTime = newEndTime;
      }

      clipTimesSet = true; // 标记用户已设置切片时间
    }
  }

  function seekToClipStart() {
    if (videoElement) {
      videoElement.currentTime = clipStartTime;
    }
  }

  function seekToClipEnd() {
    if (videoElement) {
      videoElement.currentTime = clipEndTime;
    }
  }

  function clearClipSelection() {
    clipStartTime = 0;
    clipEndTime = 0;
    clipTimesSet = false; // 重置标记，恢复透明状态
  }

  async function generateClip() {
    if (!video) return;

    if (clipStartTime >= clipEndTime) {
      alert("开始时间必须小于结束时间");
      return;
    }

    if (clipEndTime - clipStartTime < 1) {
      alert("切片长度不能少于1秒");
      return;
    }

    // 如果没有设置切片标题，则以当前本地时间戳命名
    let title = clipTitle.trim();
    if (!title) {
      const now = new Date();
      const pad = (n) => n.toString().padStart(2, "0");
      const timestamp = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}_${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
      title = `clip_${timestamp}`;
    }

    const eventId = generateEventId();
    const request = {
      eventId,
      parentVideoId: video.id,
      startTime: clipStartTime,
      endTime: clipEndTime,
      clipTitle: title,
    };
    announceTask({
      id: eventId,
      task_type: "clip_video",
      metadata: JSON.stringify({
        parent_video_id: request.parentVideoId,
        start_time: request.startTime,
        end_time: request.endTime,
        clip_title: title,
      }),
    });
    openTaskCenter(eventId);
    // The range now belongs to this task, so the next clip starts fresh.
    clearClipSelection();
    clipTitle = "";

    try {
      await invoke("clip_video", request);
      await onVideoListUpdate?.();
    } catch (error) {
      const message = String(error);
      // Cancelled from the task center, or elsewhere such as the task page.
      if (!wasCancelled(eventId) && !message.endsWith("切片任务已取消")) {
        alert("切片生成失败：" + message);
      }
    } finally {
      settleTask(eventId);
    }
  }

  function canBeClipped(video: VideoItem): boolean {
    if (!video) {
      return false;
    }

    // 只要不是正在录制的视频(status !== -1)，都可以切片
    // 这包括：
    // - 导入的视频 (imported)
    // - 所有平台的切片 (clip, bilibili_clip, douyin_clip等)
    // - 录制完成的视频 (status === 0 或 status === 1)
    return video.status !== -1;
  }

  // 键盘快捷键处理：按物理键位（event.code）匹配，不受输入法和大小写影响
  function handleKeydown(event: KeyboardEvent) {
    if (!show || !isVideoLoaded || showStyleEditor || showShortcutHelp) return;

    const target = event.target as HTMLElement | null;
    const tagName = target?.tagName;

    // 如果当前焦点位于可编辑元素内，则跳过快捷键处理
    const isInInput =
      (!!tagName && ["INPUT", "TEXTAREA", "SELECT"].includes(tagName)) ||
      !!target?.isContentEditable ||
      !!target?.closest(
        "input, textarea, select, [contenteditable='true'], [data-hotkey-block]"
      );
    if (isInInput || event.altKey) return;
    // 只有方向键允许按住连发；其它键连发会反复重启试听或跟着播放头改写时间
    if (event.repeat && !event.code.startsWith("Arrow")) return;

    const clipEnabled = canBeClipped(video);

    // Aegisub 的 Ctrl+3 / Ctrl+4，macOS 上也接受 ⌘
    if (event.ctrlKey || event.metaKey) {
      if (event.shiftKey) return;
      switch (event.code) {
        case "Digit3":
          event.preventDefault();
          setSubtitleBoundaryToPlayhead(true);
          break;
        case "Digit4":
          event.preventDefault();
          setSubtitleBoundaryToPlayhead(false);
          break;
      }
      return;
    }

    if (event.shiftKey) {
      switch (event.code) {
        case "BracketLeft":
          if (clipEnabled) {
            event.preventDefault();
            seekToClipStart();
          }
          break;
        case "BracketRight":
          if (clipEnabled) {
            event.preventDefault();
            seekToClipEnd();
          }
          break;
      }
      return;
    }

    switch (event.code) {
      case "BracketLeft":
        if (clipEnabled) {
          event.preventDefault();
          setClipStartTime();
        }
        break;
      case "BracketRight":
        if (clipEnabled) {
          event.preventDefault();
          setClipEndTime();
        }
        break;
      case "KeyG":
        // 多个切片任务可以同时进行
        if (clipEnabled) {
          event.preventDefault();
          generateClip();
        }
        break;
      case "KeyC":
        if (clipEnabled) {
          event.preventDefault();
          clearClipSelection();
        }
        break;
      case "Space":
        event.preventDefault();
        togglePlay();
        break;
      case "ArrowLeft":
        event.preventDefault();
        if (videoElement) {
          videoElement.currentTime = Math.max(0, videoElement.currentTime - 5);
        }
        break;
      case "ArrowRight":
        event.preventDefault();
        if (videoElement) {
          videoElement.currentTime = Math.min(
            videoElement.duration,
            videoElement.currentTime + 5
          );
        }
        break;
      // 以下为 Aegisub 默认的打轴键位，作用于选中的字幕
      case "KeyZ":
        event.preventDefault();
        selectAdjacentSubtitle(-1);
        break;
      case "KeyX":
        event.preventDefault();
        selectAdjacentSubtitle(1);
        break;
      case "KeyS":
        event.preventDefault();
        playSubtitlePart("whole");
        break;
      case "KeyQ":
        event.preventDefault();
        playSubtitlePart("before");
        break;
      case "KeyW":
        event.preventDefault();
        playSubtitlePart("after");
        break;
      case "KeyE":
        event.preventDefault();
        playSubtitlePart("begin");
        break;
      case "KeyD":
        event.preventDefault();
        playSubtitlePart("end");
        break;
    }
  }

  // 跳转并等待 seeked：远距离跳转需要解码，不等待会吃掉试听片段的开头
  function seekVideo(time: number): Promise<void> {
    const video = videoElement;
    return new Promise((resolve) => {
      if (Math.abs(video.currentTime - time) < 0.001) {
        resolve();
        return;
      }
      let timer: ReturnType<typeof setTimeout>;
      const done = () => {
        clearTimeout(timer);
        video.removeEventListener("seeked", done);
        resolve();
      };
      timer = setTimeout(done, 2000); // 兜底，避免 seeked 事件丢失时一直等待
      video.addEventListener("seeked", done);
      video.currentTime = time;
      currentTime = time;
    });
  }

  // 播放 [start, end) 后自动暂停；从按键起，用户跳转或自行播放、暂停都会结束区间试听
  async function playRange(start: number, end: number) {
    const video = videoElement;
    if (!video?.duration) return;
    stopRangePlayback?.();

    start = Math.max(0, start);
    end = Math.min(video.duration, end);
    if (end - start < 0.01) return;

    let frame = 0;
    let stopped = false;
    let waitingForSeek = true;
    const stop = () => {
      stopped = true;
      cancelAnimationFrame(frame);
      video.removeEventListener("seeking", handleSeeking);
      video.removeEventListener("play", handlePlay);
      video.removeEventListener("timeupdate", checkEnd);
      if (stopRangePlayback === stop) stopRangePlayback = null;
    };
    // 跳转过程中 currentTime 即目标位置：目标不是 start 的跳转来自用户（时间轴、方向键等）
    const handleSeeking = () => {
      if (Math.abs(video.currentTime - start) > 0.001) stop();
    };
    // 还在等待跳转时用户自己开始播放，交给用户控制
    const handlePlay = () => {
      if (waitingForSeek) stop();
    };
    const checkEnd = () => {
      if (stopped) return;
      currentTime = video.currentTime;
      if (video.paused) {
        stop();
      } else if (video.currentTime >= end) {
        video.pause();
        isPlaying = false;
        stop();
      }
    };
    stopRangePlayback = stop;
    video.addEventListener("seeking", handleSeeking);
    video.addEventListener("play", handlePlay);

    video.pause();
    isPlaying = false;
    await seekVideo(start);
    if (stopped) return;
    waitingForSeek = false;
    try {
      await video.play();
    } catch {
      stop();
      return;
    }
    if (stopped) return;
    isPlaying = true;

    // timeupdate 每秒只触发约 4 次，用逐帧检查保证在 end 附近及时停下；
    // 窗口隐藏时 requestAnimationFrame 会暂停，timeupdate 作为兜底
    const tick = () => {
      checkEnd();
      if (!stopped) frame = requestAnimationFrame(tick);
    };
    video.addEventListener("timeupdate", checkEnd);
    frame = requestAnimationFrame(tick);
  }

  // 快捷键作用的字幕：优先用选中的字幕，未选中时取播放位置所在的字幕并选中它
  function resolveTargetSubtitle(): number {
    if (selectedSubtitleIndex >= 0 && selectedSubtitleIndex < subtitles.length) {
      return selectedSubtitleIndex;
    }
    const time = videoElement?.currentTime ?? 0;
    const index = subtitles.findIndex(
      (sub) => time >= sub.startTime && time < sub.endTime
    );
    if (index >= 0) selectedSubtitleIndex = index;
    return index;
  }

  function selectSubtitle(index: number, seek = false) {
    if (index < 0 || index >= subtitles.length) return;
    selectedSubtitleIndex = index;
    subtitleRowElements[index]?.scrollIntoView({
      behavior: "smooth",
      block: "nearest",
    });
    if (seek) {
      seekToTime(subtitles[index].startTime);
      subtitleBlockElements[index]?.scrollIntoView({
        behavior: "smooth",
        block: "nearest",
        inline: "nearest",
      });
    }
  }

  // Z / X：选中上一条 / 下一条字幕并跳到开头；未选中时以播放位置为基准
  function selectAdjacentSubtitle(direction: -1 | 1) {
    if (!videoElement) return;
    const time = videoElement.currentTime;
    let index: number;
    if (selectedSubtitleIndex >= 0 && selectedSubtitleIndex < subtitles.length) {
      index = selectedSubtitleIndex + direction;
    } else if (direction > 0) {
      index = subtitles.findIndex((sub) => sub.startTime > time + 0.001);
    } else {
      index = subtitles.length - 1;
      while (index >= 0 && subtitles[index].startTime >= time - 0.001) index--;
    }
    selectSubtitle(index, true);
  }

  // S / Q / W / E / D：试听选中字幕的整段、前后 500ms、开头或结尾 500ms
  function playSubtitlePart(
    part: "whole" | "before" | "after" | "begin" | "end"
  ) {
    const index = resolveTargetSubtitle();
    if (index < 0) return;
    const { startTime, endTime } = subtitles[index];
    const ranges = {
      whole: [startTime, endTime],
      before: [startTime - PREVIEW_SPAN, startTime],
      after: [endTime, endTime + PREVIEW_SPAN],
      begin: [startTime, Math.min(startTime + PREVIEW_SPAN, endTime)],
      end: [Math.max(endTime - PREVIEW_SPAN, startTime), endTime],
    };
    const [start, end] = ranges[part];
    playRange(start, end);
  }

  // Ctrl/⌘+3、Ctrl/⌘+4：把选中字幕的开始 / 结束设为当前播放位置
  function setSubtitleBoundaryToPlayhead(isStart: boolean) {
    if (!videoElement?.duration) return;
    const index = resolveTargetSubtitle();
    if (index < 0) return;

    const videoDuration = videoElement.duration;
    const time = videoElement.currentTime;
    const sub = subtitles[index];
    const length = Math.max(sub.endTime - sub.startTime, MIN_SUBTITLE_DURATION);
    let { startTime, endTime } = sub;
    // 越过另一端时保持原时长，而不是把字幕压成最短
    if (isStart) {
      startTime = Math.min(time, videoDuration - MIN_SUBTITLE_DURATION);
      if (endTime - startTime < MIN_SUBTITLE_DURATION) {
        endTime = Math.min(startTime + length, videoDuration);
      }
    } else {
      endTime = Math.max(time, MIN_SUBTITLE_DURATION);
      if (endTime - startTime < MIN_SUBTITLE_DURATION) {
        startTime = Math.max(endTime - length, 0);
      }
    }
    subtitles[index] = { ...sub, startTime, endTime };
    sortSubtitles();
  }

  // 按开始时间重新排序，选中的仍是同一条字幕
  function sortSubtitles() {
    const selected = subtitles[selectedSubtitleIndex];
    subtitles = [...subtitles].sort((a, b) => a.startTime - b.startTime);
    selectedSubtitleIndex = selected ? subtitles.indexOf(selected) : -1;
  }

  // 字幕输入框内按 Enter / Esc 退出编辑，回到快捷键操作
  function handleSubtitleInputKeydown(event: KeyboardEvent) {
    if (event.isComposing || event.keyCode === 229) return; // 输入法选词中
    if (event.key === "Enter" || event.key === "Escape") {
      event.preventDefault();
      (event.currentTarget as HTMLInputElement).blur();
    }
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

    // 同步波形图进度
    syncWaveformWithVideo();
  }

  function handleVideoEnded() {
    isPlaying = false;
  }

  function handleTimelineClick(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();

    // 如果正在拖动进度条，不处理点击事件
    if (isDraggingSeekbar) return;

    if (!timelineElement || !videoElement) return;
    const rect = timelineElement.getBoundingClientRect();
    const x = Math.max(0, Math.min(e.clientX - rect.left, rect.width));
    // 直接使用容器的实际宽度计算时间，与 getSubtitleStyle 逻辑保持一致
    const time = (x / rect.width) * videoElement.duration;
    videoElement.currentTime = time;
  }

  // 进度条拖动事件处理
  function handleSeekbarMouseDown(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();

    if (!videoElement || !seekbarElement) return;

    isDraggingSeekbar = true;
    wasPlayingBeforeDrag = isPlaying;

    // 先初始化预览时间为当前时间，避免跳跃
    previewTime = videoElement.currentTime;

    // 然后计算鼠标位置对应的时间
    const rect = seekbarElement.getBoundingClientRect();
    const x = Math.max(0, Math.min(e.clientX - rect.left, rect.width));
    const newTime = (x / rect.width) * videoElement.duration;
    previewTime = newTime;

    // 拖动到松手才会跳转，先结束区间试听，免得等待中的试听在拖动时开始播放
    stopRangePlayback?.();

    // 暂停播放
    if (isPlaying) {
      videoElement.pause();
      isPlaying = false;
    }

    // 添加全局事件监听器
    document.addEventListener("mousemove", handleSeekbarMouseMove);
    document.addEventListener("mouseup", handleSeekbarMouseUp);
  }

  function handleSeekbarMouseMove(e: MouseEvent) {
    if (!isDraggingSeekbar || !seekbarElement || !videoElement) return;

    const rect = seekbarElement.getBoundingClientRect();
    const x = Math.max(0, Math.min(e.clientX - rect.left, rect.width));
    const newTime = (x / rect.width) * videoElement.duration;
    previewTime = newTime;
  }

  function handleSeekbarMouseUp(e: MouseEvent) {
    if (!isDraggingSeekbar) return;

    // 应用最终时间
    if (videoElement) {
      videoElement.currentTime = previewTime;
      // 立即同步currentTime变量，避免视觉偏移
      currentTime = previewTime;
    }

    isDraggingSeekbar = false;

    // 移除全局事件监听器
    document.removeEventListener("mousemove", handleSeekbarMouseMove);
    document.removeEventListener("mouseup", handleSeekbarMouseUp);

    // 恢复播放状态
    if (wasPlayingBeforeDrag && videoElement) {
      videoElement.play();
      isPlaying = true;
    }
  }

  function calculateSubtitleLanes(items: Subtitle[]): {
    lanes: number[];
    count: number;
  } {
    const lanes = new Array(items.length).fill(0);
    const laneItems: number[][] = [];
    const sortedIndices = items
      .map((_, index) => index)
      .sort(
        (a, b) =>
          items[a].layerOrder - items[b].layerOrder ||
          a - b
      );

    for (const index of sortedIndices) {
      const subtitle = items[index];
      let lane = laneItems.findIndex((assignedIndices) =>
        assignedIndices.every((assignedIndex) => {
          const assigned = items[assignedIndex];
          return (
            subtitle.endTime <= assigned.startTime + 0.001 ||
            subtitle.startTime >= assigned.endTime - 0.001
          );
        })
      );
      if (lane === -1) {
        lane = laneItems.length;
        laneItems.push([]);
      }
      laneItems[lane].push(index);
      lanes[index] = lane;
    }

    return { lanes, count: Math.max(1, laneItems.length) };
  }

  function insertSubtitleAfter(index: number) {
    if (!videoElement?.duration) return;

    const current = subtitles[index];
    if (!current) return;

    const startTime = current.endTime;
    if (startTime >= videoElement.duration - 0.1) {
      alert("当前字幕已位于视频末尾，无法继续添加");
      return;
    }

    const currentDuration = Math.max(1, current.endTime - current.startTime);
    const defaultDuration = Math.min(5, currentDuration);
    const nextSubtitle = subtitles[index + 1];
    const availableEnd =
      nextSubtitle?.startTime > startTime + 0.1
        ? nextSubtitle.startTime
        : videoElement.duration;
    const endTime = Math.min(
      startTime + defaultDuration,
      availableEnd,
      videoElement.duration
    );

    subtitles = [
      ...subtitles.slice(0, index + 1),
      {
        startTime,
        endTime,
        text: "",
        layerOrder: nextSubtitleLayerOrder++,
      },
      ...subtitles.slice(index + 1),
    ];

    videoElement.currentTime = startTime;
    currentTime = startTime;
    currentSubtitleIndex = index + 1;
    selectedSubtitleIndex = index + 1;

    setTimeout(() => {
      subtitleRowElements[index + 1]
        ?.querySelector<HTMLInputElement>("input")
        ?.focus();
    });
  }

  function updateSubtitleTime(index: number, isStart: boolean, time: number) {
    if (!videoElement?.duration) return; // 防御性检查

    subtitles = subtitles.map((sub, i) => {
      if (i !== index) return sub;

      if (isStart) {
        // 开始时间约束：不能小于0，不能大于结束时间-0.1秒
        const newStartTime = Math.max(
          0,
          Math.min(time, sub.endTime - MIN_SUBTITLE_DURATION)
        );
        return { ...sub, startTime: newStartTime };
      } else {
        // 结束时间约束：不能小于开始时间+0.1秒，不能大于视频总长度
        const newEndTime = Math.min(
          videoElement.duration,
          Math.max(time, sub.startTime + MIN_SUBTITLE_DURATION)
        );
        return { ...sub, endTime: newEndTime };
      }
    });
  }

  // 整体移动字幕；alignEnd 时以结束时间为基准，保证吸附后与目标精确相等
  function moveSubtitle(index: number, time: number, alignEnd = false) {
    if (!videoElement?.duration) return;

    const sub = subtitles[index];
    const duration = sub.endTime - sub.startTime;

    let finalStartTime: number;
    let finalEndTime: number;
    if (alignEnd) {
      // 约束结束时间到有效范围 [duration, videoDuration]
      finalEndTime = Math.max(duration, Math.min(time, videoElement.duration));
      finalStartTime = finalEndTime - duration;
    } else {
      // 约束开始时间到有效范围 [0, videoDuration - duration]
      finalStartTime = Math.max(
        0,
        Math.min(time, videoElement.duration - duration)
      );
      finalEndTime = finalStartTime + duration;
    }

    subtitles = subtitles.map((s, i) =>
      i === index
        ? { ...s, startTime: finalStartTime, endTime: finalEndTime }
        : s
    );
  }

  async function removeSubtitle(index: number) {
    subtitles = subtitles.filter((_, i) => i !== index);
    if (selectedSubtitleIndex === index) {
      selectedSubtitleIndex = -1;
    } else if (selectedSubtitleIndex > index) {
      selectedSubtitleIndex--;
    }
    await saveSubtitles(); // 删除字幕时保存
  }

  async function clearSubtitles() {
    subtitles = [];
    selectedSubtitleIndex = -1;
    await saveSubtitles(); // 清空字幕时保存
  }

  function seekToTime(time: number) {
    videoElement.currentTime = time;
  }

  // 辅助函数：统一开始边缘拖拽
  function startEdgeDragging(index: number, isStart: boolean) {
    draggingSubtitle = { index, isStart };
    document.addEventListener("mousemove", handleTimelineMouseMove);
    document.addEventListener("mouseup", handleTimelineMouseUp);
  }

  // 辅助函数：统一开始块拖拽
  function startBlockDragging(
    index: number,
    mouseTime: number,
    startTime: number
  ) {
    draggingBlock = index;
    dragOffset = mouseTime - startTime;
    document.addEventListener("mousemove", handleBlockMouseMove);
    document.addEventListener("mouseup", handleBlockMouseUp);
  }

  function handleBlockMouseDown(e: MouseEvent, index: number) {
    selectSubtitle(index);

    const sub = subtitles[index];
    const rect = timelineElement.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const mouseTime = (x / rect.width) * videoElement.duration;

    // 计算边缘检测参数
    const blockWidth =
      rect.width * ((sub.endTime - sub.startTime) / videoElement.duration);
    const relativeX = x - rect.width * (sub.startTime / videoElement.duration);
    const edgeSize = Math.min(5, Math.max(2, blockWidth / 3));

    // 确定拖拽类型
    if (relativeX < edgeSize) {
      // 左边缘：调整开始时间
      startEdgeDragging(index, true);
    } else if (blockWidth > edgeSize * 2 && relativeX > blockWidth - edgeSize) {
      // 右边缘：调整结束时间（仅当有足够空间时）
      startEdgeDragging(index, false);
    } else if (blockWidth <= edgeSize * 2 && relativeX > edgeSize) {
      // 短字幕右侧：调整结束时间
      startEdgeDragging(index, false);
    } else {
      // 中间区域：移动整个字幕
      startBlockDragging(index, mouseTime, sub.startTime);
    }
  }

  // 拖动吸附：返回距 time 最近且在阈值内的吸附点（其它字幕的起止时间与播放头）
  function findSnapTime(
    time: number,
    excludeIndex: number,
    rect: DOMRect,
    accept: (target: number) => boolean = () => true
  ): number | null {
    const targets = [videoElement.currentTime];
    for (let i = 0; i < subtitles.length; i++) {
      if (i === excludeIndex) continue;
      targets.push(subtitles[i].startTime, subtitles[i].endTime);
    }

    const threshold = (SNAP_THRESHOLD_PX / rect.width) * videoElement.duration;
    let best: number | null = null;
    let bestDistance = threshold;
    for (const target of targets) {
      const distance = Math.abs(target - time);
      if (distance <= bestDistance && accept(target)) {
        best = target;
        bestDistance = distance;
      }
    }
    return best;
  }

  function handleTimelineMouseMove(e: MouseEvent) {
    if (!draggingSubtitle || !timelineElement) return;

    const rect = timelineElement.getBoundingClientRect();
    const x = e.clientX - rect.left;
    // 直接使用容器的实际宽度计算时间，与 getSubtitleStyle 逻辑保持一致
    const time = (x / rect.width) * videoElement.duration;

    // 按住 Shift 时暂时关闭吸附；只吸附到不会让字幕短于最小时长的位置
    const { index, isStart } = draggingSubtitle;
    const sub = subtitles[index];
    snapTime = e.shiftKey
      ? null
      : findSnapTime(time, index, rect, (target) =>
          isStart
            ? target <= sub.endTime - MIN_SUBTITLE_DURATION
            : target >= sub.startTime + MIN_SUBTITLE_DURATION
        );

    updateSubtitleTime(index, isStart, snapTime ?? time);
  }

  function handleBlockMouseMove(e: MouseEvent) {
    if (draggingBlock === null || !timelineElement) return;

    const rect = timelineElement.getBoundingClientRect();
    const x = e.clientX - rect.left;
    // 直接使用容器的实际宽度计算时间，与 getSubtitleStyle 逻辑保持一致
    const mouseTime = (x / rect.width) * videoElement.duration;
    const newStartTime = mouseTime - dragOffset; // 使用偏移量计算新的开始时间

    snapTime = null;
    if (!e.shiftKey) {
      // 开始和结束都可以吸附，取离吸附点更近的一端
      const sub = subtitles[draggingBlock];
      const newEndTime = newStartTime + (sub.endTime - sub.startTime);
      const startSnap = findSnapTime(newStartTime, draggingBlock, rect);
      const endSnap = findSnapTime(newEndTime, draggingBlock, rect);
      if (
        endSnap !== null &&
        (startSnap === null ||
          Math.abs(endSnap - newEndTime) < Math.abs(startSnap - newStartTime))
      ) {
        snapTime = endSnap;
        moveSubtitle(draggingBlock, endSnap, true);
        return;
      }
      snapTime = startSnap;
    }

    moveSubtitle(draggingBlock, snapTime ?? newStartTime);
  }

  function handleTimelineMouseUp() {
    draggingSubtitle = null;
    snapTime = null;
    sortSubtitles();
    document.removeEventListener("mousemove", handleTimelineMouseMove);
    document.removeEventListener("mouseup", handleTimelineMouseUp);
  }

  function handleBlockMouseUp() {
    draggingBlock = null;
    snapTime = null;
    sortSubtitles();
    document.removeEventListener("mousemove", handleBlockMouseMove);
    document.removeEventListener("mouseup", handleBlockMouseUp);
  }

  function getSubtitleStyle(subtitle: Subtitle, index: number) {
    if (!isVideoLoaded || !videoElement?.duration) return "";
    // 字幕块位置应该是相对于时间轴容器的百分比，不需要乘以缩放因子
    // 因为容器本身已经通过 style="width: {100 * timelineScale}%" 进行了缩放
    const start = (subtitle.startTime / videoElement.duration) * 100;
    const width =
      ((subtitle.endTime - subtitle.startTime) / videoElement.duration) * 100;
    const laneFromTop =
      subtitleLaneCount - 1 - (subtitleLanes[index] || 0);
    const top = 4 + laneFromTop * 36;
    return `left: ${start}%; width: ${width}%; top: ${top}px;`;
  }

  function handleVolumeChange(e: Event) {
    const input = e.target as HTMLInputElement;
    volume = parseFloat(input.value);
    if (videoElement) {
      videoElement.volume = volume;
    }
  }

  function handleCoverError(event: Event) {
    console.error("Cover image load failed:", event);
    showDefaultCoverIcon = true;
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

  function handleScaleChange(e: Event) {
    const input = e.target as HTMLInputElement;
    timelineScale = parseFloat(input.value);
    requestAnimationFrame(() => {
      if (!timelineElement) return;
      timelineWidth = timelineElement.getBoundingClientRect().width;
      updateTimeMarkers();
    });
  }

  function handleWheel(e: WheelEvent) {
    e.preventDefault();
    if (timelineContainer) {
      timelineContainer.scrollLeft += e.deltaY;
    }
  }

  async function encodeVideoSubtitle() {
    if (submittingEncode) return;
    submittingEncode = true;
    // Capture the selected video/document before any await or further editing.
    const id = video.id;
    const assContent = subtitleAss;
    const eventId = generateEventId();
    try {
      await saveSubtitles();
      announceTask({
        id: eventId,
        task_type: "encode_video_subtitle",
        metadata: JSON.stringify({ video_id: id }),
      });
      openTaskCenter(eventId);
      await invoke("encode_video_subtitle", {
        eventId,
        id,
        srtStyle: "",
        assContent,
      });
    } catch (error) {
      alert("加入压制队列失败：" + error);
    } finally {
      settleTask(eventId);
      submittingEncode = false;
    }
  }

  function handleVideoSelect(e: Event) {
    const selectedVideo = videos.find(
      (v) => v.id === Number((e.target as HTMLSelectElement).value)
    );
    if (selectedVideo) {
      // 清空字幕列表
      subtitles = [];
      currentSubtitleIndex = -1;
      selectedSubtitleIndex = -1;
      stopRangePlayback?.();
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
  // 当视频改变时重置封面错误状态
  $effect(() => {
    if (video) {
      showDefaultCoverIcon = false;
    }
  });
  $effect(() => {
    if (show) {
      isVideoLoaded = false;
      subtitles = []; // 清空字幕列表
      currentSubtitleIndex = -1;
      selectedSubtitleIndex = -1;
      subtitleRowElements = [];
      subtitleBlockElements = [];
      loadSubtitleStyle(); // 加载字幕样式

      // 销毁旧的波形图实例
      destroyWaveSurfer();
    }
  });
  $effect(() => {
    const laneLayout = calculateSubtitleLanes(subtitles);
    subtitleLanes = laneLayout.lanes;
    subtitleLaneCount = laneLayout.count;
    subtitleTrackHeight = Math.max(48, subtitleLaneCount * 36 + 8);
    timelineTotalHeight = 108 + subtitleTrackHeight;
  });
  $effect(() => {
    const activeSubtitleIndices = subtitles
      .map((subtitle, index) => ({ subtitle, index }))
      .filter(
        ({ subtitle }) =>
          currentTime >= subtitle.startTime && currentTime < subtitle.endTime
      )
      .map(({ index }) => index);
    currentSubtitleIndices = activeSubtitleIndices;
    currentSubtitleIndex = activeSubtitleIndices[0] ?? -1;
  });
  $effect(() => {
    window.localStorage.setItem("profile-" + roomId, JSON.stringify(profile));
  });
  // 监听当前字幕索引变化
  $effect(() => {
    if (currentSubtitleIndex >= 0 && subtitleRowElements[currentSubtitleIndex]) {
      subtitleRowElements[currentSubtitleIndex].scrollIntoView({
        behavior: "smooth",
        block: "nearest",
      });
    }
  });
  // 当视频改变时重新初始化切片时间（只在视频ID改变时触发）
  $effect(() => {
    if (video && videoElement?.duration && video.id !== lastVideoId) {
      lastVideoId = video.id;
      // 切换视频时重置切片时间 - 不设置默认值，等待用户输入
      clipStartTime = 0;
      clipEndTime = 0;
      clipTitle = "";
      clipTimesSet = false; // 重置标记，新视频默认透明
    }
  });
  // 监听样式编辑器关闭，重新加载样式
  $effect(() => {
    if (!showStyleEditor) {
      loadSubtitleStyle();
    }
  });

  // The encoder scales and pads to 1920×1080 before applying ASS. Preview the
  // same complete output frame, including the bars on portrait/ultrawide input.
  $effect(() => {
    const container = previewContainer;
    if (!container) return;
    const resize = () => {
      previewFrameWidth = Math.min(container.clientWidth, container.clientHeight * 16 / 9);
    };
    const observer = new ResizeObserver(resize);
    observer.observe(container);
    resize();
    return () => observer.disconnect();
  });
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
        <!-- 视频选择器 -->
        <div class="relative flex items-center space-x-2">
          <select
            class="bg-[#1c1c1e] text-gray-300 text-sm rounded-md px-3 py-1.5 border border-gray-700 focus:border-[#0A84FF] outline-none appearance-none cursor-pointer hover:bg-[#2c2c2e] transition-colors duration-200"
            value={video.id}
            onchange={handleVideoSelect}
          >
            {#each videos as v}
              <option value={v.id}>{v.name}</option>
            {/each}
          </select>
          <!-- 保存按钮 -->
          {#if !TAURI_ENV}
            <button
              class="text-blue-500 hover:text-blue-400 transition-colors duration-200 px-2 py-1.5 rounded-md hover:bg-blue-500/10"
              onclick={saveVideo}
            >
              <Download class="w-4 h-4" />
            </button>
          {/if}
          <!-- 删除按钮 -->
          <button
            class="text-red-500 hover:text-red-400 transition-colors duration-200 px-2 py-1.5 rounded-md hover:bg-red-500/10"
            onclick={async () => {
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
                  // 如果列表为空，关闭窗口
                  close_window();
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
              aria-label="关闭压制确认"
              onclick={() => (showEncodeModal = false)}
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
            将当前字幕和样式加入压制队列，按顺序执行。提交后可继续编辑或添加其他压制任务，进度和取消都在任务列表中。每个任务完成后都会生成独立的带字幕切片。
          </div>

          <!-- Modal 底部按钮 -->
          <div
            class="px-4 py-3 border-t border-gray-800/50 flex justify-end space-x-2"
          >
            <button
              class="px-4 py-1.5 text-sm bg-gray-700/50 text-gray-200 rounded-md hover:bg-gray-700/70 transition-colors duration-200 border border-gray-600/50"
              onclick={() => (showEncodeModal = false)}
            >
              取消
            </button>
            <button
              class="px-4 py-1.5 text-sm bg-[#0A84FF] text-white rounded-md hover:bg-[#0A84FF]/90 transition-colors duration-200 disabled:opacity-50 disabled:cursor-not-allowed flex items-center space-x-1"
              onclick={() => {
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

    <div class="relative flex h-[calc(100vh-3.5rem)] overflow-hidden">
      <!-- 视频区域 -->
      <div class="flex-1 flex flex-col min-w-0">
        <!-- 切片控制信息条 -->
        {#if canBeClipped(video)}
          <div
            class="bg-black px-4 py-2 flex items-center justify-between text-sm"
          >
            <div class="flex items-center space-x-6">
              <div class="text-gray-300">
                切片起点: <span class="text-[#0A84FF] font-mono"
                  >{formatTime(clipStartTime)}</span
                >
              </div>
              <div class="text-gray-300">
                切片终点: <span class="text-[#0A84FF] font-mono"
                  >{formatTime(clipEndTime)}</span
                >
              </div>
              <div class="text-gray-300">
                时长: <span class="text-white font-mono"
                  >{formatTime(clipEndTime - clipStartTime)}</span
                >
              </div>
            </div>
          </div>
        {/if}
        <!-- 视频容器 -->
        <div class="flex-1 bg-black relative">
          <div bind:this={previewContainer} class="absolute inset-0 flex items-center justify-center">
            <div class="relative aspect-video" style:width="{previewFrameWidth}px">
            <!-- svelte-ignore a11y_media_has_caption -->
            <video
              bind:this={videoElement}
              src={video?.file}
              class="absolute inset-0 w-full h-full object-contain cursor-pointer"
              ontimeupdate={handleTimeUpdate}
              onended={handleVideoEnded}
              onloadedmetadata={handleVideoLoaded}
              onclick={togglePlay}
></video>

            {#if isVideoLoaded && videoElement}
              <AssSubtitlePreview assContent={subtitleAss} video={videoElement} fontName={subtitleStyle.fontName} />
            {/if}
            </div>
          </div>
        </div>

        <!-- 字幕控制栏 -->
        <div class="bg-[#1c1c1e] border-t border-gray-800/50 p-2">
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
                oninput={handleScaleChange}
                class="w-32 h-1 bg-gray-600 rounded-lg appearance-none cursor-pointer"
              />
            </div>

            <!-- 中间播放控制 -->
            <div class="flex-1 flex justify-center">
              <button
                class="p-1.5 rounded-lg bg-[#2c2c2e] hover:bg-[#3c3c3e] transition-colors duration-200"
                onclick={togglePlay}
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
                onclick={toggleMute}
              >
                {#if isMuted || volume === 0}
                  <svg
                    xmlns="http://www.w3.org/2000/svg"
                    class="h-4 w-4"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
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
                oninput={handleVolumeChange}
                class="w-20 h-1 bg-gray-600 rounded-lg appearance-none cursor-pointer"
              />
              <span class="h-4 w-px bg-gray-700/70"></span>
              <button
                class="text-gray-400 hover:text-white transition-colors duration-200"
                title="快捷键说明"
                aria-label="快捷键说明"
                onclick={() => (showShortcutHelp = true)}
              >
                <CircleHelp class="w-4 h-4" />
              </button>
            </div>
          </div>
        </div>

        <!-- 时间轴：字幕轨与音视频轨共用同一时间坐标 -->
        <div class="bg-[#1c1c1e] border-t border-gray-800/50">
          <div
            class="flex overflow-hidden"
            style="height: {timelineTotalHeight}px"
          >
            <div
              class="w-20 shrink-0 border-r border-gray-800/70 bg-[#242426] text-xs text-gray-400 select-none flex flex-col"
            >
              <div
                class="order-0 h-7 px-3 flex items-center border-b border-gray-800/70 text-[10px] uppercase tracking-wider"
              >时间</div>
              <div class="order-2 h-20 px-3 flex items-center gap-2">
                <span class="w-2 h-2 rounded-sm bg-violet-500"></span>
                视频
              </div>
              <div
                class="order-1 px-3 flex items-center gap-2 border-b border-gray-800/70"
                style="height: {subtitleTrackHeight}px"
              >
                <span class="w-2 h-2 rounded-sm bg-[#0A84FF]"></span>
                字幕
              </div>
            </div>

            <div
              class="flex-1 overflow-x-auto overflow-y-hidden sidebar-scrollbar"
              bind:this={timelineContainer}
              onwheel={(event) => {
                event.preventDefault();
                handleWheel(event);
              }}
            >
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <div
                bind:this={timelineElement}
                role="button"
                tabindex="0"
                aria-label="视频时间轴"
                class="relative min-w-full group select-none flex flex-col"
                style="width: {100 * timelineScale}%; height: {timelineTotalHeight}px"
                onmousemove={() => {
                  if (!timelineElement) return;
                  timelineWidth = timelineElement.getBoundingClientRect().width;
                  updateTimeMarkers();
                }}
                onclick={(e) => {
                  e.stopPropagation();
                  e.preventDefault();
                  if (!isDraggingSeekbar) handleTimelineClick(e);
                }}
              >
                <div
                  bind:this={seekbarElement}
                  role="slider"
                  tabindex="0"
                  aria-valuemin="0"
                  aria-valuemax={videoElement?.duration || 0}
                  aria-valuenow={isDraggingSeekbar ? previewTime : currentTime}
                  class="order-0 shrink-0 relative h-7 border-b border-gray-800/70 bg-[#202022] cursor-pointer"
                  class:dragging={isDraggingSeekbar}
                  onmousedown={handleSeekbarMouseDown}
                >
                  <div
                    class="absolute bottom-0 left-0 h-0.5 bg-[#0A84FF] pointer-events-none"
                    class:no-transition={isDraggingSeekbar}
                    style="width: {((isDraggingSeekbar
                      ? previewTime
                      : currentTime) /
                      (videoElement?.duration || 1)) *
                      100}%"
                  ></div>
                  {#each timeMarkers as time}
                    <div
                      class="absolute inset-y-0 border-l border-gray-600/70"
                      style="left: {(time / (videoElement?.duration || 1)) * 100}%"
                    >
                      <span
                        class="absolute top-1 left-1 text-[10px] text-gray-400 whitespace-nowrap"
                        >{formatTime(time)}</span
                      >
                    </div>
                  {/each}
                </div>

                <div
                  class="order-2 shrink-0 relative h-20 bg-violet-950/10 overflow-hidden"
                >
                  <div
                    class="absolute inset-x-1 inset-y-1 rounded-md border border-violet-500/50 bg-violet-500/15 overflow-hidden"
                  >
                    <div
                      class="h-6 px-3 flex items-center gap-2 text-xs text-violet-100 bg-violet-500/20"
                    >
                      <Film class="w-3.5 h-3.5 shrink-0 text-violet-300" />
                      <span class="truncate">{getVideoDisplayName()}</span>
                      <span
                        class="ml-auto text-[10px] text-violet-200/70 font-mono"
                        >{formatTime(videoElement?.duration || 0)}</span
                      >
                    </div>
                    {#if isWaveformLoading}
                      <div
                        class="absolute inset-x-0 top-6 bottom-0 z-10 flex items-center justify-center gap-2 text-xs text-gray-400 bg-[#1c1c1e]/70"
                      >
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
                        <span>加载音频波形...</span>
                      </div>
                    {/if}
                    <div
                      bind:this={waveformContainer}
                      class="absolute inset-x-0 top-6 bottom-0 waveform-container"
                      data-waveform-container
                    ></div>
                    {#if canBeClipped(video) && clipTimesSet}
                      <div
                        class="absolute inset-y-0 z-10 border-x-2 border-green-400 bg-green-400/20 pointer-events-none"
                        style="left: {(clipStartTime /
                          (videoElement?.duration || 1)) *
                          100}%; right: {100 -
                          (clipEndTime / (videoElement?.duration || 1)) * 100}%"
                      ></div>
                    {/if}
                  </div>
                </div>

                <div
                  class="order-1 shrink-0 relative border-b border-gray-800/70 bg-blue-950/10 overflow-hidden"
                  style="height: {subtitleTrackHeight}px"
                >
                  {#if subtitles.length === 0}
                    <div
                      class="absolute inset-0 flex items-center justify-center text-xs text-gray-600 pointer-events-none"
                    >
                      暂无字幕，可使用 AI 生成
                    </div>
                  {/if}
                  {#each subtitles as subtitle, index}
                    <div
                      bind:this={subtitleBlockElements[index]}
                      role="button"
                      tabindex="0"
                      class="absolute h-8 rounded-md border border-[#0A84FF]/50 bg-[#0A84FF]/20 cursor-move overflow-hidden {currentSubtitleIndices.includes(
                        index
                      ) || draggingSubtitle?.index === index || draggingBlock === index || selectedSubtitleIndex === index
                        ? 'z-20 border-[#0A84FF]/90 bg-[#0A84FF]/30'
                        : 'z-10'} {selectedSubtitleIndex === index
                        ? 'ring-1 ring-white/80'
                        : ''}"
                      style={getSubtitleStyle(subtitle, index)}
                      onmousedown={(e) => handleBlockMouseDown(e, index)}
                      onclick={(event) => event.stopPropagation()}
                    >
                      <div
                        class="h-full px-1.5 flex items-center text-[10px] text-blue-100"
                      >
                        <span class="truncate">{subtitle.text || "空字幕"}</span>
                      </div>
                    </div>
                  {/each}
                </div>

                {#each timeMarkers as time}
                  <div
                    class="absolute top-7 bottom-0 border-l border-gray-700/40 pointer-events-none"
                    style="left: {(time / (videoElement?.duration || 1)) * 100}%"
                  ></div>
                {/each}

                {#if canBeClipped(video) && clipTimesSet}
                  <div
                    class="absolute top-7 bottom-0 w-px bg-green-400/80 pointer-events-none z-20"
                    style="left: {(clipStartTime /
                      (videoElement?.duration || 1)) *
                      100}%"
                  ></div>
                  <div
                    class="absolute top-7 bottom-0 w-px bg-green-400/80 pointer-events-none z-20"
                    style="left: {(clipEndTime /
                      (videoElement?.duration || 1)) *
                      100}%"
                  ></div>
                {/if}

                <!-- 拖动字幕时的吸附位置 -->
                {#if snapTime !== null}
                  <div
                    class="absolute top-7 bottom-0 w-px bg-amber-300 pointer-events-none z-30"
                    style="left: {(snapTime / (videoElement?.duration || 1)) *
                      100}%"
                  ></div>
                {/if}

                <div
                  class="absolute top-0 bottom-0 w-px bg-[#0A84FF] pointer-events-none z-30"
                  style="left: {((isDraggingSeekbar
                    ? previewTime
                    : currentTime) /
                    (videoElement?.duration || 1)) *
                    100}%"
                >
                  <div
                    class="absolute top-0 left-1/2 -translate-x-1/2 w-2.5 h-2.5 bg-[#0A84FF] rounded-b-sm"
                  ></div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      <InspectorPanel
        title="切片编辑"
        tabs={[
          { id: "subtitle", label: "字幕", count: subtitles.length },
          { id: "upload", label: "投稿" },
        ]}
        bind:activeTab
        bind:collapsed={inspectorCollapsed}
        summary={canBeClipped(video)
          ? { label: "切片范围", value: clipEndTime > clipStartTime ? `${formatTime(clipStartTime)} – ${formatTime(clipEndTime)}` : "未选择" }
          : { label: "字幕", value: `${subtitles.length} 条` }}
      >
        {#snippet actions()}
          {#if canBeClipped(video)}
            <button
              type="button"
              class="footer-button"
              title="按当前切片范围生成新切片（G）"
              onclick={generateClip}
            >
              <Scissors class="h-3.5 w-3.5" />
              生成切片
            </button>
          {/if}
          <button
            type="button"
            class="footer-button primary"
            disabled={submittingEncode || subtitles.length === 0}
            title={subtitles.length === 0 ? "字幕列表为空，没有可压制的字幕" : undefined}
            onclick={() => (showEncodeModal = true)}
          >
            {#if submittingEncode}
              <Loader2 class="h-3.5 w-3.5 animate-spin" />
            {:else}
              <Film class="h-3.5 w-3.5" />
            {/if}
            压制字幕
          </button>
        {/snippet}

        {#if activeTab === "subtitle"}
          <div class="subtitle-toolbar">
            <button type="button" class="tool-button" onclick={() => (showStyleEditor = true)}>
              <Settings class="h-3.5 w-3.5" />
              字幕样式
            </button>
            <button type="button" class="tool-button" onclick={clearSubtitles}>
              <Eraser class="h-3.5 w-3.5" />
              清空列表
            </button>
            <button
              type="button"
              class="tool-button"
              onclick={generateSubtitles}
              disabled={current_generate_event_id !== null}
            >
              {#if current_generate_event_id !== null}
                <Loader2 class="h-3.5 w-3.5 animate-spin" />
                生成中
              {:else}
                <BrainCircuit class="h-3.5 w-3.5" />
                AI 字幕
              {/if}
            </button>
          </div>

          {#if subtitles.length > 0}
            <div class="subtitle-list">
              {#each subtitles as subtitle, index}
                <div
                  bind:this={subtitleRowElements[index]}
                  class="subtitle-row"
                  class:playing={currentSubtitleIndices.includes(index)}
                  class:selected={selectedSubtitleIndex === index}
                >
                  <button
                    type="button"
                    class="subtitle-index"
                    title="选中并跳转到字幕开始"
                    onclick={() => selectSubtitle(index, true)}
                  >
                    {index + 1}
                  </button>
                  <input
                    type="text"
                    bind:value={subtitle.text}
                    onfocus={() => selectSubtitle(index)}
                    onkeydown={handleSubtitleInputKeydown}
                    class="subtitle-text"
                    placeholder="输入字幕文本"
                  />
                  <button
                    type="button"
                    class="subtitle-action"
                    title="在后面添加字幕"
                    onclick={() => insertSubtitleAfter(index)}
                  >
                    <Plus class="h-3.5 w-3.5" />
                  </button>
                  <button
                    type="button"
                    class="subtitle-action danger"
                    title="删除字幕"
                    onclick={async () => await removeSubtitle(index)}
                  >
                    <Trash2 class="h-3.5 w-3.5" />
                  </button>
                </div>
              {/each}
            </div>
          {:else}
            <div class="empty-state">
              <strong>还没有字幕</strong>
              <span>点击上方「AI 字幕」自动生成</span>
            </div>
          {/if}
        {:else if activeTab === "upload"}
          <div class="upload-form">
            {#if video && video.id != -1}
              <section>
                <div class="section-heading">
                  <h3>视频封面</h3>
                  <button type="button" class="link-button" onclick={() => (show_cover_editor = true)}>
                    <Pen class="h-3 w-3" />
                    创建新封面
                  </button>
                </div>
                <div class="cover-frame">
                  {#if video.cover && video.cover.trim() !== ""}
                    <img
                      src={video.cover}
                      alt="视频封面"
                      onerror={handleCoverError}
                      style:display={showDefaultCoverIcon ? "none" : "block"}
                    />
                  {/if}
                  {#if !video.cover || video.cover.trim() === "" || showDefaultCoverIcon}
                    <div class="cover-placeholder">暂无封面</div>
                  {/if}
                </div>
              </section>
            {/if}

            <section>
              <div class="section-heading"><h3>基本信息</h3></div>
              <div class="field">
                <label for="title">标题</label>
                <input
                  id="title"
                  type="text"
                  bind:value={profile.title}
                  placeholder="输入视频标题"
                  class="panel-input"
                />
              </div>
              <div class="field">
                <label for="tid">视频分区</label>
                <div class="w-full" id="tid">
                  <TypeSelect bind:value={profile.tid} />
                </div>
              </div>
              <div class="field">
                <label for="uid">投稿账号</label>
                <select id="uid" bind:value={uid_selected} class="panel-input">
                  <option value={0}>选择账号</option>
                  {#each accounts as account}
                    <option value={account.value}>{account.name}</option>
                  {/each}
                </select>
              </div>
            </section>

            <section>
              <div class="section-heading"><h3>详细信息</h3></div>
              <div class="field">
                <label for="desc">描述</label>
                <textarea
                  id="desc"
                  bind:value={profile.desc}
                  placeholder="输入视频描述"
                  class="panel-input h-20 resize-none"
                ></textarea>
              </div>
              <div class="field">
                <label for="tag">标签</label>
                <input
                  id="tag"
                  type="text"
                  bind:value={profile.tag}
                  placeholder="输入视频标签，用逗号分隔"
                  class="panel-input"
                />
              </div>
              <div class="field">
                <label for="dynamic">动态</label>
                <textarea
                  id="dynamic"
                  bind:value={profile.dynamic}
                  placeholder="输入动态内容"
                  class="panel-input h-20 resize-none"
                ></textarea>
              </div>
            </section>

            {#if video}
              <button
                type="button"
                class="footer-button primary w-full justify-center"
                onclick={do_post}
                disabled={current_post_event_id != null || !uid_selected}
              >
                {#if current_post_event_id != null}
                  <Loader2 class="h-3.5 w-3.5 animate-spin" />
                  投稿中
                {:else}
                  投稿
                {/if}
              </button>
            {/if}
          </div>
        {/if}
      </InspectorPanel>
    </div>
  </div>
{/if}

<SubtitleStyleEditor
  bind:show={showStyleEditor}
  {roomId}
  onSave={(style) => (subtitleStyle = style)}
  onClose={() => (showStyleEditor = false)}
/>

<ShortcutHelpModal
  bind:show={showShortcutHelp}
  showClipShortcuts={canBeClipped(video)}
/>

<CoverEditor
  bind:show={show_cover_editor}
  {video}
  onCoverUpdate={(event) => {
    video = {
      ...video,
      cover: event.cover,
    };
  }}
/>

<!-- 键盘快捷键监听 -->
<svelte:window onkeydown={handleKeydown} />

<style>
  /* Inspector panel content; matches ArchiveInspector's palette. */
  .footer-button {
    display: inline-flex;
    height: 30px;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
    border: 1px solid #344052;
    border-radius: 8px;
    background: #202733;
    padding: 0 11px;
    color: #dce5f3;
    font-size: 12px;
    transition: background-color 150ms ease, border-color 150ms ease;
  }

  .footer-button:hover:not(:disabled) {
    border-color: #4a5a70;
  }

  .footer-button.primary {
    border-color: transparent;
    background: #0a84ff;
    color: white;
  }

  .footer-button.primary:hover:not(:disabled) {
    background: rgb(10 132 255 / 90%);
  }

  .footer-button:disabled,
  .tool-button:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .subtitle-toolbar {
    position: sticky;
    top: 0;
    z-index: 1;
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 6px;
    margin: 0 -16px;
    background: #171c25;
    padding: 0 16px 10px;
  }

  .tool-button {
    display: inline-flex;
    height: 30px;
    align-items: center;
    justify-content: center;
    gap: 5px;
    white-space: nowrap;
    border: 1px solid #293442;
    border-radius: 8px;
    background: #202733;
    color: #c4cfdd;
    font-size: 11px;
  }

  .tool-button:hover:not(:disabled) {
    border-color: #3a4658;
    color: #edf4ff;
  }

  .subtitle-list {
    overflow: hidden;
    border: 1px solid #293442;
    border-radius: 10px;
    background: #202733;
  }

  .subtitle-row {
    display: flex;
    min-height: 38px;
    align-items: center;
    gap: 4px;
    border-bottom: 1px solid #293442;
    border-left: 2px solid transparent;
    padding: 0 6px 0 4px;
  }

  .subtitle-row:last-child {
    border-bottom: 0;
  }

  .subtitle-row:hover {
    background: rgb(255 255 255 / 3%);
  }

  .subtitle-row.playing {
    border-left-color: #35a7ff;
    background: rgb(53 167 255 / 10%);
  }

  .subtitle-row.selected {
    box-shadow: inset 0 0 0 1px rgb(53 167 255 / 60%);
  }

  .subtitle-index {
    width: 22px;
    flex-shrink: 0;
    color: #6f7b8d;
    font-family: ui-monospace, monospace;
    font-size: 10px;
    text-align: right;
  }

  .subtitle-index:hover,
  .subtitle-row.selected .subtitle-index {
    color: #55b9ff;
  }

  .subtitle-text {
    min-width: 0;
    flex: 1;
    border: 1px solid transparent;
    border-radius: 6px;
    background: transparent;
    padding: 3px 5px;
    color: #dce5f3;
    font-size: 12px;
    outline: none;
  }

  .subtitle-text:focus {
    border-color: rgb(10 132 255 / 70%);
    background: #0f131a;
  }

  .subtitle-action {
    display: inline-flex;
    flex-shrink: 0;
    padding: 4px;
    color: #6f7b8d;
    opacity: 0.7;
  }

  .subtitle-action:hover {
    color: #55b9ff;
    opacity: 1;
  }

  .subtitle-action.danger:hover {
    color: #ff6b6b;
  }

  .empty-state {
    display: flex;
    min-height: 120px;
    align-items: center;
    justify-content: center;
    flex-direction: column;
    gap: 5px;
    color: #718095;
    font-size: 10px;
    text-align: center;
  }

  .empty-state strong {
    color: #a8b3c3;
    font-size: 12px;
    font-weight: 500;
  }

  .upload-form {
    display: flex;
    flex-direction: column;
    gap: 18px;
  }

  .upload-form section {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .section-heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .section-heading h3 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
  }

  .link-button {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: #55b9ff;
    font-size: 10px;
    font-weight: 500;
  }

  .cover-frame {
    overflow: hidden;
    border: 1px solid #293442;
    border-radius: 9px;
    background: #0c1016;
  }

  .cover-frame img {
    width: 100%;
  }

  .cover-placeholder {
    display: flex;
    aspect-ratio: 16 / 9;
    align-items: center;
    justify-content: center;
    color: #657186;
    font-size: 10px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .field label {
    color: #9aa6b8;
    font-size: 10px;
  }

  .panel-input {
    width: 100%;
    border: 1px solid #293442;
    border-radius: 8px;
    background: #0f131a;
    padding: 7px 10px;
    color: #e9eef8;
    font-size: 12px;
    outline: none;
    transition: border-color 150ms ease;
  }

  .panel-input:hover {
    border-color: #3a4658;
  }

  .panel-input:focus {
    border-color: #0a84ff;
  }

  .panel-input::placeholder {
    color: #5d6878;
  }

  /* 拖动时禁用过渡动画，避免与JS更新冲突 */
  .no-transition {
    transition: none !important;
  }

  /* 普通range输入框样式（不影响进度条） */
  input[type="range"]:not(.progress-bar) {
    -webkit-appearance: none;
    appearance: none;
  }

  input[type="range"]:not(.progress-bar)::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: white;
    border: none;
    cursor: pointer;
  }

  input[type="range"]:not(.progress-bar)::-webkit-slider-track {
    height: 4px;
    border-radius: 2px;
    background: #4a5568;
  }

  input[type="range"]:not(.progress-bar)::-moz-range-thumb {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: white;
    border: none;
    cursor: pointer;
  }

  input[type="range"]:not(.progress-bar)::-moz-range-track {
    height: 4px;
    border-radius: 2px;
    background: #4a5568;
  }

  /* WaveSurfer.js 样式 */
  .waveform-container {
    background: transparent;
    border-radius: inherit;
    width: 100%;
    height: 100%;
  }
</style>
