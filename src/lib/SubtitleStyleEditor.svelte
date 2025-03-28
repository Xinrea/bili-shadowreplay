<script lang="ts">
  import { X } from "lucide-svelte";
  import { parseSubtitleStyle, type SubtitleStyle } from "./interface";

  export let show = false;
  export let onClose: () => void;
  export let roomId: number;

  // 默认样式
  const defaultStyle: SubtitleStyle = {
    fontName: "Arial",
    fontSize: 24,
    fontColor: "#FFFFFF",
    outlineColor: "#000000",
    outlineWidth: 2,
    alignment: 2,
    marginV: 20,
    marginL: 20,
    marginR: 20,
  };

  // 从 localStorage 加载样式，如果没有则使用默认值
  let style: SubtitleStyle = (() => {
    const savedStyle = localStorage.getItem(`subtitle_style_${roomId}`);
    return savedStyle ? JSON.parse(savedStyle) : defaultStyle;
  })();

  // 保存样式到 localStorage
  function saveStyle() {
    localStorage.setItem(`subtitle_style_${roomId}`, JSON.stringify(style));
  }

  // 生成 ffmpeg 样式参数
  $: styleString = parseSubtitleStyle(style);

  // 预览样式
  $: previewStyle = (() => {
    // 将颜色值转换为 rgba 格式
    const fontColor = style.fontColor.startsWith("#")
      ? `rgba(${parseInt(style.fontColor.slice(1, 3), 16)}, ${parseInt(style.fontColor.slice(3, 5), 16)}, ${parseInt(style.fontColor.slice(5, 7), 16)}, 1)`
      : style.fontColor;

    const outlineColor = style.outlineColor.startsWith("#")
      ? `rgba(${parseInt(style.outlineColor.slice(1, 3), 16)}, ${parseInt(style.outlineColor.slice(3, 5), 16)}, ${parseInt(style.outlineColor.slice(5, 7), 16)}, 1)`
      : style.outlineColor;

    return `
      font-family: ${style.fontName};
      font-size: ${style.fontSize}px;
      color: ${fontColor};
      text-shadow: 
        ${style.outlineWidth}px ${style.outlineWidth}px 0 ${outlineColor},
        -${style.outlineWidth}px ${style.outlineWidth}px 0 ${outlineColor},
        ${style.outlineWidth}px -${style.outlineWidth}px 0 ${outlineColor},
        -${style.outlineWidth}px -${style.outlineWidth}px 0 ${outlineColor};
      padding: 4px 8px;
      border-radius: 4px;
      margin: ${style.marginV}px ${style.marginR}px ${style.marginV}px ${style.marginL}px;
      display: inline-block;
      white-space: nowrap;
    `;
  })();

  // 对齐方式选项
  const alignmentOptions = [
    { value: 1, label: "左下", description: "字幕显示在视频左下角" },
    { value: 2, label: "底部居中", description: "字幕显示在视频底部中间" },
    { value: 3, label: "右下", description: "字幕显示在视频右下角" },
    { value: 5, label: "左上", description: "字幕显示在视频左上角" },
    { value: 6, label: "顶部居中", description: "字幕显示在视频顶部中间" },
    { value: 7, label: "右上", description: "字幕显示在视频右上角" },
    { value: 9, label: "中间居左", description: "字幕显示在视频中间偏左" },
    { value: 10, label: "中间居中", description: "字幕显示在视频中间" },
    { value: 11, label: "中间居右", description: "字幕显示在视频中间偏右" },
  ];

  function handleClose() {
    saveStyle();
    onClose();
  }
</script>

{#if show}
  <div
    class="fixed inset-0 bg-black/50 z-[1100] flex items-center justify-center"
    on:click|self={handleClose}
  >
    <div class="bg-[#1c1c1e] rounded-lg w-[600px] max-h-[80vh] overflow-y-auto">
      <!-- 顶部标题栏 -->
      <div
        class="flex items-center justify-between p-4 border-b border-gray-800/50"
      >
        <h2 class="text-lg font-medium text-white">字幕压制样式设置</h2>
        <button
          class="text-gray-400 hover:text-white transition-colors duration-200"
          on:click={handleClose}
        >
          <X class="w-5 h-5" />
        </button>
      </div>

      <!-- 内容区域 -->
      <div class="p-4 space-y-6">
        <!-- 字体设置 -->
        <div class="space-y-4">
          <h3 class="text-sm font-medium text-gray-300">字体设置</h3>
          <div class="grid grid-cols-2 gap-4">
            <div class="space-y-2">
              <!-- svelte-ignore a11y-label-has-associated-control -->
              <label class="block text-sm text-gray-400">字体名称</label>
              <input
                type="text"
                bind:value={style.fontName}
                class="w-full px-3 py-2 bg-[#2c2c2e] text-white rounded-lg
                       border border-gray-800/50 focus:border-[#0A84FF]
                       transition duration-200 outline-none"
              />
            </div>
            <div class="space-y-2">
              <!-- svelte-ignore a11y-label-has-associated-control -->
              <label class="block text-sm text-gray-400">字体大小</label>
              <input
                type="number"
                bind:value={style.fontSize}
                class="w-full px-3 py-2 bg-[#2c2c2e] text-white rounded-lg
                       border border-gray-800/50 focus:border-[#0A84FF]
                       transition duration-200 outline-none"
              />
            </div>
          </div>
        </div>

        <!-- 颜色设置 -->
        <div class="space-y-4">
          <h3 class="text-sm font-medium text-gray-300">颜色设置</h3>
          <div class="grid grid-cols-2 gap-4">
            <div class="space-y-2">
              <label class="block text-sm text-gray-400">字体颜色</label>
              <input
                type="color"
                bind:value={style.fontColor}
                class="w-full h-10 bg-[#2c2c2e] rounded-lg
                       border border-gray-800/50 focus:border-[#0A84FF]
                       transition duration-200 outline-none"
              />
            </div>
            <div class="space-y-2">
              <label class="block text-sm text-gray-400">描边颜色</label>
              <input
                type="color"
                bind:value={style.outlineColor}
                class="w-full h-10 bg-[#2c2c2e] rounded-lg
                       border border-gray-800/50 focus:border-[#0A84FF]
                       transition duration-200 outline-none"
              />
            </div>
          </div>
        </div>

        <!-- 描边设置 -->
        <div class="space-y-4">
          <h3 class="text-sm font-medium text-gray-300">描边设置</h3>
          <div class="space-y-2">
            <label class="block text-sm text-gray-400">描边宽度</label>
            <input
              type="range"
              min="0"
              max="4"
              bind:value={style.outlineWidth}
              class="w-full"
            />
          </div>
        </div>

        <!-- 对齐和边距设置 -->
        <div class="space-y-4">
          <h3 class="text-sm font-medium text-gray-300">对齐和边距</h3>
          <div class="grid grid-cols-2 gap-4">
            <div class="space-y-2">
              <label class="block text-sm text-gray-400">对齐方式</label>
              <select
                bind:value={style.alignment}
                class="w-full px-3 py-2 bg-[#2c2c2e] text-white rounded-lg
                       border border-gray-800/50 focus:border-[#0A84FF]
                       transition duration-200 outline-none"
              >
                {#each alignmentOptions as option}
                  <option value={option.value} title={option.description}>
                    {option.label}
                  </option>
                {/each}
              </select>
            </div>
            <div class="space-y-2">
              <label class="block text-sm text-gray-400">垂直边距</label>
              <input
                type="number"
                bind:value={style.marginV}
                class="w-full px-3 py-2 bg-[#2c2c2e] text-white rounded-lg
                       border border-gray-800/50 focus:border-[#0A84FF]
                       transition duration-200 outline-none"
              />
            </div>
          </div>
          <div class="grid grid-cols-2 gap-4">
            <div class="space-y-2">
              <label class="block text-sm text-gray-400">左边距</label>
              <input
                type="number"
                bind:value={style.marginL}
                class="w-full px-3 py-2 bg-[#2c2c2e] text-white rounded-lg
                       border border-gray-800/50 focus:border-[#0A84FF]
                       transition duration-200 outline-none"
              />
            </div>
            <div class="space-y-2">
              <label class="block text-sm text-gray-400">右边距</label>
              <input
                type="number"
                bind:value={style.marginR}
                class="w-full px-3 py-2 bg-[#2c2c2e] text-white rounded-lg
                       border border-gray-800/50 focus:border-[#0A84FF]
                       transition duration-200 outline-none"
              />
            </div>
          </div>
        </div>

        <!-- 预览区域 -->
        <div class="space-y-4">
          <h3 class="text-sm font-medium text-gray-300">预览</h3>
          <div class="bg-black p-4 rounded-lg flex items-center justify-center">
            <div style={previewStyle}>这是一段示例字幕文本</div>
          </div>
        </div>

        <!-- FFmpeg 样式字符串 -->
        <div class="space-y-4">
          <h3 class="text-sm font-medium text-gray-300">FFmpeg 样式字符串</h3>
          <div class="p-3 bg-[#2c2c2e] rounded-lg">
            <code class="text-sm text-gray-300 break-all">{styleString}</code>
          </div>
        </div>
      </div>
    </div>
  </div>
{/if}
