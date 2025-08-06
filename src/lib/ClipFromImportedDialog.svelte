<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { invoke, convertFileSrc } from "./invoker";
  import type { VideoItem } from "./interface";
  import { X, Play, Scissors } from "lucide-svelte";

  export let showDialog = false;
  export let videoToClip: VideoItem | null = null;

  const dispatch = createEventDispatcher();

  let startTime = 0;
  let endTime = 0;
  let clipTitle = "";
  let clipping = false;
  let previewVideo: HTMLVideoElement;
  let videoDuration = 0;
  let videoLoaded = false;

  // 当视频改变时重置状态
  $: if (videoToClip && showDialog) {
    resetDialog();
    loadVideo();
  }

  function resetDialog() {
    startTime = 0;
    endTime = 0;
    clipTitle = "";
    clipping = false;
    videoDuration = 0;
    videoLoaded = false;
  }

  async function loadVideo() {
    if (!videoToClip) return;
    
    try {
      // 检查是否在Tauri环境中
      const TAURI_ENV = typeof window.__TAURI_INTERNALS__ !== "undefined";
      
      let videoUrl;
      if (TAURI_ENV) {
        // 在Tauri环境中，获取配置并使用convertFileSrc
        const config: any = await invoke("get_config");
        const fullPath = `${config.output}/${videoToClip.file}`;
        videoUrl = convertFileSrc(fullPath);
      } else {
        // 在Web环境中，使用配置的端点或当前origin
        const ENDPOINT = localStorage.getItem("endpoint") || "";
        videoUrl = `${ENDPOINT || window.location.origin}/output/${videoToClip.file}`;
      }
      
      console.log("Loading video:", videoToClip, "URL:", videoUrl);
      
      if (previewVideo) {
        previewVideo.src = videoUrl;
        previewVideo.load();
      }
    } catch (error) {
      console.error("Failed to load video:", error);
    }
  }

  function onVideoLoaded() {
    console.log("Video loaded successfully");
    if (previewVideo) {
      videoDuration = previewVideo.duration;
      endTime = videoDuration;
      videoLoaded = true;
      console.log("Video duration:", videoDuration);
    }
  }
  
  function onVideoError(event: Event) {
    console.error("Video load error:", event);
    const video = event.target as HTMLVideoElement;
    console.error("Video error details:", video.error);
  }

  function setStartTime() {
    if (previewVideo) {
      startTime = previewVideo.currentTime;
      if (startTime >= endTime) {
        endTime = Math.min(startTime + 10, videoDuration);
      }
    }
  }

  function setEndTime() {
    if (previewVideo) {
      endTime = previewVideo.currentTime;
      if (endTime <= startTime) {
        startTime = Math.max(endTime - 10, 0);
      }
    }
  }

  function seekToStart() {
    if (previewVideo) {
      previewVideo.currentTime = startTime;
    }
  }

  function seekToEnd() {
    if (previewVideo) {
      previewVideo.currentTime = endTime;
    }
  }

  function formatTime(seconds: number): string {
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    const secs = Math.floor(seconds % 60);
    
    if (hours > 0) {
      return `${hours.toString().padStart(2, '0')}:${minutes.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
    } else {
      return `${minutes.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
    }
  }

  async function startClip() {
    if (!videoToClip || !clipTitle.trim()) return;
    
    if (startTime >= endTime) {
      alert("开始时间必须小于结束时间");
      return;
    }

    if (endTime - startTime < 1) {
      alert("切片长度不能少于1秒");
      return;
    }

    clipping = true;
    try {
      const eventId = `clip_${Date.now()}`;
      
      // 根据视频类型选择不同的切片命令
      if (videoToClip.platform === "imported") {
        await invoke("clip_from_imported_video", {
          eventId,
          parentVideoId: videoToClip.id,
          startTime,
          endTime,
          clipTitle: clipTitle.trim(),
        });
      } else {
        // 对于录制的视频使用新的切片命令
        await invoke("clip_from_recorded_video", {
          eventId,
          parentVideoId: videoToClip.id,
          startTime,
          endTime,
          clipTitle: clipTitle.trim(),
        });
      }
      
      dispatch("clipped");
      closeDialog();
    } catch (error) {
      console.error("Failed to clip video:", error);
      alert(`切片失败: ${error}`);
    } finally {
      clipping = false;
    }
  }

  function closeDialog() {
    showDialog = false;
    resetDialog();
  }

  // 处理ESC键关闭对话框
  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Escape" && !clipping) {
      closeDialog();
    }
  }

  function formatPlatformName(platform: string): string {
    switch (platform?.toLowerCase()) {
      case "bilibili":
        return "B站";
      case "douyin":
        return "抖音";
      case "huya":
        return "虎牙";
      case "youtube":
        return "YouTube";
      default:
        return platform || "未知";
    }
  }
</script>

<svelte:window on:keydown={handleKeydown} />

{#if showDialog && videoToClip}
  <div class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
    <div class="bg-white dark:bg-gray-800 rounded-lg shadow-xl max-w-4xl w-full mx-4 max-h-[90vh] flex flex-col">
      <!-- 头部 -->
      <div class="flex items-center justify-between p-6 border-b border-gray-200 dark:border-gray-700">
        <h3 class="text-lg font-semibold text-gray-900 dark:text-white">
          切片视频: {videoToClip.title || videoToClip.file}
          {#if videoToClip.platform !== "imported"}
            <span class="text-sm text-gray-500 ml-2">({formatPlatformName(videoToClip.platform)})</span>
          {/if}
        </h3>
        <button
          on:click={closeDialog}
          disabled={clipping}
          class="text-gray-400 hover:text-gray-600 dark:hover:text-gray-300 disabled:opacity-50"
        >
          <X class="w-6 h-6" />
        </button>
      </div>

      <!-- 内容区域 -->
      <div class="flex-1 overflow-y-auto p-6">
        <div class="space-y-6">
          <!-- 视频预览 -->
          <div class="space-y-4">
            <h4 class="text-md font-medium text-gray-900 dark:text-white">视频预览</h4>
            <div class="bg-black rounded-lg overflow-hidden">
              <video
                bind:this={previewVideo}
                on:loadedmetadata={onVideoLoaded}
                on:error={onVideoError}
                controls
                class="w-full h-64 object-contain"
              >
                <track kind="captions" label="Empty captions" />
                您的浏览器不支持视频播放
              </video>
            </div>
          </div>

          {#if videoLoaded}
            <!-- 时间选择 -->
            <div class="grid grid-cols-1 md:grid-cols-2 gap-6">
              <!-- 开始时间 -->
              <div class="space-y-2">
                <label for="startTime" class="block text-sm font-medium text-gray-700 dark:text-gray-300">
                  开始时间
                </label>
                <div class="flex items-center space-x-2">
                  <input
                    id="startTime"
                    type="number"
                    bind:value={startTime}
                    min="0"
                    max={videoDuration}
                    step="0.1"
                    class="flex-1 px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500 dark:bg-gray-700 dark:text-white"
                    disabled={clipping}
                  />
                  <span class="text-sm text-gray-500 dark:text-gray-400 min-w-[60px]">
                    {formatTime(startTime)}
                  </span>
                  <button
                    on:click={setStartTime}
                    disabled={clipping}
                    class="px-2 py-2 bg-blue-600 text-white rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed text-xs"
                    title="设置为当前播放时间"
                  >
                    用当前时间
                  </button>
                  <button
                    on:click={seekToStart}
                    disabled={clipping}
                    class="px-3 py-2 bg-gray-600 text-white rounded-md hover:bg-gray-700 disabled:opacity-50 disabled:cursor-not-allowed"
                    title="跳转到开始时间点"
                  >
                    <Play class="w-4 h-4" />
                  </button>
                </div>
              </div>

              <!-- 结束时间 -->
              <div class="space-y-2">
                <label for="endTime" class="block text-sm font-medium text-gray-700 dark:text-gray-300">
                  结束时间
                </label>
                <div class="flex items-center space-x-2">
                  <input
                    id="endTime"
                    type="number"
                    bind:value={endTime}
                    min="0"
                    max={videoDuration}
                    step="0.1"
                    class="flex-1 px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500 dark:bg-gray-700 dark:text-white"
                    disabled={clipping}
                  />
                  <span class="text-sm text-gray-500 dark:text-gray-400 min-w-[60px]">
                    {formatTime(endTime)}
                  </span>
                  <button
                    on:click={setEndTime}
                    disabled={clipping}
                    class="px-2 py-2 bg-blue-600 text-white rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed text-xs"
                    title="设置为当前播放时间"
                  >
                    用当前时间
                  </button>
                  <button
                    on:click={seekToEnd}
                    disabled={clipping}
                    class="px-3 py-2 bg-gray-600 text-white rounded-md hover:bg-gray-700 disabled:opacity-50 disabled:cursor-not-allowed"
                    title="跳转到结束时间点"
                  >
                    <Play class="w-4 h-4" />
                  </button>
                </div>
              </div>
            </div>

            <!-- 切片信息 -->
            <div class="space-y-2">
              <div class="text-sm text-gray-600 dark:text-gray-400">
                切片时长: {formatTime(Math.max(0, endTime - startTime))}
              </div>
            </div>

            <!-- 切片标题 -->
            <div class="space-y-2">
              <label for="clipTitle" class="block text-sm font-medium text-gray-700 dark:text-gray-300">
                切片标题 *
              </label>
              <input
                id="clipTitle"
                type="text"
                bind:value={clipTitle}
                placeholder="请输入切片标题"
                class="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500 dark:bg-gray-700 dark:text-white"
                disabled={clipping}
              />
            </div>
          {/if}
        </div>
      </div>

      <!-- 底部按钮 -->
      <div class="border-t border-gray-200 dark:border-gray-700 px-6 py-4">
        <div class="flex justify-end space-x-3">
          <button
            on:click={closeDialog}
            disabled={clipping}
            class="px-4 py-2 text-gray-700 dark:text-gray-300 bg-gray-100 dark:bg-gray-600 hover:bg-gray-200 dark:hover:bg-gray-500 rounded-md disabled:opacity-50 disabled:cursor-not-allowed"
          >
            取消
          </button>
          <button
            on:click={startClip}
            disabled={clipping || !videoLoaded || !clipTitle.trim() || startTime >= endTime}
            class="px-4 py-2 bg-green-600 text-white rounded-md hover:bg-green-700 disabled:opacity-50 disabled:cursor-not-allowed flex items-center space-x-2"
          >
            <Scissors class="w-4 h-4" />
            <span>{clipping ? "切片中..." : "开始切片"}</span>
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}