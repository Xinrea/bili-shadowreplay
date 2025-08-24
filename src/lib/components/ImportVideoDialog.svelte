<script lang="ts">
  import {
    invoke,
    TAURI_ENV,
    ENDPOINT,
    listen,
    onConnectionRestore,
  } from "../invoker";
  import { Upload, X, CheckCircle } from "lucide-svelte";
  import { createEventDispatcher, onDestroy } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import type { ProgressUpdate, ProgressFinished } from "../interface";

  export let showDialog = false;
  export let roomId: number | null = null;

  const dispatch = createEventDispatcher();

  let selectedFilePath: string | null = null;
  let selectedFileName: string = "";
  let selectedFileSize: number = 0;
  let videoTitle = "";
  let importing = false;
  let uploading = false;
  let uploadProgress = 0;
  let dragOver = false;
  let fileInput: HTMLInputElement;
  let importProgress = "";
  let currentImportEventId: string | null = null;

  // 批量导入状态
  let selectedFiles: string[] = [];
  let batchImporting = false;
  let currentFileIndex = 0;
  let totalFiles = 0;

  // 获取当前正在处理的文件名（从文件路径中提取文件名）
  $: currentFileName =
    currentFileIndex > 0 && selectedFiles.length > 0
      ? selectedFiles[currentFileIndex - 1]?.split(/[/\\]/).pop() || "未知文件"
      : "";

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

  // 进度监听器
  const progressUpdateListener = listen<ProgressUpdate>(
    "progress-update",
    (e) => {
      if (e.payload.id === currentImportEventId) {
        importProgress = e.payload.content;

        // 从进度文本中提取当前文件索引
        const match = importProgress.match(/正在导入第(\d+)个/);
        if (match) {
          currentFileIndex = parseInt(match[1]);
        }
      }
    }
  );

  const progressFinishedListener = listen<ProgressFinished>(
    "progress-finished",
    (e) => {
      if (e.payload.id === currentImportEventId) {
        if (e.payload.success) {
          // 导入成功，关闭对话框并刷新列表
          showDialog = false;
          selectedFilePath = null;
          selectedFileName = "";
          selectedFileSize = 0;
          videoTitle = "";
          resetBatchImportState();
          dispatch("imported");
        } else {
          alert("导入失败: " + e.payload.message);
          resetBatchImportState();
        }
        // 无论成功失败都要重置状态
        importing = false;
        currentImportEventId = null;
        importProgress = "";
      }
    }
  );

  // 连接恢复时检查任务状态
  async function checkTaskStatus() {
    if (!currentImportEventId || !importing) return;

    try {
      const progress = await invoke("get_import_progress");
      if (!progress) {
        importing = false;
        currentImportEventId = null;
        importProgress = "";
        resetBatchImportState();
        dispatch("imported");
      }
    } catch (error) {
      console.error(`[ImportDialog] Failed to check task status:`, error);
    }
  }

  // 注册连接恢复回调
  if (!TAURI_ENV) {
    onConnectionRestore(checkTaskStatus);
  }

  onDestroy(() => {
    progressUpdateListener?.then((fn) => fn());
    progressFinishedListener?.then((fn) => fn());
  });

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
          "请选择支持的视频文件格式 (MP4, MKV, AVI, MOV, WMV, FLV, M4V, WebM)"
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
      formData.append("roomId", String(roomId || 0));

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
      formData.append("room_id", String(roomId || 0));

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
            totalFiles - 1
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
            dispatch("imported");
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
      currentImportEventId = eventId;

      await invoke("batch_import_external_videos", {
        eventId: eventId,
        filePaths: selectedFiles,
        roomId: roomId || 0,
      });

      // 注意：成功处理在 progressFinishedListener 中进行
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
    currentImportEventId = null;
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
      currentImportEventId = eventId;

      await invoke("import_external_video", {
        eventId: eventId,
        filePath: selectedFilePath,
        title: videoTitle,
        originalName: selectedFileName,
        size: selectedFileSize,
        roomId: roomId || 0,
      });

      // 注意：成功处理移到了progressFinishedListener中
    } catch (error) {
      console.error("导入失败:", error);
      alert("导入失败: " + error);
      importing = false;
      currentImportEventId = null;
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
    currentImportEventId = null;
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
    on:change={handleFileInputChange}
  />
{/if}

{#if showDialog}
  <div
    class="fixed inset-0 bg-black/20 dark:bg-black/40 backdrop-blur-sm z-50 flex items-center justify-center p-4"
  >
    <div
      class="bg-white dark:bg-[#323234] rounded-xl shadow-xl w-full max-w-[600px] max-h-[90vh] overflow-hidden flex flex-col"
    >
      <div class="flex-1 overflow-y-auto">
        <div class="p-6 space-y-4">
          <div class="flex justify-between items-center">
            <h3 class="text-lg font-medium text-gray-900 dark:text-white">
              导入外部视频
            </h3>
            <button
              on:click={closeDialog}
              class="text-gray-400 hover:text-gray-600"
            >
              <X class="w-5 h-5" />
            </button>
          </div>

          <!-- 文件选择区域 -->
          <div
            class="border-2 border-dashed rounded-lg p-8 text-center transition-colors {dragOver
              ? 'border-blue-400 bg-blue-50 dark:bg-blue-900/20'
              : 'border-gray-300 dark:border-gray-600'}"
            on:dragover={handleDragOver}
            on:dragleave={handleDragLeave}
            on:drop={handleDrop}
          >
            {#if uploading}
              <!-- 上传进度 -->
              <div class="space-y-4">
                <Upload
                  class="w-12 h-12 text-blue-500 mx-auto animate-bounce"
                />
                <p class="text-sm text-gray-900 dark:text-white font-medium">
                  上传中...
                </p>
                <div
                  class="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-2"
                >
                  <div
                    class="bg-blue-500 h-2 rounded-full transition-all"
                    style="width: {uploadProgress}%"
                  ></div>
                </div>
                <p class="text-xs text-gray-500">{uploadProgress}%</p>
              </div>
            {:else if batchImporting}
              <!-- 批量导入中 -->
              <div class="space-y-4">
                <div class="flex items-center justify-center">
                  <svg
                    class="animate-spin h-12 w-12 text-blue-500"
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
                <p class="text-sm text-gray-900 dark:text-white font-medium">
                  批量导入进行中...
                </p>
                <div class="text-xs text-gray-500">{importProgress}</div>
                {#if currentFileName}
                  <div class="text-xs text-gray-400 break-all">
                    当前文件：{currentFileName}
                  </div>
                {/if}
              </div>
            {:else if selectedFilePath}
              <!-- 已选择文件 -->
              <div class="space-y-4">
                <div class="flex items-center justify-center">
                  <CheckCircle class="w-12 h-12 text-green-500 mx-auto" />
                </div>
                <p class="text-sm text-gray-900 dark:text-white font-medium">
                  {selectedFileName}
                </p>
                <p class="text-xs text-gray-500">
                  大小: {formatFileSize(selectedFileSize)}
                </p>
                <p
                  class="text-xs text-gray-400 break-all"
                  title={selectedFilePath}
                >
                  {selectedFilePath}
                </p>
                <button
                  on:click={() => {
                    selectedFilePath = null;
                    selectedFileName = "";
                    selectedFileSize = 0;
                    videoTitle = "";
                  }}
                  class="text-sm text-red-500 hover:text-red-700"
                >
                  重新选择
                </button>
              </div>
            {:else}
              <!-- 选择文件提示 -->
              <div class="space-y-4">
                <Upload class="w-12 h-12 text-gray-400 mx-auto" />
                {#if TAURI_ENV}
                  <p class="text-sm text-gray-600 dark:text-gray-400">
                    点击按钮选择视频文件（支持多选）
                  </p>
                {:else}
                  <p class="text-sm text-gray-600 dark:text-gray-400">
                    拖拽视频文件到此处，或点击按钮选择文件（支持多选）
                  </p>
                {/if}
                <p class="text-xs text-gray-500 dark:text-gray-500">
                  支持 MP4, MKV, AVI, MOV, WMV, FLV, M4V, WebM 格式
                </p>
              </div>
            {/if}

            {#if !uploading && !selectedFilePath && !batchImporting}
              <button
                on:click={handleFileSelect}
                class="mt-4 px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
              >
                {TAURI_ENV ? "选择文件" : "选择或拖拽文件"}
              </button>
            {/if}
          </div>

          <!-- 视频信息编辑 -->
          {#if selectedFilePath}
            <div class="space-y-4">
              <div>
                <label
                  for="video-title-input"
                  class="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2"
                >
                  视频标题
                </label>
                <input
                  id="video-title-input"
                  type="text"
                  bind:value={videoTitle}
                  class="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500 dark:focus:ring-blue-400"
                  placeholder="输入视频标题"
                />
              </div>
            </div>
          {/if}
        </div>
      </div>

      <!-- 操作按钮 - 固定在底部 -->
      <div
        class="border-t border-gray-200 dark:border-gray-700 p-4 bg-gray-50 dark:bg-[#2a2a2c]"
      >
        <div class="flex justify-end space-x-3">
          <button
            on:click={closeDialog}
            class="px-4 py-2 text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-600 rounded-lg transition-colors"
          >
            取消
          </button>
          <button
            on:click={startImport}
            disabled={!selectedFilePath ||
              importing ||
              !videoTitle.trim() ||
              uploading ||
              batchImporting}
            class="px-4 py-2 bg-green-500 text-white rounded-lg hover:bg-green-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors flex items-center space-x-2"
          >
            {#if importing}
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
            {/if}
            <span>{importing ? importProgress || "导入中..." : "开始导入"}</span
            >
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}
