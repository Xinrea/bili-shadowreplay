<script lang="ts">
  import { invoke, TAURI_ENV, ENDPOINT } from "../lib/invoker";
  import { Upload, X, CheckCircle } from "lucide-svelte";
  import { createEventDispatcher } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  
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
  
  async function handleFileSelect() {
    if (TAURI_ENV) {
      // Tauri模式：使用文件对话框
      try {
        const selected = await open({
          multiple: false,
          filters: [{
            name: '视频文件',
            extensions: ['mp4', 'mkv', 'avi', 'mov', 'wmv', 'flv', 'm4v', 'webm']
          }]
        });
        
        if (selected && typeof selected === 'string') {
          await setSelectedFile(selected, 0);
        }
      } catch (error) {
        console.error("文件选择失败:", error);
      }
    } else {
      // Web模式：触发文件输入
      fileInput?.click();
    }
  }
  
  async function handleFileInputChange(event: Event) {
    const target = event.target as HTMLInputElement;
    const file = target.files?.[0];
    if (file) {
      // 提前设置文件信息，提升用户体验
      selectedFileName = file.name;
      videoTitle = file.name.replace(/\.[^/.]+$/, ""); // 去掉扩展名
      selectedFileSize = file.size;
      
      await uploadFile(file);
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
      const allowedTypes = ['video/mp4', 'video/x-msvideo', 'video/quicktime', 'video/x-ms-wmv', 'video/x-flv', 'video/x-m4v', 'video/webm', 'video/x-matroska'];
      if (allowedTypes.includes(file.type) || file.name.match(/\.(mp4|mkv|avi|mov|wmv|flv|m4v|webm)$/i)) {
        // 提前设置文件信息，提升用户体验
        selectedFileName = file.name;
        videoTitle = file.name.replace(/\.[^/.]+$/, ""); // 去掉扩展名
        selectedFileSize = file.size;
        
        await uploadFile(file);
      } else {
        alert("请选择支持的视频文件格式 (MP4, MKV, AVI, MOV, WMV, FLV, M4V, WebM)");
      }
    }
  }
  
  async function uploadFile(file: File) {
    uploading = true;
    uploadProgress = 0;
    
    try {
      const formData = new FormData();
      formData.append('file', file);
      formData.append('roomId', String(roomId || 0));
      
      const xhr = new XMLHttpRequest();
      
      // 监听上传进度
      xhr.upload.addEventListener('progress', (e) => {
        if (e.lengthComputable) {
          uploadProgress = Math.round((e.loaded / e.total) * 100);
        }
      });
      
      // 处理上传完成
      xhr.addEventListener('load', async () => {
        if (xhr.status === 200) {
          const response = JSON.parse(xhr.responseText);

          if (response.code === 0 && response.data) {
            // 使用本地文件信息，更快更准确
            await setSelectedFile(response.data.filePath, file.size);
          } else {
            throw new Error(response.message || '上传失败');
          }
        } else {
          throw new Error(`上传失败: HTTP ${xhr.status}`);
        }
        uploading = false;
      });
      
      xhr.addEventListener('error', () => {
        alert("上传失败：网络错误");
        uploading = false;
      });
      
      xhr.open('POST', `${ENDPOINT}/api/upload_file`);
      xhr.send(formData);
      
    } catch (error) {
      console.error("上传失败:", error);
      alert("上传失败: " + error);
      uploading = false;
    }
  }
  
  async function setSelectedFile(filePath: string, fileSize?: number) {
    selectedFilePath = filePath;
    selectedFileName = filePath.split(/[/\\]/).pop() || '';
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
  
  async function startImport() {
    if (!selectedFilePath) return;
    
    importing = true;
    try {
      const eventId = "import_" + Date.now();
      await invoke("import_external_video", {
        eventId: eventId,
        filePath: selectedFilePath,
        title: videoTitle,
        originalName: selectedFileName,
        size: selectedFileSize,
        roomId: roomId || 0
      });
      
      // 导入成功，关闭对话框并刷新列表
      showDialog = false;
      selectedFilePath = null;
      selectedFileName = "";
      selectedFileSize = 0;
      videoTitle = "";
      dispatch("imported");
    } catch (error) {
      console.error("导入失败:", error);
      alert("导入失败: " + error);
    } finally {
      importing = false;
      uploading = false;
      uploadProgress = 0;
    }
  }
  
  function closeDialog() {
    showDialog = false;
    selectedFilePath = null;
    selectedFileName = "";
    selectedFileSize = 0;
    videoTitle = "";
    uploading = false;
    uploadProgress = 0;
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
  accept="video/*"
  style="display: none"
  on:change={handleFileInputChange}
/>
{/if}

{#if showDialog}
<div class="fixed inset-0 bg-black/20 dark:bg-black/40 backdrop-blur-sm z-50 flex items-center justify-center p-4">
  <div class="bg-white dark:bg-[#323234] rounded-xl shadow-xl w-full max-w-[600px] max-h-[90vh] overflow-hidden flex flex-col">
    <div class="flex-1 overflow-y-auto">
      <div class="p-6 space-y-4">
      <div class="flex justify-between items-center">
        <h3 class="text-lg font-medium text-gray-900 dark:text-white">导入外部视频</h3>
        <button on:click={closeDialog} class="text-gray-400 hover:text-gray-600">
          <X class="w-5 h-5" />
        </button>
      </div>
      
      <!-- 文件选择区域 -->
      <div 
        class="border-2 border-dashed rounded-lg p-8 text-center transition-colors {
          dragOver ? 'border-blue-400 bg-blue-50 dark:bg-blue-900/20' : 
          'border-gray-300 dark:border-gray-600'
        }"
        on:dragover={handleDragOver}
        on:dragleave={handleDragLeave}
        on:drop={handleDrop}
      >
        {#if uploading}
          <!-- 上传进度 -->
          <div class="space-y-4">
            <Upload class="w-12 h-12 text-blue-500 mx-auto animate-bounce" />
            <p class="text-sm text-gray-900 dark:text-white font-medium">上传中...</p>
            <div class="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-2">
              <div class="bg-blue-500 h-2 rounded-full transition-all" style="width: {uploadProgress}%"></div>
            </div>
            <p class="text-xs text-gray-500">{uploadProgress}%</p>
          </div>
        {:else if selectedFilePath}
          <!-- 已选择文件 -->
          <div class="space-y-4">
            <div class="flex items-center justify-center">
              <CheckCircle class="w-12 h-12 text-green-500 mx-auto" />
            </div>
            <p class="text-sm text-gray-900 dark:text-white font-medium">{selectedFileName}</p>
            <p class="text-xs text-gray-500">大小: {(selectedFileSize / 1024 / 1024).toFixed(2)} MB</p>
            <p class="text-xs text-gray-400 break-all" title={selectedFilePath}>{selectedFilePath}</p>
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
                点击按钮选择视频文件
              </p>
            {:else}
              <p class="text-sm text-gray-600 dark:text-gray-400">
                拖拽视频文件到此处，或点击按钮选择文件
              </p>
            {/if}
            <p class="text-xs text-gray-500 dark:text-gray-500">
              支持 MP4, MKV, AVI, MOV, WMV, FLV, M4V, WebM 格式
            </p>
          </div>
        {/if}
        
        {#if !uploading && !selectedFilePath}
          <button
            on:click={handleFileSelect}
            class="mt-4 px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
          >
            {TAURI_ENV ? '选择文件' : '选择或拖拽文件'}
          </button>
        {/if}
      </div>
      
      <!-- 视频信息编辑 -->
      {#if selectedFilePath}
      <div class="space-y-4">
        <div>
          <label for="video-title-input" class="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
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
    <div class="border-t border-gray-200 dark:border-gray-700 p-4 bg-gray-50 dark:bg-[#2a2a2c]">
      <div class="flex justify-end space-x-3">
        <button
          on:click={closeDialog}
          class="px-4 py-2 text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-600 rounded-lg transition-colors"
        >
          取消
        </button>
        <button
          on:click={startImport}
          disabled={!selectedFilePath || importing || !videoTitle.trim() || uploading}
          class="px-4 py-2 bg-green-500 text-white rounded-lg hover:bg-green-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          {importing ? "导入中..." : "开始导入"}
        </button>
      </div>
    </div>
  </div>
</div>
{/if}