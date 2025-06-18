<script lang="ts">
  import { Play, X, Type, Palette, Move, Plus, Trash2 } from "lucide-svelte";
  import { invoke, log } from "../lib/invoker";
  import { onMount, createEventDispatcher } from "svelte";

  const dispatch = createEventDispatcher();
  export let video = null;
  export let show: boolean = false;

  // 文本列表
  let texts = [
    {
      id: 1,
      content: "",
      position: { x: 50, y: 50 },
      fontSize: 48,
      color: "#FF7F00",
      strokeColor: "#FFFFFF",
    },
  ];

  let selectedTextId = 1;

  let isDragging = false;
  let startPos = { x: 0, y: 0 };
  let startTextPos = { x: 0, y: 0 };

  let videoElement: HTMLVideoElement;
  let videoFrame;
  let isVideoLoaded = false;
  let currentTime = 0;
  let duration = 0;

  let canvas: HTMLCanvasElement;
  let ctx: CanvasRenderingContext2D;
  let canvasWidth = 1280;
  let canvasHeight = 720;
  let scale = 1;
  let backgroundImage: HTMLImageElement;
  let redrawRequestId: number | null = null;
  let isRedrawScheduled = false;

  onMount(() => {
    ctx = canvas.getContext("2d");
    loadBackgroundImage();
    resizeCanvas();
    window.addEventListener("resize", handleResize);
    return () => {
      window.removeEventListener("resize", handleResize);
      if (redrawRequestId !== null) {
        cancelAnimationFrame(redrawRequestId);
      }
    };
  });

  function handleResize() {
    if (!isRedrawScheduled) {
      isRedrawScheduled = true;
      redrawRequestId = requestAnimationFrame(() => {
        resizeCanvas();
        isRedrawScheduled = false;
      });
    }
  }

  function scheduleRedraw() {
    if (!isRedrawScheduled) {
      isRedrawScheduled = true;
      redrawRequestId = requestAnimationFrame(() => {
        redraw();
        isRedrawScheduled = false;
      });
    }
  }

  function loadBackgroundImage() {
    if (!videoFrame) {
      return;
    }
    backgroundImage = new Image();
    backgroundImage.crossOrigin = "anonymous";
    backgroundImage.onload = () => {
      scheduleRedraw();
    };
    backgroundImage.onerror = (e) => {
      log.error("Failed to load image:", e);
    };
    backgroundImage.src = videoFrame;
  }

  function resizeCanvas() {
    const container = document.getElementById("cover-container");
    if (!container) return;

    const rect = container.getBoundingClientRect();
    scale = rect.width / canvasWidth;
    canvas.style.width = `${rect.width}px`;
    canvas.style.height = `${rect.height}px`;
    canvas.width = canvasWidth;
    canvas.height = canvasHeight;

    scheduleRedraw();
  }

  function redraw() {
    if (!ctx) return;

    // 清空画布
    ctx.clearRect(0, 0, canvas.width, canvas.height);

    // 绘制背景图片
    if (backgroundImage && backgroundImage.complete) {
      ctx.drawImage(backgroundImage, 0, 0, canvas.width, canvas.height);
    }

    // 绘制所有文本
    texts.forEach((text) => {
      drawText(text);
    });
  }

  function drawText(text) {
    if (!ctx) return;

    const x = (text.position.x / 100) * canvas.width;
    const y = (text.position.y / 100) * canvas.height;

    ctx.font = `bold ${text.fontSize}px sans-serif`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";

    // 绘制描边
    ctx.strokeStyle = text.strokeColor;
    ctx.lineWidth = 4;
    ctx.lineJoin = "round";
    ctx.miterLimit = 2;
    ctx.strokeText(text.content, x, y);

    // 绘制半透明描边
    ctx.strokeStyle = `${text.strokeColor}80`;
    ctx.lineWidth = 6;
    ctx.strokeText(text.content, x, y);

    // 绘制文本
    ctx.fillStyle = text.color;
    ctx.fillText(text.content, x, y);
  }

  function handleMouseDown(event: MouseEvent) {
    const rect = canvas.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;

    // 检查是否点击到文本
    texts.forEach((text) => {
      const textX = (text.position.x / 100) * rect.width;
      const textY = (text.position.y / 100) * rect.height;

      ctx.font = `bold ${text.fontSize}px sans-serif`;
      const metrics = ctx.measureText(text.content);
      const textWidth = metrics.width;
      const textHeight = text.fontSize;

      if (
        x >= textX - textWidth / 2 - 10 &&
        x <= textX + textWidth / 2 + 10 &&
        y >= textY - textHeight / 2 - 10 &&
        y <= textY + textHeight / 2 + 10
      ) {
        isDragging = true;
        selectedTextId = text.id;
        startPos = { x: event.clientX, y: event.clientY };
        startTextPos = { ...text.position };
      }
    });
  }

  function handleMouseMove(event: MouseEvent) {
    if (!isDragging) return;

    const rect = canvas.getBoundingClientRect();
    const deltaX = ((event.clientX - startPos.x) / rect.width) * 100;
    const deltaY = ((event.clientY - startPos.y) / rect.height) * 100;

    // 限制文本位置在画布范围内
    const newX = Math.max(0, Math.min(100, startTextPos.x + deltaX));
    const newY = Math.max(0, Math.min(100, startTextPos.y + deltaY));

    texts = texts.map((text) => {
      if (text.id === selectedTextId) {
        return {
          ...text,
          position: {
            x: newX,
            y: newY,
          },
        };
      }
      return text;
    });

    scheduleRedraw();
  }

  function handleMouseUp() {
    if (isDragging) {
      isDragging = false;
    }
  }

  function addNewText() {
    const newId = Math.max(0, ...texts.map((t) => t.id)) + 1;
    texts = [
      ...texts,
      {
        id: newId,
        content: "",
        position: { x: 50, y: 50 },
        fontSize: 48,
        color: "#FF7F00",
        strokeColor: "#FFFFFF",
      },
    ];
    selectedTextId = newId;
    scheduleRedraw();
  }

  function deleteText(id: number) {
    texts = texts.filter((t) => t.id !== id);
    if (texts.length > 0) {
      selectedTextId = texts[0].id;
    }
    scheduleRedraw();
  }

  function handleVideoLoaded() {
    isVideoLoaded = true;
    duration = videoElement.duration;
    updateCoverFromVideo();
  }

  function handleTimeUpdate() {
    currentTime = videoElement.currentTime;
  }

  function handleSeek(event: Event) {
    const target = event.target as HTMLInputElement;
    const time = parseFloat(target.value);
    if (videoElement) {
      videoElement.currentTime = time;
      updateCoverFromVideo();
    }
  }

  function formatTime(seconds: number): string {
    const mins = Math.floor(seconds / 60);
    const secs = Math.floor(seconds % 60);
    return `${mins}:${secs.toString().padStart(2, "0")}`;
  }

  function updateCoverFromVideo() {
    if (!videoElement) return;

    const tempCanvas = document.createElement("canvas");
    tempCanvas.width = videoElement.videoWidth;
    tempCanvas.height = videoElement.videoHeight;
    const tempCtx = tempCanvas.getContext("2d");
    tempCtx.drawImage(videoElement, 0, 0, tempCanvas.width, tempCanvas.height);
    videoFrame = tempCanvas.toDataURL("image/jpeg");
    loadBackgroundImage();
  }

  function handleClose() {
    show = false;
  }

  async function handleSave() {
    // 确保 Canvas 已完全渲染
    await new Promise<void>((resolve) => {
      requestAnimationFrame(async () => {
        // 强制重绘一次
        redraw();
        // 等待一帧以确保渲染完成
        requestAnimationFrame(async () => {
          try {
            // 直接使用 canvas 的内容作为新封面
            const newCover = canvas.toDataURL("image/jpeg");

            await invoke("update_video_cover", {
              id: video.value,
              cover: newCover,
            });

            // 触发自定义事件通知父组件更新封面
            dispatch("coverUpdate", { cover: newCover });
            handleClose();
          } catch (e) {
            alert("更新封面失败: " + e);
          }
          resolve();
        });
      });
    });
  }

  function handleTextInput(text, event: Event) {
    const target = event.target as HTMLTextAreaElement;
    text.content = target.value;
    scheduleRedraw();
  }

  $: {
    // 当文本内容或样式改变时重绘
    if (ctx) {
      texts = texts.map((text) => {
        if (text.id === selectedTextId) {
          return {
            ...text,
            content: text.content,
            fontSize: text.fontSize,
            color: text.color,
            position: text.position,
          };
        }
        return text;
      });
      scheduleRedraw();
    }
  }

  $: selectedText = texts.find((t) => t.id === selectedTextId);

  // 监听 show 变化，当模态框显示时重新绘制
  $: if (show && ctx) {
    setTimeout(() => {
      loadBackgroundImage();
      resizeCanvas();
    }, 50);
  }
</script>

<svelte:window
  on:mousemove={handleMouseMove}
  on:mouseup={handleMouseUp}
  on:blur={() => (isDragging = false)}
  on:visibilitychange={() => {
    if (document.hidden) {
      isDragging = false;
    }
  }}
/>

<!-- Modal Backdrop -->
<div
  class="fixed inset-0 bg-black/30 backdrop-blur-sm z-[1000] transition-opacity duration-200"
  class:opacity-0={!show}
  class:opacity-100={show}
  class:pointer-events-none={!show}
>
  <!-- Modal Content -->
  <div
    class="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 w-[800px] bg-[#1c1c1e] rounded-2xl shadow-2xl overflow-hidden transition-all duration-200"
    class:opacity-0={!show}
    class:opacity-100={show}
    class:scale-95={!show}
    class:scale-100={show}
  >
    <!-- Header -->
    <div
      class="flex items-center justify-between px-6 py-4 border-b border-gray-800/50 bg-[#2c2c2e]"
    >
      <h3 class="text-base font-medium text-white">编辑封面</h3>
      <button
        class="w-[22px] h-[22px] rounded-full bg-[#ff5f57] hover:bg-[#ff5f57]/90 transition-colors duration-200 flex items-center justify-center group"
        on:click={handleClose}
      >
        <X
          class="w-3 h-3 text-[#1c1c1e] opacity-0 group-hover:opacity-100 transition-opacity duration-200"
        />
      </button>
    </div>

    <!-- Body -->
    <div class="p-5 space-y-4">
      <!-- Video Frame Selection -->
      <div class="space-y-2">
        <div class="text-sm text-gray-400 flex items-center justify-between">
          <span class="font-medium">选择视频帧</span>
          <div class="flex items-center space-x-2 text-xs">
            <span>{formatTime(currentTime)}</span>
            <span class="opacity-50">/</span>
            <span>{formatTime(duration)}</span>
          </div>
        </div>

        <!-- Hidden Video Element -->
        <!-- svelte-ignore a11y-media-has-caption -->
        <video
          bind:this={videoElement}
          src={video?.file}
          class="hidden"
          crossorigin="anonymous"
          on:loadedmetadata={handleVideoLoaded}
          on:timeupdate={handleTimeUpdate}
        />

        <!-- Video Controls -->
        <div class="flex items-center space-x-2">
          <input
            type="range"
            min="0"
            max={duration}
            step="0.1"
            bind:value={currentTime}
            on:input={handleSeek}
            class="flex-1"
            disabled={!isVideoLoaded}
          />
        </div>
      </div>

      <!-- Cover Preview -->
      <div class="space-y-2">
        <div class="text-sm text-gray-400 flex items-center justify-between">
          <div class="flex items-center space-x-2">
            <span class="font-medium">视频封面</span>
            <span class="text-xs opacity-60">(拖拽文字调整位置)</span>
          </div>
        </div>
        <div
          id="cover-container"
          class="relative rounded-xl overflow-hidden bg-black/20 border border-gray-800/50 aspect-video"
        >
          <canvas
            bind:this={canvas}
            on:mousedown={handleMouseDown}
            class="w-full h-full"
          />
        </div>
      </div>

      <!-- Text Controls -->
      <div class="space-y-3">
        <div class="flex items-start space-x-4">
          <!-- Text List and Input -->
          <div class="flex-1 space-y-3">
            <!-- Text List -->
            <div class="flex items-center justify-between">
              <!-- svelte-ignore a11y-label-has-associated-control -->
              <label
                class="flex items-center space-x-2 text-sm font-medium text-gray-300"
              >
                <Type class="w-4 h-4" />
                <span>文字列表</span>
              </label>
              <button
                on:click={addNewText}
                class="p-1.5 rounded-lg bg-[#2c2c2e] hover:bg-[#3c3c3e] transition-colors duration-200 text-white"
              >
                <Plus class="w-4 h-4" />
              </button>
            </div>
            <div class="space-y-1.5">
              {#each texts as text (text.id)}
                <!-- svelte-ignore a11y-click-events-have-key-events -->
                <div
                  class="flex items-center space-x-2 p-2 rounded-lg transition-colors duration-200 cursor-pointer"
                  class:bg-[#2c2c2e]={selectedTextId === text.id}
                  on:click={() => (selectedTextId = text.id)}
                >
                  <textarea
                    value={text.content}
                    on:input={(e) => handleTextInput(text, e)}
                    placeholder="输入文字内容"
                    class="flex-1 bg-transparent text-white text-sm outline-none resize-none placeholder:text-gray-500"
                  />
                  {#if texts.length > 1}
                    <button
                      on:click={() => deleteText(text.id)}
                      class="p-1 rounded hover:bg-[#3c3c3e] transition-colors duration-200 text-red-500"
                    >
                      <Trash2 class="w-4 h-4" />
                    </button>
                  {/if}
                </div>
              {/each}
            </div>
          </div>

          <!-- Text Style Controls -->
          {#if selectedText}
            <div class="w-48 space-y-2">
              <!-- svelte-ignore a11y-label-has-associated-control -->
              <label
                class="flex items-center space-x-2 text-sm font-medium text-gray-300"
              >
                <Palette class="w-4 h-4" />
                <span>文字样式</span>
              </label>
              <div
                class="space-y-3 p-2.5 rounded-lg bg-[#2c2c2e] border border-gray-800/50"
              >
                <!-- Font Size -->
                <div class="space-y-1">
                  <label
                    for="fontSize"
                    class="text-xs text-gray-400 font-medium">字体大小</label
                  >
                  <input
                    id="fontSize"
                    type="range"
                    bind:value={selectedText.fontSize}
                    min="48"
                    max="160"
                    class="w-full"
                  />
                </div>
                <!-- Colors -->
                <div class="grid grid-cols-2 gap-2">
                  <!-- Text Color -->
                  <div class="space-y-1">
                    <label
                      for="textColor"
                      class="text-xs text-gray-400 font-medium">文字颜色</label
                    >
                    <input
                      id="textColor"
                      type="color"
                      bind:value={selectedText.color}
                      class="w-full h-7 rounded-lg cursor-pointer"
                    />
                  </div>
                  <!-- Stroke Color -->
                  <div class="space-y-1">
                    <label
                      for="strokeColor"
                      class="text-xs text-gray-400 font-medium">描边颜色</label
                    >
                    <input
                      id="strokeColor"
                      type="color"
                      bind:value={selectedText.strokeColor}
                      class="w-full h-7 rounded-lg cursor-pointer"
                    />
                  </div>
                </div>
              </div>
            </div>
          {/if}
        </div>
      </div>
    </div>

    <!-- Footer -->
    <div
      class="px-5 py-3 border-t border-gray-800/50 flex justify-end space-x-3"
    >
      <button
        class="px-4 py-1.5 text-gray-400 hover:text-white transition-colors duration-200 text-sm font-medium"
        on:click={handleClose}
      >
        取消
      </button>
      <button
        class="px-4 py-1.5 bg-[#0A84FF] text-white rounded-lg hover:bg-[#0A84FF]/90 transition-colors duration-200 text-sm font-medium"
        on:click={handleSave}
      >
        确定
      </button>
    </div>
  </div>
</div>

<style>
  input[type="range"] {
    -webkit-appearance: none;
    appearance: none;
    background: transparent;
    height: 24px;
    margin: -8px 0;
  }

  input[type="range"]::-webkit-slider-runnable-track {
    width: 100%;
    height: 4px;
    background: #4a4a4a;
    border-radius: 2px;
    cursor: pointer;
  }

  input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    height: 16px;
    width: 16px;
    border-radius: 50%;
    background: #0a84ff;
    margin-top: -6px;
    cursor: pointer;
    box-shadow: 0 2px 4px rgba(0, 0, 0, 0.2);
    transition: transform 0.2s ease;
  }

  input[type="range"]:hover::-webkit-slider-thumb {
    transform: scale(1.1);
  }

  input[type="range"]:active::-webkit-slider-thumb {
    transform: scale(0.95);
    background: #0A84FF/90;
  }

  input[type="range"]:focus {
    outline: none;
  }

  input[type="color"] {
    -webkit-appearance: none;
    appearance: none;
    border: none;
    padding: 0;
    background: transparent;
  }

  input[type="color"]::-webkit-color-swatch-wrapper {
    padding: 0;
  }

  input[type="color"]::-webkit-color-swatch {
    border: none;
    border-radius: 6px;
  }

  textarea {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
      Helvetica, Arial, sans-serif;
  }

  textarea::placeholder {
    color: rgba(255, 255, 255, 0.3);
  }
</style>
