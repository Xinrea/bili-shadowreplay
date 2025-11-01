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
    Pen,
    Scissors,
  } from "lucide-svelte";
  import {
    generateEventId,
    parseSubtitleStyle,
    type ProgressFinished,
    type ProgressUpdate,
    type SubtitleStyle,
    type VideoItem,
    type Profile,
    type Config,
    default_profile,
  } from "../interface";
  import SubtitleStyleEditor from "./SubtitleStyleEditor.svelte";
  import CoverEditor from "./CoverEditor.svelte";
  import TypeSelect from "./TypeSelect.svelte";
  import {
    invoke,
    TAURI_ENV,
    listen,
    log,
    close_window,
    get_cover,
  } from "../invoker";
  import { onDestroy, onMount } from "svelte";
  import { listen as tauriListen } from "@tauri-apps/api/event";
  import type { AccountInfo } from "../db";
  import WaveSurfer from "wavesurfer.js";

  export let show = false;
  export let video: VideoItem;
  export let roomId: string;
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
  let showDefaultCoverIcon = false;
  let timelineWidth = 0;

  // 当视频改变时重置封面错误状态
  $: if (video) {
    showDefaultCoverIcon = false;
  }
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
  let windowCloseUnlisten: (() => void) | null = null;
  let activeTab = "subtitle"; // 添加当前激活的 tab

  // 切片功能相关变量
  let clipStartTime = 0;
  let clipEndTime = 0;
  let clipTitle = "";
  let clipping = false;
  let current_clip_event_id = null;
  let show_detail = false; // 控制快捷键说明的展开
  let lastVideoId = -1; // 记录上一个视频ID，避免重复初始化
  let clipTimesSet = false; // 标记用户是否主动设置过切片时间

  // 进度条拖动相关变量
  let isDraggingSeekbar = false;
  let seekbarElement: HTMLElement;
  let previewTime = 0; // 拖动时预览的时间
  let wasPlayingBeforeDrag = false; // 拖动前的播放状态

  // 投稿相关变量
  let current_post_event_id = null;
  let config: Config = null;
  let accounts: any[] = [];
  let uid_selected = 0;
  let show_cover_editor = false;

  // WaveSurfer.js 相关变量
  let wavesurfer: any = null;
  let waveformContainer: HTMLElement;
  let isWaveformLoaded = false;
  let isWaveformLoading = false;
  let syncTimeout: ReturnType<typeof setTimeout> | null = null;

  // 获取 profile 从 localStorage
  function get_profile(): Profile {
    const profile_str = window.localStorage.getItem("profile-" + roomId);
    if (profile_str && profile_str.includes("videos")) {
      return JSON.parse(profile_str);
    }
    return default_profile();
  }

  let profile: Profile = get_profile();

  $: {
    window.localStorage.setItem("profile-" + roomId, JSON.stringify(profile));
  }

  // 初始化 WaveSurfer.js
  async function initWaveSurfer() {
    if (typeof window === "undefined" || !video?.file) return;

    isWaveformLoading = true;

    try {
      createWaveSurfer();
    } catch (error) {
      console.error("Failed to initialize WaveSurfer.js:", error);
      isWaveformLoading = false;
    }
  }

  function createWaveSurfer() {
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

    // 确保容器有正确的尺寸，考虑 timeline scale
    container.style.width = `${100 * timelineScale}%`;
    container.style.height = "60px";
    container.style.minHeight = "60px";
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
        waveColor: "#4a5568",
        progressColor: "#0A84FF",
        cursorColor: "#0A84FF",
        barWidth: 2,
        barRadius: 1,
        height: 60,
        normalize: true,
        interact: true, // 启用交互，允许点击切换进度
        plugins: [],
      });

      console.log("WaveSurfer created, loading file:", video.file);
      // 加载音频
      wavesurfer.load(video.file);

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
    // 清理 WaveSurfer 实例
    destroyWaveSurfer();
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

  function update_post_prompt(str: string) {
    const span = document.getElementById("post-prompt");
    if (span) {
      span.textContent = str;
    }
  }

  // 投稿相关函数
  async function do_post() {
    if (!video) {
      return;
    }

    let event_id = generateEventId();
    current_post_event_id = event_id;

    update_post_prompt(`投稿上传中`);

    const clear_update_listener = await listen(
      `progress-update:${event_id}`,
      (e) => {
        update_post_prompt(e.payload.content);
      }
    );
    const clear_finished_listener = await listen(
      `progress-finished:${event_id}`,
      (e) => {
        update_post_prompt(`投稿`);
        if (!e.payload.success) {
          alert(e.payload.message);
        }

        current_post_event_id = null;

        clear_update_listener();
        clear_finished_listener();
      }
    );

    // update profile in local storage
    window.localStorage.setItem("profile-" + roomId, JSON.stringify(profile));
    invoke("upload_procedure", {
      uid: uid_selected,
      eventId: event_id,
      roomId: roomId,
      videoId: video.id,
      cover: video.cover,
      profile: profile,
    }).then(async () => {
      uid_selected = 0;
      await onVideoListUpdate?.();
    });
  }

  async function cancel_post() {
    if (!current_post_event_id) {
      return;
    }
    invoke("cancel", { eventId: current_post_event_id });
  }

  function pauseVideo() {
    if (videoElement) {
      videoElement.pause();
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
      const clear_update_listener = await listen(
        `progress-update:${current_generate_event_id}`,
        (e) => {
          update_generate_prompt(e.payload.content);
        }
      );
      const clear_finished_listener = await listen(
        `progress-finished:${current_generate_event_id}`,
        (e) => {
          update_generate_prompt(`AI 生成字幕`);
          if (!e.payload.success) {
            alert("生成字幕失败: " + e.payload.message);
          }

          current_generate_event_id = null;

          clear_update_listener();
          clear_finished_listener();
        }
      );
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

    // 销毁旧的波形图实例
    destroyWaveSurfer();
  }

  // 当视频改变时重新初始化切片时间（只在视频ID改变时触发）
  $: if (video && videoElement?.duration && video.id !== lastVideoId) {
    lastVideoId = video.id;
    // 切换视频时重置切片时间 - 不设置默认值，等待用户输入
    clipStartTime = 0;
    clipEndTime = 0;
    clipTitle = "";
    clipTimesSet = false; // 重置标记，新视频默认透明
  }

  // 监听样式编辑器关闭，重新加载样式
  $: if (!showStyleEditor) {
    loadSubtitleStyle();
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
    initClipTimes(); // 初始化切片时间

    // 初始化波形图
    setTimeout(() => {
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
    const maxMarkers = Math.floor(timelineWidth / minMarkerWidth);
    const interval = Math.ceil(duration / maxMarkers);

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

    // 如果没有设置切片标题，则以当前本地时间戳命名
    if (!clipTitle.trim()) {
      const now = new Date();
      const pad = (n) => n.toString().padStart(2, "0");
      const timestamp = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}_${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
      clipTitle = `clip_${timestamp}`;
    }

    if (clipStartTime >= clipEndTime) {
      alert("开始时间必须小于结束时间");
      return;
    }

    if (clipEndTime - clipStartTime < 1) {
      alert("切片长度不能少于1秒");
      return;
    }

    clipping = true;
    current_clip_event_id = generateEventId();
    const clear_update_listener = await listen(
      `progress-update:${current_clip_event_id}`,
      (e) => {
        update_clip_prompt(e.payload.content);
      }
    );
    const clear_finished_listener = await listen(
      `progress-finished:${current_clip_event_id}`,
      (e) => {
        update_clip_prompt(`生成切片`);
        if (e.payload.success) {
          // 切片生成成功，刷新视频列表
          if (onVideoListUpdate) {
            onVideoListUpdate();
          }
          // 重置切片设置
          clipStartTime = 0;
          clipEndTime = 0;
          clipTitle = "";
          clipTimesSet = false; // 重置标记
        } else {
          alert("切片生成失败: " + e.payload.message);
        }
        clipping = false;

        current_clip_event_id = null;

        clear_update_listener();
        clear_finished_listener();
      }
    );

    try {
      await invoke("clip_video", {
        eventId: current_clip_event_id,
        parentVideoId: video.id,
        startTime: clipStartTime,
        endTime: clipEndTime,
        clipTitle: clipTitle,
      });
    } catch (error) {
      console.error("切片失败:", error);
      alert("切片失败: " + error);
      clipping = false;
      current_clip_event_id = null;
    }
  }

  function update_clip_prompt(text: string) {
    let span = document.getElementById("generate-clip-prompt");
    if (span) {
      span.textContent = text;
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

  // 键盘快捷键处理
  function handleKeydown(event: KeyboardEvent) {
    if (!show || !isVideoLoaded) return;

    const target = event.target as HTMLElement | null;
    const tagName = target?.tagName;

    // 如果当前焦点位于可编辑元素内，则跳过快捷键处理
    const isInInput =
      (!!tagName && ["INPUT", "TEXTAREA", "SELECT"].includes(tagName)) ||
      !!target?.isContentEditable ||
      !!target?.closest(
        "input, textarea, select, [contenteditable='true'], [data-hotkey-block]"
      );

    switch (event.key) {
      case "【":
      case "[":
        if (!isInInput && canBeClipped(video)) {
          event.preventDefault();
          setClipStartTime();
        }
        break;
      case "】":
      case "]":
        if (!isInInput && canBeClipped(video)) {
          event.preventDefault();
          setClipEndTime();
        }
        break;
      case "q":
      case "Q":
        if (!isInInput && canBeClipped(video)) {
          event.preventDefault();
          seekToClipStart();
        }
        break;
      case "e":
      case "E":
        if (!isInInput && canBeClipped(video)) {
          event.preventDefault();
          seekToClipEnd();
        }
        break;
      case " ":
        if (!isInInput) {
          event.preventDefault();
          togglePlay();
        }
        break;
      case "ArrowLeft":
        if (!isInInput) {
          event.preventDefault();
          if (videoElement) {
            videoElement.currentTime = Math.max(
              0,
              videoElement.currentTime - 5
            );
          }
        }
        break;
      case "ArrowRight":
        if (!isInInput) {
          event.preventDefault();
          if (videoElement) {
            videoElement.currentTime = Math.min(
              videoElement.duration,
              videoElement.currentTime + 5
            );
          }
        }
        break;
      case "g":
      case "G":
        if (!isInInput && canBeClipped(video)) {
          event.preventDefault();
          generateClip();
        }
        break;
      case "c":
      case "C":
        if (!isInInput && canBeClipped(video)) {
          event.preventDefault();
          clearClipSelection();
        }
        break;
      case "h":
      case "H":
        if (!isInInput && canBeClipped(video)) {
          event.preventDefault();
          show_detail = !show_detail;
        }
        break;
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
    // Find current subtitle
    currentSubtitleIndex = getCurrentSubtitleIndex();
    const currentSub = subtitles[currentSubtitleIndex];
    currentSubtitle = currentSub?.text || "";

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
    if (!videoElement?.duration) return; // 防御性检查

    subtitles = subtitles.map((sub, i) => {
      if (i !== index) return sub;

      if (isStart) {
        // 开始时间约束：不能小于0，不能大于结束时间-0.1秒
        const newStartTime = Math.max(0, Math.min(time, sub.endTime - 0.1));
        return { ...sub, startTime: newStartTime };
      } else {
        // 结束时间约束：不能小于开始时间+0.1秒，不能大于视频总长度
        const newEndTime = Math.min(
          videoElement.duration,
          Math.max(time, sub.startTime + 0.1)
        );
        return { ...sub, endTime: newEndTime };
      }
    });
    subtitles = subtitles.sort((a, b) => a.startTime - b.startTime);
  }

  function moveSubtitle(index: number, newStartTime: number) {
    if (!videoElement?.duration) return;

    const sub = subtitles[index];
    const duration = sub.endTime - sub.startTime;

    // 约束开始时间到有效范围 [0, videoDuration - duration]
    const finalStartTime = Math.max(
      0,
      Math.min(newStartTime, videoElement.duration - duration)
    );
    const finalEndTime = finalStartTime + duration;

    subtitles = subtitles.map((s, i) =>
      i === index
        ? { ...s, startTime: finalStartTime, endTime: finalEndTime }
        : s
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
          i === index ? { ...s, startTime: newTime } : s
        );
        subtitles = subtitles.sort((a, b) => a.startTime - b.startTime);
      }
    } else {
      const newTime = Math.min(videoElement.duration, sub.endTime + delta);
      if (newTime > sub.startTime + 0.1) {
        subtitles = subtitles.map((s, i) =>
          i === index ? { ...s, endTime: newTime } : s
        );
        subtitles = subtitles.sort((a, b) => a.startTime - b.startTime);
      }
    }
  }

  function handleTimelineMouseDown(
    e: MouseEvent,
    index: number,
    isStart: boolean
  ) {
    startEdgeDragging(index, isStart);
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

  function handleTimelineMouseMove(e: MouseEvent) {
    if (!draggingSubtitle || !timelineElement) return;

    const rect = timelineElement.getBoundingClientRect();
    const x = e.clientX - rect.left;
    // 直接使用容器的实际宽度计算时间，与 getSubtitleStyle 逻辑保持一致
    const time = (x / rect.width) * videoElement.duration;

    updateSubtitleTime(draggingSubtitle.index, draggingSubtitle.isStart, time);
  }

  function handleBlockMouseMove(e: MouseEvent) {
    if (draggingBlock === null || !timelineElement) return;

    const rect = timelineElement.getBoundingClientRect();
    const x = e.clientX - rect.left;
    // 直接使用容器的实际宽度计算时间，与 getSubtitleStyle 逻辑保持一致
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
    // 字幕块位置应该是相对于时间轴容器的百分比，不需要乘以缩放因子
    // 因为容器本身已经通过 style="width: {100 * timelineScale}%" 进行了缩放
    const start = (subtitle.startTime / videoElement.duration) * 100;
    const width =
      ((subtitle.endTime - subtitle.startTime) / videoElement.duration) * 100;
    return `left: ${start}%; width: ${width}%;`;
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

  function getCurrentSubtitleIndex(): number {
    return subtitles.findIndex(
      (sub) => currentTime >= sub.startTime && currentTime < sub.endTime
    );
  }

  function handleScaleChange(e: Event) {
    const input = e.target as HTMLInputElement;
    timelineScale = parseFloat(input.value);
    const rect = timelineElement.getBoundingClientRect();
    timelineWidth = rect.width;
    updateTimeMarkers();

    // 同步调整 waveform 容器宽度
    if (waveformContainer) {
      waveformContainer.style.width = `${100 * timelineScale}%`;
    }
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
    const clear_update_listener = await listen(
      `progress-update:${event_id}`,
      (e) => {
        update_encode_prompt(e.payload.content);
      }
    );
    const clear_finished_listener = await listen(
      `progress-finished:${event_id}`,
      (e) => {
        update_encode_prompt(`压制字幕`);
        if (!e.payload.success) {
          alert("压制失败: " + e.payload.message);
        }

        current_encode_event_id = null;

        clear_update_listener();
        clear_finished_listener();
      }
    );
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
      (v) => v.id === Number((e.target as HTMLSelectElement).value)
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

      <div class="flex items-center space-x-2">
        {#if canBeClipped(video)}
          <button
            class="px-4 py-1.5 text-sm bg-green-600 text-white rounded-md hover:bg-green-600/90 transition-colors duration-200 border border-gray-600/50 flex items-center space-x-2 disabled:opacity-50 disabled:cursor-not-allowed"
            on:click={generateClip}
            disabled={clipping || current_clip_event_id != null}
          >
            {#if clipping || current_clip_event_id != null}
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
              <Scissors class="w-4 h-4" />
            {/if}
            <span id="generate-clip-prompt"
              >{clipping ? "生成中..." : "生成切片"}</span
            >
          </button>
        {/if}
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
                d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 714 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
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

            <!-- 切片快捷键说明 -->
            {#if canBeClipped(video)}
              <div
                id="overlay"
                class="absolute top-2 left-2 rounded-md px-2 py-2 flex flex-col pointer-events-none"
                style="background-color: rgba(0, 0, 0, 0.5); color: white; font-size: 0.8em;"
              >
                <p style="margin: 0;">
                  快捷键说明
                  <kbd
                    style="border: 1px solid white; padding: 0 0.2em; border-radius: 0.2em; margin: 4px;"
                    >h</kbd
                  >展开
                </p>
                {#if show_detail}
                  <span>
                    <p style="margin: 0;">
                      <kbd
                        style="border: 1px solid white; padding: 0 0.2em; border-radius: 0.2em; margin: 4px;"
                        >[</kbd
                      >设定选区开始
                    </p>
                    <p style="margin: 0;">
                      <kbd
                        style="border: 1px solid white; padding: 0 0.2em; border-radius: 0.2em; margin: 4px;"
                        >]</kbd
                      >设定选区结束
                    </p>
                    <p style="margin: 0;">
                      <kbd
                        style="border: 1px solid white; padding: 0 0.2em; border-radius: 0.2em; margin: 4px;"
                        >q</kbd
                      >跳转到选区开始
                    </p>
                    <p style="margin: 0;">
                      <kbd
                        style="border: 1px solid white; padding: 0 0.2em; border-radius: 0.2em; margin: 4px;"
                        >e</kbd
                      >跳转到选区结束
                    </p>
                    <p style="margin: 0;">
                      <kbd
                        style="border: 1px solid white; padding: 0 0.2em; border-radius: 0.2em; margin: 4px;"
                        >g</kbd
                      >生成切片
                    </p>
                    <p style="margin: 0;">
                      <kbd
                        style="border: 1px solid white; padding: 0 0.2em; border-radius: 0.2em; margin: 4px;"
                        >c</kbd
                      >清除选区
                    </p>
                    <p style="margin: 0;">
                      <kbd
                        style="border: 1px solid white; padding: 0 0.2em; border-radius: 0.2em; margin: 4px;"
                        >Space</kbd
                      >播放/暂停
                    </p>
                    <p style="margin: 0;">
                      <kbd
                        style="border: 1px solid white; padding: 0 0.2em; border-radius: 0.2em; margin: 4px;"
                        >←</kbd
                      ><kbd
                        style="border: 1px solid white; padding: 0 0.2em; border-radius: 0.2em; margin: 4px;"
                        >→</kbd
                      >前进/后退
                    </p>
                  </span>
                {/if}
              </div>
            {/if}
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
                on:input={handleVolumeChange}
                class="w-20 h-1 bg-gray-600 rounded-lg appearance-none cursor-pointer"
              />
            </div>
          </div>
        </div>

        <!-- 时间轴容器（包含波形图和字幕时间轴） -->
        <div class="bg-[#1c1c1e] border-t border-gray-800/50">
          <div
            class="h-48 overflow-x-auto overflow-y-hidden sidebar-scrollbar"
            bind:this={timelineContainer}
            on:wheel|preventDefault={handleWheel}
          >
            {#if isWaveformLoading}
              <div class="flex items-center space-x-2 text-gray-400 w-full">
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
                <span class="text-sm">加载音频波形...</span>
              </div>
            {/if}
            <div
              bind:this={waveformContainer}
              class="w-full h-full waveform-container"
              style="min-height: 60px; width: 100%; height: 60px;"
              data-waveform-container
            ></div>

            <!-- 字幕时间轴 -->
            <!-- svelte-ignore a11y-click-events-have-key-events -->
            <div
              bind:this={timelineElement}
              class="relative h-32 group"
              style="width: {100 * timelineScale}%"
              on:mousemove={(e) => {
                if (!timelineElement) return;
                const rect = timelineElement.getBoundingClientRect();
                timelineWidth = rect.width;
                updateTimeMarkers();
              }}
              on:click|preventDefault|stopPropagation={(e) => {
                // 只有在不拖动进度条时才处理时间轴点击
                if (!isDraggingSeekbar) {
                  handleTimelineClick(e);
                }
              }}
            >
              <!-- 切片选区可视化 -->
              {#if canBeClipped(video) && clipTimesSet}
                <div
                  class="absolute top-0 left-0 right-0 h-1 group-hover:h-1.5 transition-all duration-200 z-15"
                >
                  <!-- 切片选中区域 -->
                  <div
                    class="absolute h-full bg-green-400/80 transition-all duration-200"
                    style="left: {(clipStartTime /
                      (videoElement?.duration || 1)) *
                      100}%; right: {100 -
                      (clipEndTime / (videoElement?.duration || 1)) * 100}%"
                  ></div>
                  <!-- 切片起点标记 -->
                  <div
                    class="absolute h-full w-0.5 bg-green-500 transition-all duration-200"
                    style="left: {(clipStartTime /
                      (videoElement?.duration || 1)) *
                      100}%"
                  ></div>
                  <!-- 切片终点标记 -->
                  <div
                    class="absolute h-full w-0.5 bg-green-500 transition-all duration-200"
                    style="left: {(clipEndTime /
                      (videoElement?.duration || 1)) *
                      100}%; transform: translateX(-100%)"
                  ></div>
                </div>
              {/if}
              <!-- 播放进度条容器 (借鉴Shaka Player样式) -->
              <div
                bind:this={seekbarElement}
                class="shaka-seek-bar-container absolute top-2 left-0 right-0 h-1 group-hover:h-1.5 bg-white/30 rounded-full cursor-pointer transition-all duration-200 z-10"
                class:dragging={isDraggingSeekbar}
                on:mousedown={handleSeekbarMouseDown}
              >
                <!-- 播放进度条 -->
                <div
                  class="h-full bg-[#0A84FF] rounded-full pointer-events-none transition-all duration-200"
                  class:no-transition={isDraggingSeekbar}
                  style="width: {((isDraggingSeekbar
                    ? previewTime
                    : currentTime) /
                    (videoElement?.duration || 1)) *
                    100}%"
                ></div>

                <!-- 播放进度条滑块 (hover或拖动时显示) -->
                <div
                  class="absolute top-1/2 -translate-y-1/2 w-3 h-3 bg-white rounded-full border-2 border-[#0A84FF] shadow-lg z-30 opacity-0 group-hover:opacity-100 transition-opacity duration-200"
                  class:opacity-100={isDraggingSeekbar}
                  style="left: calc({((isDraggingSeekbar
                    ? previewTime
                    : currentTime) /
                    (videoElement?.duration || 1)) *
                    100}% - 6px)"
                ></div>
              </div>

              <!-- 时间刻度 -->
              {#each timeMarkers as time}
                <div
                  class="absolute top-2 bottom-0 border-l border-gray-700"
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
                  class="absolute top-6 bottom-6 bg-[#0A84FF]/30 rounded-lg cursor-move"
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
        class="w-80 border-l border-gray-800/50 bg-[#2c2c2e] overflow-y-auto sidebar-scrollbar"
      >
        <!-- Tab 导航 -->
        <div class="flex border-b border-gray-800/50 bg-[#1c1c1e]">
          <button
            class="px-6 py-3 text-sm font-medium transition-all duration-200 relative"
            class:text-white={activeTab === "subtitle"}
            class:text-gray-400={activeTab !== "subtitle"}
            class:bg-[#2c2c2e]={activeTab === "subtitle"}
            class:bg-transparent={activeTab !== "subtitle"}
            on:click={() => (activeTab = "subtitle")}
          >
            字幕
            {#if activeTab === "subtitle"}
              <div
                class="absolute bottom-0 left-0 right-0 h-0.5 bg-[#0A84FF]"
              ></div>
            {/if}
          </button>

          <button
            class="px-6 py-3 text-sm font-medium transition-all duration-200 relative"
            class:text-white={activeTab === "upload"}
            class:text-gray-400={activeTab !== "upload"}
            class:bg-[#2c2c2e]={activeTab === "upload"}
            class:bg-transparent={activeTab !== "upload"}
            on:click={() => (activeTab = "upload")}
          >
            快速投稿
            {#if activeTab === "upload"}
              <div
                class="absolute bottom-0 left-0 right-0 h-0.5 bg-[#0A84FF]"
              ></div>
            {/if}
          </button>
        </div>

        <!-- Tab 内容 -->
        {#if activeTab === "subtitle"}
          <!-- 字幕 Tab 内容 -->
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
                    class="w-full px-3 py-2 bg-[#2c2c2e] text-white rounded-lg border border-gray-800/50 focus:border-[#0A84FF] transition duration-200 outline-none hover:border-gray-700/50"
                    placeholder="输入字幕文本"
                  />
                </div>
              {/each}
            </div>
          </div>
        {:else if activeTab === "upload"}
          <!-- 投稿 Tab 内容 -->
          <div class="p-4 space-y-6">
            <!-- 封面预览 -->
            {#if video && video.id != -1}
              <section>
                <div class="group">
                  <div
                    class="text-sm text-gray-400 mb-2 flex items-center justify-between"
                  >
                    <span>视频封面</span>
                    <button
                      class="text-[#0A84FF] hover:text-[#0A84FF]/80 transition-colors duration-200 flex items-center space-x-1"
                      on:click={() => (show_cover_editor = true)}
                    >
                      <Pen class="w-4 h-4" />
                      <span class="text-xs">创建新封面</span>
                    </button>
                  </div>
                  <div
                    class="relative rounded-xl overflow-hidden bg-black/20 border border-gray-800/50"
                  >
                    {#if video.cover && video.cover.trim() !== ""}
                      <img
                        src={video.cover}
                        alt="视频封面"
                        class="w-full"
                        on:error={handleCoverError}
                        style:display={showDefaultCoverIcon ? "none" : "block"}
                      />
                    {/if}
                    {#if !video.cover || video.cover.trim() === "" || showDefaultCoverIcon}
                      <div
                        class="w-full aspect-video flex items-center justify-center bg-gray-800"
                      >
                        <!-- 默认视频图标 -->
                        <svg
                          class="w-16 h-16 text-gray-400"
                          fill="none"
                          stroke="currentColor"
                          viewBox="0 0 24 24"
                        >
                          <path
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            stroke-width="2"
                            d="M15 10l4.553-2.276A1 1 0 0121 8.618v6.764a1 1 0 01-1.447.894L15 14M5 18h8a2 2 0 002-2V8a2 2 0 00-2-2H5a2 2 0 00-2 2v8a2 2 0 002 2z"
                          ></path>
                        </svg>
                      </div>
                    {/if}
                  </div>
                </div>
              </section>
            {/if}

            <!-- 基本信息 -->
            <div class="space-y-4">
              <h3 class="text-sm font-medium text-gray-400">基本信息</h3>
              <!-- 标题 -->
              <div class="space-y-2">
                <label
                  for="title"
                  class="block text-sm font-medium text-gray-300">标题</label
                >
                <input
                  id="title"
                  type="text"
                  bind:value={profile.title}
                  placeholder="输入视频标题"
                  class="w-full px-3 py-2 bg-[#1c1c1e] text-white rounded-lg border border-gray-800/50 focus:border-[#0A84FF] transition duration-200 outline-none hover:border-gray-700/50"
                />
              </div>

              <!-- 视频分区 -->
              <div class="space-y-2">
                <label for="tid" class="block text-sm font-medium text-gray-300"
                  >视频分区</label
                >
                <div class="w-full" id="tid">
                  <TypeSelect bind:value={profile.tid} />
                </div>
              </div>

              <!-- 投稿账号 -->
              <div class="space-y-2">
                <label for="uid" class="block text-sm font-medium text-gray-300"
                  >投稿账号</label
                >
                <select
                  bind:value={uid_selected}
                  class="w-full px-3 py-2 bg-[#1c1c1e] text-white rounded-lg border border-gray-800/50 focus:border-[#0A84FF] transition duration-200 outline-none appearance-none hover:border-gray-700/50"
                >
                  <option value={0}>选择账号</option>
                  {#each accounts as account}
                    <option value={account.value}>{account.name}</option>
                  {/each}
                </select>
              </div>
            </div>

            <!-- 详细信息 -->
            <div class="space-y-4">
              <h3 class="text-sm font-medium text-gray-400">详细信息</h3>
              <!-- 描述 -->
              <div class="space-y-2">
                <label
                  for="desc"
                  class="block text-sm font-medium text-gray-300">描述</label
                >
                <textarea
                  id="desc"
                  bind:value={profile.desc}
                  placeholder="输入视频描述"
                  class="w-full px-3 py-2 bg-[#1c1c1e] text-white rounded-lg border border-gray-800/50 focus:border-[#0A84FF] transition duration-200 outline-none resize-none h-24 hover:border-gray-700/50"
                />
              </div>

              <!-- 标签 -->
              <div class="space-y-2">
                <label for="tag" class="block text-sm font-medium text-gray-300"
                  >标签</label
                >
                <input
                  id="tag"
                  type="text"
                  bind:value={profile.tag}
                  placeholder="输入视频标签，用逗号分隔"
                  class="w-full px-3 py-2 bg-[#1c1c1e] text-white rounded-lg border border-gray-800/50 focus:border-[#0A84FF] transition duration-200 outline-none hover:border-gray-700/50"
                />
              </div>

              <!-- 动态 -->
              <div class="space-y-2">
                <label
                  for="dynamic"
                  class="block text-sm font-medium text-gray-300">动态</label
                >
                <textarea
                  id="dynamic"
                  bind:value={profile.dynamic}
                  placeholder="输入动态内容"
                  class="w-full px-3 py-2 bg-[#1c1c1e] text-white rounded-lg border border-gray-800/50 focus:border-[#0A84FF] transition duration-200 outline-none resize-none h-24 hover:border-gray-700/50"
                />
              </div>
            </div>

            <!-- 投稿按钮 -->
            {#if video}
              <div class="pt-4">
                <div class="flex gap-2">
                  <button
                    on:click={do_post}
                    disabled={current_post_event_id != null || !uid_selected}
                    class="flex-1 px-3 py-2 bg-[#0A84FF] text-white rounded-lg transition-all duration-200 hover:bg-[#0A84FF]/90 disabled:opacity-50 disabled:cursor-not-allowed flex items-center justify-center space-x-2 text-sm"
                  >
                    {#if current_post_event_id != null}
                      <div
                        class="w-3 h-3 border-2 border-current border-t-transparent rounded-full animate-spin"
                      />
                    {/if}
                    <span id="post-prompt">投稿</span>
                  </button>
                  {#if current_post_event_id != null}
                    <button
                      on:click={() => cancel_post()}
                      class="px-3 py-2 bg-red-500 text-white rounded-lg transition-all duration-200 hover:bg-red-500/90 flex items-center justify-center text-sm"
                    >
                      取消
                    </button>
                  {/if}
                </div>
              </div>
            {/if}
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<SubtitleStyleEditor
  bind:show={showStyleEditor}
  {roomId}
  onClose={() => (showStyleEditor = false)}
/>

<CoverEditor
  bind:show={show_cover_editor}
  {video}
  on:coverUpdate={(event) => {
    video = {
      ...video,
      cover: event.detail.cover,
    };
  }}
/>

<!-- 键盘快捷键监听 -->
<svelte:window on:keydown={handleKeydown} />

<style>
  /* 拖动时禁用过渡动画，避免与JS更新冲突 */
  .no-transition {
    transition: none !important;
  }

  /* 确保层级顺序正确 */
  .z-15 {
    z-index: 15;
  }

  /* Shaka Player风格的进度条样式 */
  .shaka-seek-bar-container {
    position: relative;
    background: rgba(255, 255, 255, 0.3);
    transition: height 0.2s cubic-bezier(0.4, 0, 1, 1);
  }

  .shaka-seek-bar-container:hover {
    background: rgba(255, 255, 255, 0.4);
  }

  /* 确保切片选区在hover时也有相同的高度变化 */
  .group:hover .shaka-seek-bar-container {
    height: 6px;
  }

  /* 拖动状态样式 */
  .shaka-seek-bar-container.dragging {
    background: rgba(255, 255, 255, 0.5);
    height: 6px;
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
    background: #1c1c1e;
    border-radius: 4px;
    width: 100%;
    height: 100%;
  }
</style>
