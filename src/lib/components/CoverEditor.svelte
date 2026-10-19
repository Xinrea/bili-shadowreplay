<script lang="ts">
  import { Play, X, Type, Palette, Move, Plus, Trash2 } from "lucide-svelte";
  import { invoke, log } from "../invoker";
  import { onMount } from "svelte";

  interface Props {
    video?: any;
    /** Background to use when there is no video to pick frames from. */
    image?: string;
    show?: boolean;
    onCoverUpdate?: (cover: { cover: string }) => void;
    /** Store the cover somewhere other than the video, such as a project. */
    onSave?: (cover: string) => Promise<void>;
  }

  let { video = null, image = "", show = $bindable(false), onCoverUpdate, onSave }: Props = $props();

  // 文本列表
  let texts = $state([
    {
      id: 1,
      content: "",
      position: { x: 50, y: 50 },
      fontSize: 48,
      color: "#FF7F00",
      strokeColor: "#FFFFFF",
    },
  ]);

  let selectedTextId = $state(1);

  let isDragging = $state(false);
  let startPos = { x: 0, y: 0 };
  let startTextPos = { x: 0, y: 0 };

  let videoElement: HTMLVideoElement = $state();
  let videoFrame;
  let isVideoLoaded = $state(false);
  let currentTime = $state(0);
  let duration = $state(0);

  let canvas: HTMLCanvasElement = $state();
  let ctx: CanvasRenderingContext2D = $state();
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
    const source = videoFrame || image;
    if (!source) {
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
    backgroundImage.src = source;
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
    // 延迟一点时间确保视频完全准备好
    setTimeout(() => {
      updateCoverFromVideo();
    }, 100);
  }

  function handleTimeUpdate() {
    currentTime = videoElement.currentTime;
  }

  function handleSeek(event: Event) {
    const target = event.target as HTMLInputElement;
    const time = parseFloat(target.value);
    if (videoElement) {
      videoElement.currentTime = time;
    }
  }

  function handleVideoSeeked() {
    updateCoverFromVideo();
  }

  function formatTime(seconds: number): string {
    const mins = Math.floor(seconds / 60);
    const secs = Math.floor(seconds % 60);
    return `${mins}:${secs.toString().padStart(2, "0")}`;
  }

  function updateCoverFromVideo() {
    if (!videoElement || !isVideoLoaded) return;

    // 确保视频已经准备好并且有有效的尺寸
    if (videoElement.videoWidth === 0 || videoElement.videoHeight === 0) {
      // 如果视频尺寸无效，等待一下再重试
      setTimeout(() => {
        if (
          videoElement &&
          videoElement.videoWidth > 0 &&
          videoElement.videoHeight > 0
        ) {
          updateCoverFromVideo();
        }
      }, 100);
      return;
    }

    try {
      const tempCanvas = document.createElement("canvas");
      tempCanvas.width = videoElement.videoWidth;
      tempCanvas.height = videoElement.videoHeight;
      const tempCtx = tempCanvas.getContext("2d");

      if (!tempCtx) {
        log.error("Failed to get canvas context");
        return;
      }

      tempCtx.drawImage(
        videoElement,
        0,
        0,
        tempCanvas.width,
        tempCanvas.height
      );
      videoFrame = tempCanvas.toDataURL("image/jpeg");
      loadBackgroundImage();
    } catch (error) {
      log.error("Failed to capture video frame:", error);
    }
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

            if (onSave) {
              await onSave(newCover);
            } else {
              await invoke("update_video_cover", {
                id: video.value,
                cover: newCover,
              });
            }

            // 触发自定义事件通知父组件更新封面
            onCoverUpdate?.({ cover: newCover });
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

  $effect(() => {
    // 当文本内容或样式改变时重绘
    if (ctx) {
      // Read the fields that affect the canvas so nested $state changes
      // invalidate this effect without writing back to the same state.
      texts.map((text) => ({
        id: text.id,
        content: text.content,
        fontSize: text.fontSize,
        color: text.color,
        strokeColor: text.strokeColor,
        position: text.position,
      }));
      scheduleRedraw();
    }
  });

  let selectedText = $derived(texts.find((t) => t.id === selectedTextId));

  // 监听 show 变化，当模态框显示时重新绘制
  $effect(() => {
    if (show && ctx) {
      setTimeout(() => {
        if (isVideoLoaded && videoElement) {
          // 如果视频已加载，更新封面
          updateCoverFromVideo();
        }
        loadBackgroundImage();
        resizeCanvas();
      }, 50);
    }
  });
</script>

<svelte:window
  onmousemove={handleMouseMove}
  onmouseup={handleMouseUp}
  onkeydown={(event) => {
    if (show && event.key === "Escape") {
      event.preventDefault();
      event.stopImmediatePropagation();
      handleClose();
    }
  }}
  onblur={() => (isDragging = false)}
  onvisibilitychange={() => {
    if (document.hidden) {
      isDragging = false;
    }
  }}
/>

<!-- Kept mounted while hidden so the canvas and video stay ready. -->
<div
  class="ui-modal-backdrop z-[1100] transition-opacity duration-200"
  class:opacity-0={!show}
  class:pointer-events-none={!show}
  inert={!show}
  data-hotkey-block
>
  <div
    class="ui-modal max-h-[92vh] w-[800px] max-w-full transition-transform duration-200"
    class:scale-95={!show}
    role="dialog"
    aria-modal="true"
    aria-labelledby="cover-editor-title"
  >
    <div class="ui-modal-header">
      <h3 id="cover-editor-title">编辑封面</h3>
      <button
        type="button"
        class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost -mr-1.5"
        aria-label="关闭"
        onclick={handleClose}
      >
        <X class="h-4 w-4" />
      </button>
    </div>

    <div class="ui-modal-body custom-scrollbar-light space-y-4">
      <!-- Hidden Video Element -->
      <!-- svelte-ignore a11y_media_has_caption -->
      <video
        bind:this={videoElement}
        src={video?.file}
        class="hidden"
        crossorigin="anonymous"
        onloadedmetadata={handleVideoLoaded}
        ontimeupdate={handleTimeUpdate}
        onseeked={handleVideoSeeked}
      ></video>

      <!-- Video Frame Selection -->
      {#if video?.file}
        <div class="space-y-2.5">
          <div class="flex items-center justify-between">
            <span class="ui-label">选择视频帧</span>
            <span class="text-[11px] tabular-nums text-fg-muted">
              {formatTime(currentTime)} / {formatTime(duration)}
            </span>
          </div>
          <input
            type="range"
            min="0"
            max={duration}
            step="0.1"
            bind:value={currentTime}
            oninput={handleSeek}
            class="w-full"
            aria-label="视频帧位置"
            disabled={!isVideoLoaded}
          />
        </div>
      {/if}

      <!-- Cover Preview -->
      <div class="space-y-2">
        <div class="flex items-baseline gap-2">
          <span class="ui-label">封面预览</span>
          <span class="ui-hint">拖拽文字调整位置</span>
        </div>
        <div
          id="cover-container"
          class="relative aspect-video overflow-hidden rounded-[10px] border border-line bg-sunken"
        >
          <canvas
            bind:this={canvas}
            onmousedown={handleMouseDown}
            class="h-full w-full"
          ></canvas>
        </div>
      </div>

      <!-- Text Controls -->
      <div class="flex items-start gap-4">
        <div class="min-w-0 flex-1 space-y-2">
          <div class="flex items-center justify-between">
            <span class="ui-label flex items-center gap-1.5">
              <Type class="h-3.5 w-3.5 text-fg-muted" />
              文字列表
            </span>
            <button type="button" class="ui-btn ui-btn-sm" onclick={addNewText}>
              <Plus class="h-3.5 w-3.5" />
              添加文字
            </button>
          </div>
          <div class="space-y-1.5">
            {#each texts as text, index (text.id)}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div
                class="flex items-start gap-2 rounded-[10px] border p-1.5 transition-colors {selectedTextId === text.id
                  ? 'border-accent/70 bg-accent-soft/40'
                  : 'border-line bg-control/50 hover:border-line-strong'}"
                onclick={() => (selectedTextId = text.id)}
              >
                <span
                  class="mt-2 w-4 shrink-0 text-right font-mono text-[10px] {selectedTextId === text.id
                    ? 'text-accent-fg'
                    : 'text-fg-faint'}">{index + 1}</span
                >
                <textarea
                  value={text.content}
                  oninput={(e) => handleTextInput(text, e)}
                  onfocus={() => (selectedTextId = text.id)}
                  placeholder="输入文字内容"
                  rows="2"
                  class="ui-input min-w-0 flex-1 resize-none"
                ></textarea>
                {#if texts.length > 1}
                  <button
                    type="button"
                    class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost hover:text-danger-fg"
                    aria-label="删除文字"
                    title="删除文字"
                    onclick={(event) => {
                      event.stopPropagation();
                      deleteText(text.id);
                    }}
                  >
                    <Trash2 class="h-3.5 w-3.5" />
                  </button>
                {/if}
              </div>
            {/each}
          </div>
        </div>

        <!-- Text Style Controls -->
        {#if selectedText}
          <div class="w-56 shrink-0 space-y-2">
            <span class="ui-label flex h-[26px] items-center gap-1.5">
              <Palette class="h-3.5 w-3.5 text-fg-muted" />
              文字样式
            </span>
            <div class="ui-card space-y-3 bg-control/50 p-3">
              <label class="block space-y-2">
                <span class="flex items-center justify-between text-[11px] text-fg-muted">
                  字体大小
                  <span class="tabular-nums text-fg-secondary">{selectedText.fontSize}</span>
                </span>
                <input
                  type="range"
                  bind:value={selectedText.fontSize}
                  min="48"
                  max="160"
                  class="w-full"
                />
              </label>
              <label class="color-field flex items-center justify-between gap-3">
                <span class="text-[11px] text-fg-muted">文字颜色</span>
                <span class="ui-input flex h-[28px] w-[112px] items-center gap-2 px-2">
                  <input type="color" bind:value={selectedText.color} />
                  <span class="font-mono text-[11px] text-fg-secondary">{selectedText.color.toUpperCase()}</span>
                </span>
              </label>
              <label class="color-field flex items-center justify-between gap-3">
                <span class="text-[11px] text-fg-muted">描边颜色</span>
                <span class="ui-input flex h-[28px] w-[112px] items-center gap-2 px-2">
                  <input type="color" bind:value={selectedText.strokeColor} />
                  <span class="font-mono text-[11px] text-fg-secondary">{selectedText.strokeColor.toUpperCase()}</span>
                </span>
              </label>
            </div>
          </div>
        {/if}
      </div>
    </div>

    <div class="ui-modal-footer">
      <button type="button" class="ui-btn" onclick={handleClose}>取消</button>
      <button type="button" class="ui-btn ui-btn-primary" onclick={handleSave}>保存封面</button>
    </div>
  </div>
</div>

<style>
  /* Matches the project editor's sliders. */
  input[type="range"] {
    -webkit-appearance: none;
    appearance: none;
    height: 4px;
    border-radius: 2px;
    background: rgb(var(--c-fg-faint) / 50%);
    cursor: pointer;
  }

  input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: white;
    box-shadow: 0 1px 3px rgb(0 0 0 / 35%);
  }

  input[type="range"]::-moz-range-thumb {
    width: 12px;
    height: 12px;
    border: none;
    border-radius: 50%;
    background: white;
  }

  input[type="range"]:focus {
    outline: none;
  }

  input[type="range"]:focus-visible {
    outline: 2px solid rgb(var(--c-accent));
    outline-offset: 4px;
  }

  input[type="range"]:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .color-field:focus-within .ui-input {
    border-color: rgb(var(--c-accent));
  }

  input[type="color"] {
    -webkit-appearance: none;
    appearance: none;
    width: 20px;
    height: 16px;
    flex-shrink: 0;
    border: none;
    padding: 0;
    background: transparent;
    cursor: pointer;
  }

  input[type="color"]::-webkit-color-swatch-wrapper {
    padding: 0;
  }

  input[type="color"]::-webkit-color-swatch {
    border: 1px solid rgb(var(--c-line-strong));
    border-radius: 4px;
  }

  input[type="color"]::-moz-color-swatch {
    border: 1px solid rgb(var(--c-line-strong));
    border-radius: 4px;
  }
</style>
