<script lang="ts">
  import { invoke, TAURI_ENV, ENDPOINT, listen } from "../invoker";
  import { Upload, X, CheckCircle } from "lucide-svelte";
  import { onDestroy } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import type { ProgressUpdate, ProgressFinished } from "../interface";

  interface Props {
    showDialog?: boolean;
    roomId?: string | null;
    onImported?: () => void;
  }

  let { showDialog = $bindable(false), roomId = null, onImported }: Props = $props();
  const IMPORTED_VIDEO_ROOM = "bsr:import";

  let selectedFilePath: string | null = $state(null);
  let selectedFileName: string = $state("");
  let selectedFileSize: number = $state(0);
  let videoTitle = $state("");
  let importing = $state(false);
  let uploading = $state(false);
  let uploadProgress = $state(0);
  let dragOver = $state(false);
  let fileInput: HTMLInputElement = $state();
  let importProgress = $state("");

  // 批量导入状态
  let selectedFiles: string[] = $state([]);
  let batchImporting = $state(false);
  let currentFileIndex = $state(0);
  let totalFiles = 0;

  // 获取当前正在处理的文件名（从文件路径中提取文件名）
  let currentFileName =
    $derived(currentFileIndex > 0 && selectedFiles.length > 0
      ? selectedFiles[currentFileIndex - 1]?.split(/[/\\]/).pop() || "未知文件"
      : "");

  // 格式化文件大小
  function formatFileSize(sizeInBytes: number): string {
    if (sizeInBytes === 0) return "0 B";

    const units = ["B", "KB", "MB", "GB", "TB"];
    const k = 1024;
    let unitIndex = 0;
    let size = sizeInBytes;

    // 找到合适的单位
    while (size >= k && unitIndex < units.length - 1) {
      size /= k;
      unitIndex++;
    }

    // 对于GB以上，显示2位小数；MB显示2位小数；KB及以下显示1位小数
    const decimals = unitIndex >= 3 ? 2 : unitIndex >= 2 ? 2 : 1;

    return size.toFixed(decimals) + " " + units[unitIndex];
  }

  async function handleFileSelect() {
    if (TAURI_ENV) {
      // Tauri模式：使用文件对话框，支持多选
      try {
        const selected = await open({
          multiple: true,
          filters: [
            {
              name: "视频文件",
              extensions: [
                "mp4",
                "mkv",
                "avi",
                "mov",
                "wmv",
                "flv",
                "m4v",
                "webm",
              ],
            },
          ],
        });

        // 检查用户是否取消了选择
        if (!selected) return;

        if (Array.isArray(selected) && selected.length > 1) {
          // 批量导入：多个文件
          selectedFiles = selected;
          await startBatchImport();
        } else if (Array.isArray(selected) && selected.length === 1) {
          // 单文件导入：数组中的单个文件
          await setSelectedFile(selected[0]);
        } else if (typeof selected === "string") {
          // 单文件导入：直接返回字符串路径
          await setSelectedFile(selected);
        }
      } catch (error) {
        console.error("文件选择失败:", error);
        alert("文件选择失败: " + error);
      }
    } else {
      // Web模式：触发文件输入
      fileInput?.click();
    }
  }

  async function handleFileInputChange(event: Event) {
    const target = event.target as HTMLInputElement;
    const files = target.files;
    if (files && files.length > 0) {
      if (files.length > 1) {
        // 批量上传模式
        await uploadAndImportMultipleFiles(Array.from(files));
      } else {
        // 单文件上传模式（保持现有逻辑）
        const file = files[0];
        // 提前设置文件信息，提升用户体验
        selectedFileName = file.name;
        videoTitle = file.name.replace(/\.[^/.]+$/, ""); // 去掉扩展名
        selectedFileSize = file.size;

        await uploadFile(file);
      }
    }
  }

  async function handleDrop(event: DragEvent) {
    event.preventDefault();
    dragOver = false;

    if (TAURI_ENV) return; // Tauri模式不支持拖拽

    const files = event.dataTransfer?.files;
    if (files && files.length > 0) {
      const file = files[0];
      // 检查文件类型
      const allowedTypes = [
        "video/mp4",
        "video/x-msvideo",
        "video/quicktime",
        "video/x-ms-wmv",
        "video/x-flv",
        "video/x-m4v",
        "video/webm",
        "video/x-matroska",
      ];
      if (
        allowedTypes.includes(file.type) ||
        file.name.match(/\.(mp4|mkv|avi|mov|wmv|flv|m4v|webm)$/i)
      ) {
        // 提前设置文件信息，提升用户体验
        selectedFileName = file.name;
        videoTitle = file.name.replace(/\.[^/.]+$/, ""); // 去掉扩展名
        selectedFileSize = file.size;

        await uploadFile(file);
      } else {
        alert(
          "请选择支持的视频文件格式 (MP4, MKV, AVI, MOV, WMV, FLV, M4V, WebM)",
        );
      }
    }
  }

  async function uploadFile(file: File) {
    uploading = true;
    uploadProgress = 0;

    try {
      const formData = new FormData();
      formData.append("file", file);
      formData.append("roomId", roomId || IMPORTED_VIDEO_ROOM);

      const xhr = new XMLHttpRequest();

      // 监听上传进度
      xhr.upload.addEventListener("progress", (e) => {
        if (e.lengthComputable) {
          uploadProgress = Math.round((e.loaded / e.total) * 100);
        }
      });

      // 处理上传完成
      xhr.addEventListener("load", async () => {
        if (xhr.status === 200) {
          const response = JSON.parse(xhr.responseText);

          if (response.code === 0 && response.data) {
            // 使用本地文件信息，更快更准确
            await setSelectedFile(response.data.filePath, file.size);
          } else {
            throw new Error(response.message || "上传失败");
          }
        } else {
          throw new Error(`上传失败: HTTP ${xhr.status}`);
        }
        uploading = false;
      });

      xhr.addEventListener("error", () => {
        alert("上传失败：网络错误");
        uploading = false;
      });

      xhr.open("POST", `${ENDPOINT}/api/upload_file`);
      xhr.send(formData);
    } catch (error) {
      console.error("上传失败:", error);
      alert("上传失败: " + error);
      uploading = false;
    }
  }

  async function uploadAndImportMultipleFiles(files: File[]) {
    batchImporting = true;
    importing = true;
    totalFiles = files.length;
    currentFileIndex = 0;
    importProgress = `准备批量上传和导入 ${totalFiles} 个文件...`;

    // 设置当前处理的文件名列表
    const fileNames = files.map((file) => file.name);

    try {
      // 验证所有文件格式
      const allowedTypes = [
        "video/mp4",
        "video/x-msvideo",
        "video/quicktime",
        "video/x-ms-wmv",
        "video/x-flv",
        "video/x-m4v",
        "video/webm",
        "video/x-matroska",
      ];
      for (const file of files) {
        if (
          !allowedTypes.includes(file.type) &&
          !file.name.match(/\.(mp4|mkv|avi|mov|wmv|flv|m4v|webm)$/i)
        ) {
          throw new Error(`不支持的文件格式: ${file.name}`);
        }
      }

      const formData = new FormData();
      formData.append("room_id", roomId || IMPORTED_VIDEO_ROOM);

      files.forEach((file) => {
        formData.append("files", file);
      });

      const xhr = new XMLHttpRequest();

      // 监听上传进度
      xhr.upload.addEventListener("progress", (e) => {
        if (e.lengthComputable) {
          const progress = Math.round((e.loaded / e.total) * 100);
          importProgress = `批量上传进度: ${progress}%`;

          // 根据进度估算当前正在上传的文件
          const estimatedCurrentIndex = Math.min(
            Math.floor((progress / 100) * totalFiles),
            totalFiles - 1,
          );
          currentFileName = fileNames[estimatedCurrentIndex] || fileNames[0];
        }
      });

      // 处理上传完成
      xhr.addEventListener("load", () => {
        if (xhr.status === 200) {
          const response = JSON.parse(xhr.responseText);
          if (response.code === 0) {
            // 批量上传和导入成功，关闭对话框并刷新列表
            showDialog = false;
            selectedFilePath = null;
            selectedFileName = "";
            selectedFileSize = 0;
            videoTitle = "";
            resetBatchImportState();
            onImported?.();
          } else {
            throw new Error(response.message || "批量导入失败");
          }
        } else {
          throw new Error(`批量上传失败: HTTP ${xhr.status}`);
        }
      });

      xhr.addEventListener("error", () => {
        alert("批量上传失败：网络错误");
        resetBatchImportState();
      });

      xhr.open("POST", `${ENDPOINT}/api/upload_and_import_files`);
      xhr.send(formData);
    } catch (error) {
      console.error("批量上传失败:", error);
      alert("批量上传失败: " + error);
      resetBatchImportState();
    }
  }

  async function setSelectedFile(filePath: string, fileSize?: number) {
    selectedFilePath = filePath;
    selectedFileName = filePath.split(/[/\\]/).pop() || "";
    videoTitle = selectedFileName.replace(/\.[^/.]+$/, ""); // 去掉扩展名

    if (fileSize !== undefined) {
      selectedFileSize = fileSize;
    } else {
      // 获取文件大小 (Tauri模式)
      try {
        selectedFileSize = await invoke("get_file_size", { filePath });
      } catch (e) {
        selectedFileSize = 0;
      }
    }
  }

  /**
   * 开始批量导入视频文件
   */
  async function startBatchImport() {
    if (selectedFiles.length === 0) return;

    batchImporting = true;
    importing = true;
    totalFiles = selectedFiles.length;
    currentFileIndex = 0;
    importProgress = `准备批量导入 ${totalFiles} 个文件...`;

    try {
      const eventId = "batch_import_" + Date.now();

      const clear_update_listener = await listen(
        `progress-update:${eventId}`,
        (e) => {
          importProgress = e.payload.content;

          // 从进度文本中提取当前文件索引
          const match = importProgress.match(/正在导入第(\d+)个/);
          if (match) {
            currentFileIndex = parseInt(match[1]);
          }
        },
      );
      const clear_finished_listener = await listen(
        `progress-finished:${eventId}`,
        (e) => {
          if (e.payload.success) {
            // 导入成功，关闭对话框并刷新列表
            showDialog = false;
            selectedFilePath = null;
            selectedFileName = "";
            selectedFileSize = 0;
            videoTitle = "";
            resetBatchImportState();
            onImported?.();
          } else {
            alert("导入失败: " + e.payload.message);
            resetBatchImportState();
          }
          // 无论成功失败都要重置状态
          importing = false;
          importProgress = "";

          clear_update_listener();
          clear_finished_listener();
        },
      );

      await invoke("batch_import_external_videos", {
        eventId: eventId,
        filePaths: selectedFiles,
        roomId: roomId || IMPORTED_VIDEO_ROOM,
      });
    } catch (error) {
      console.error("批量导入失败:", error);
      alert("批量导入失败: " + error);
      resetBatchImportState();
    }
  }

  /**
   * 重置批量导入状态
   */
  function resetBatchImportState() {
    batchImporting = false;
    importing = false;
    importProgress = "";
    selectedFiles = [];
    totalFiles = 0;
    currentFileIndex = 0;
  }

  async function startImport() {
    if (!selectedFilePath) return;

    importing = true;
    importProgress = "准备导入...";

    try {
      const eventId = "import_" + Date.now();

      const clear_update_listener = await listen(
        `progress-update:${eventId}`,
        (e) => {
          importProgress = e.payload.content;

          // 从进度文本中提取当前文件索引
          const match = importProgress.match(/正在导入第(\d+)个/);
          if (match) {
            currentFileIndex = parseInt(match[1]);
          }
        },
      );
      const clear_finished_listener = await listen(
        `progress-finished:${eventId}`,
        (e) => {
          if (e.payload.success) {
            // 导入成功，关闭对话框并刷新列表
            showDialog = false;
            selectedFilePath = null;
            selectedFileName = "";
            selectedFileSize = 0;
            videoTitle = "";
            resetBatchImportState();
            onImported?.();
          } else {
            alert("导入失败: " + e.payload.message);
            resetBatchImportState();
          }
          // 无论成功失败都要重置状态
          importing = false;
          importProgress = "";

          clear_update_listener();
          clear_finished_listener();
        },
      );

      await invoke("import_external_video", {
        eventId: eventId,
        filePath: selectedFilePath,
        title: videoTitle,
        roomId: roomId || IMPORTED_VIDEO_ROOM,
      });

      // 注意：成功处理移到了progressFinishedListener中
    } catch (error) {
      console.error("导入失败:", error);
      alert("导入失败: " + error);
      importing = false;
      importProgress = "";
    }
  }

  /**
   * 关闭对话框并重置所有状态
   */
  function closeDialog() {
    showDialog = false;
    // 重置单文件导入状态
    selectedFilePath = null;
    selectedFileName = "";
    selectedFileSize = 0;
    videoTitle = "";
    uploading = false;
    uploadProgress = 0;
    importing = false;
    importProgress = "";
    // 重置批量导入状态
    resetBatchImportState();
  }

  function handleDragOver(event: DragEvent) {
    event.preventDefault();
    if (!TAURI_ENV) {
      dragOver = true;
    }
  }

  function handleDragLeave() {
    dragOver = false;
  }
</script>

<!-- 隐藏的文件输入 -->
{#if !TAURI_ENV}
  <input
    bind:this={fileInput}
    type="file"
    accept=".mp4,.mkv,.avi,.mov,.wmv,.flv,.m4v,.webm,video/*"
    multiple
    style="display: none"
    onchange={handleFileInputChange}
  />
{/if}

{#if showDialog}
  <div class="ui-modal-backdrop">
    <div class="ui-modal max-h-[90vh] w-full max-w-[600px]">
      <div class="ui-modal-header">
        <h3 class="text-[14px] font-semibold text-fg">导入外部视频</h3>
        <button
          onclick={closeDialog}
          class="ui-btn ui-btn-sm ui-btn-icon ui-btn-ghost -mr-1.5"
        >
          <X class="h-4 w-4" />
        </button>
      </div>

      <div class="ui-modal-body space-y-4">
        <!-- 文件选择区域 -->
        <div
          class="rounded-[10px] border border-dashed p-8 text-center transition-colors {dragOver
            ? 'border-accent bg-accent-soft'
            : 'border-line-strong bg-control/40'}"
          role="presentation"
          ondragover={handleDragOver}
          ondragleave={handleDragLeave}
          ondrop={handleDrop}
        >
          {#if uploading}
            <!-- 上传进度 -->
            <div class="space-y-3">
              <Upload class="mx-auto h-10 w-10 animate-bounce text-accent" />
              <p class="text-[13px] font-medium text-fg">上传中...</p>
              <div class="h-1.5 w-full overflow-hidden rounded-full bg-sunken">
                <div
                  class="h-1.5 rounded-full bg-accent transition-all"
                  style="width: {uploadProgress}%"
                ></div>
              </div>
              <p class="text-[11px] tabular-nums text-fg-muted">
                {uploadProgress}%
              </p>
            </div>
          {:else if batchImporting}
            <!-- 批量导入中 -->
            <div class="space-y-3">
              <div class="flex items-center justify-center">
                <svg
                  class="h-10 w-10 animate-spin text-accent"
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
              </div>
              <p class="text-[13px] font-medium text-fg">批量导入进行中...</p>
              <div class="text-[12px] text-fg-muted">{importProgress}</div>
              {#if currentFileName}
                <div class="break-all text-[11px] text-fg-faint">
                  当前文件：{currentFileName}
                </div>
              {/if}
            </div>
          {:else if selectedFilePath}
            <!-- 已选择文件 -->
            <div class="space-y-2">
              <div class="flex items-center justify-center pb-1">
                <CheckCircle class="mx-auto h-10 w-10 text-success" />
              </div>
              <p class="break-all text-[13px] font-medium text-fg">
                {selectedFileName}
              </p>
              <p class="text-[12px] text-fg-muted">
                大小: {formatFileSize(selectedFileSize)}
              </p>
              <p
                class="break-all text-[11px] text-fg-faint"
                title={selectedFilePath}
              >
                {selectedFilePath}
              </p>
              <button
                onclick={() => {
                  selectedFilePath = null;
                  selectedFileName = "";
                  selectedFileSize = 0;
                  videoTitle = "";
                }}
                class="mt-1 text-[12px] text-danger-fg hover:underline"
              >
                重新选择
              </button>
            </div>
          {:else}
            <!-- 选择文件提示 -->
            <div class="space-y-2">
              <Upload class="mx-auto mb-1 h-10 w-10 text-fg-faint" />
              {#if TAURI_ENV}
                <p class="text-[13px] text-fg-secondary">
                  点击按钮选择视频文件（支持多选）
                </p>
              {:else}
                <p class="text-[13px] text-fg-secondary">
                  拖拽视频文件到此处，或点击按钮选择文件（支持多选）
                </p>
              {/if}
              <p class="text-[11px] text-fg-muted">
                支持 MP4, MKV, AVI, MOV, WMV, FLV, M4V, WebM 格式
              </p>
            </div>
          {/if}

          {#if !uploading && !selectedFilePath && !batchImporting}
            <button
              onclick={handleFileSelect}
              class="ui-btn ui-btn-primary mt-4"
            >
              {TAURI_ENV ? "选择文件" : "选择或拖拽文件"}
            </button>
          {/if}
        </div>

        <!-- 视频信息编辑 -->
        {#if selectedFilePath}
          <div class="space-y-4">
            <div>
              <label for="video-title-input" class="ui-label mb-1.5 block">
                视频标题
              </label>
              <input
                id="video-title-input"
                type="text"
                bind:value={videoTitle}
                class="ui-input w-full"
                placeholder="输入视频标题"
              />
            </div>
          </div>
        {/if}
      </div>

      <!-- 操作按钮 - 固定在底部 -->
      <div class="ui-modal-footer">
        <button onclick={closeDialog} class="ui-btn">取消</button>
        <button
          onclick={startImport}
          disabled={!selectedFilePath ||
            importing ||
            !videoTitle.trim() ||
            uploading ||
            batchImporting}
          class="inline-flex h-[30px] items-center gap-1.5 whitespace-nowrap rounded-[8px] bg-success px-[11px] text-[12px] font-medium text-white transition-colors hover:bg-success/90 disabled:cursor-not-allowed disabled:opacity-50 disabled:hover:bg-success"
        >
          {#if importing}
            <svg
              class="h-3.5 w-3.5 animate-spin"
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
          {/if}
          <span>{importing ? importProgress || "导入中..." : "开始导入"}</span>
        </button>
      </div>
    </div>
  </div>
{/if}
