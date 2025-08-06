<script lang="ts">
  import { invoke } from "../lib/invoker";
  import { Upload, X, FileVideo } from "lucide-svelte";
  import { createEventDispatcher } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  
  export let showDialog = false;
  
  const dispatch = createEventDispatcher();
  
  let selectedFilePath: string | null = null;
  let selectedFileName: string = "";
  let selectedFileSize: number = 0;
  let videoTitle = "";
  let importing = false;
  let dragOver = false;
  
  async function handleFileSelect() {
    try {
      const selected = await open({
        multiple: false,
        filters: [{
          name: '视频文件',
          extensions: ['mp4', 'mkv', 'avi', 'mov', 'wmv', 'flv', 'm4v', 'webm']
        }]
      });
      
      if (selected && typeof selected === 'string') {
        selectedFilePath = selected;
        selectedFileName = selected.split(/[/\\]/).pop() || '';
        videoTitle = selectedFileName.replace(/\.[^/.]+$/, ""); // 去掉扩展名
        
        // 获取文件大小 (这里需要调用后端API)
        try {
          selectedFileSize = await invoke("get_file_size", { filePath: selected });
        } catch (e) {
          selectedFileSize = 0;
        }
      }
    } catch (error) {
      console.error("文件选择失败:", error);
    }
  }
  
  // 移除拖拽功能，使用文件对话框
  
  async function startImport() {
    if (!selectedFilePath) return;
    
    importing = true;
    try {
      const result = await invoke("import_external_video", {
        filePath: selectedFilePath,
        title: videoTitle,
        originalName: selectedFileName,
        size: selectedFileSize
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
    }
  }
  
  function closeDialog() {
    showDialog = false;
    selectedFilePath = null;
    selectedFileName = "";
    selectedFileSize = 0;
    videoTitle = "";
  }
</script>

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
      <div class="border-2 border-dashed border-gray-300 dark:border-gray-600 rounded-lg p-8 text-center">
        {#if selectedFilePath}
          <div class="space-y-4">
            <FileVideo class="w-12 h-12 text-green-500 mx-auto" />
            <p class="text-sm text-gray-900 dark:text-white font-medium">{selectedFileName}</p>
            <p class="text-xs text-gray-500">大小: {(selectedFileSize / 1024 / 1024).toFixed(2)} MB</p>
            <p class="text-xs text-gray-400 break-all">{selectedFilePath}</p>
          </div>
        {:else}
          <Upload class="w-12 h-12 text-gray-400 mx-auto mb-4" />
          <p class="text-sm text-gray-600 dark:text-gray-400 mb-4">
            点击按钮选择视频文件
          </p>
          <p class="text-xs text-gray-500 dark:text-gray-500">
            支持 MP4, MKV, AVI, MOV 等常见格式
          </p>
        {/if}
        
        <button
          on:click={handleFileSelect}
          class="mt-4 px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
        >
          选择文件
        </button>
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
          disabled={!selectedFilePath || importing || !videoTitle.trim()}
          class="px-4 py-2 bg-green-500 text-white rounded-lg hover:bg-green-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          {importing ? "导入中..." : "开始导入"}
        </button>
      </div>
    </div>
  </div>
</div>
{/if}