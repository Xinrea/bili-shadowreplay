# MoltenVK → VideoToolbox：共享 GPU 导出链路

2026-10-31，本机 Apple M2 Max / macOS 27.0.1 与 Windows RTX 5070 验证机验证。
Windows 保留 Vulkan Video，macOS 使用 MoltenVK 1.4.2 + FFmpeg 9.0.2 VideoToolbox。
普通构建默认包含 GPU 后端；默认速度优先配置经完整能力探测后自动选择，无需额外 feature。

## 共享边界

`MediaPlan` 继续负责解码、剪辑/滤镜、时间戳、音频、编码、封装、取消和输出发布。
两端共用 `VulkanCompositor`、Skia Vulkan、libplacebo 合成与 NV12 转换、有限在途帧队列。
`RenderPipeline` 独立选择 renderer 和 encoder，并把渲染身份写入分片及投稿缓存键。
CPU 与 GPU 都可使用 VideoToolbox，因此不能再用编码器名称推断渲染方式。

平台差异集中在 `HardwareEncoderFrames`：

| 平台 | GPU 帧交接 |
| --- | --- |
| Windows | 合成后的 Vulkan NV12 `AVFrame` 直接交给原 MediaPlan Vulkan Video 编码器 |
| macOS | MoltenVK 的独立 R8/RG8 纹理，经 `VK_EXT_metal_objects` 导出，Metal blit 到 VideoToolbox 帧池的 IOSurface，返回 `AV_PIX_FMT_VIDEOTOOLBOX` |

macOS 没有改用 Skia Metal，也没有依赖 Vulkan Video。图像创建时声明 Metal 导出能力；
交接等待 Vulkan 写入完成，保持源帧、两张 Metal 纹理和目标 CVPixelBuffer 存活到 blit 完成。
编码器继续持有目标硬件帧，错误/取消时先完成同步再销毁资源。
这是**没有 CPU 帧回读的一次 GPU 内部复制**，不是严格零拷贝；第一版还有 blit 完成等待。
CPU 底图解码、一次上传以及文字/图片资源上传仍存在。

## 验证结果

原始回执与汇总见 [validation/shared-gpu-2026-10-31.json](validation/shared-gpu-2026-10-31.json)。

| 检查 | 结果 |
| --- | --- |
| macOS 完整 headless 测试 | 486 通过、30 忽略 |
| macOS GPU 业务回归 | 5 通过；H.264/HEVC、同步/异步探测、真实运动视频、透明度、音频、分片缓存、取消和资源释放 |
| Windows GPU 业务回归 | 5 通过，保持 Vulkan Video 默认选择 |
| Windows FFmpeg 模块测试 | 169 通过、26 忽略 |
| Windows 普通 GUI 构建检查 | `cargo check` 通过 |
| 双平台同步/异步基准回归 | H.264/HEVC 均通过；使用各平台实际编码参数，并验证每帧、音频及采样画面一致性 |
| macOS 打包与签名 | 2 项测试通过；重复打包、最低部署版本、签名和动态库解析 |
| 无 SDK 环境的实际包内 GPU 导出 | 通过；清除 Vulkan SDK、ICD 和动态库路径环境变量，仍选择 GPU |
| 5 分钟真实视频 + 每秒 20 条弹幕 | 18,000 帧，4,800 文本 + 1,200 表情图片；约 116.5 秒导出，154.5 FPS |
| 长视频输出独立校验 | 完整 CFR 帧网格通过；音频包及载荷完全一致；8 个采样点（含 299.9 秒末帧附近）均存在文字和图片叠层 |

长视频使用现有 `scripts/export-comparison/assets/scenario.json`、HEVC/速度优先/10 Mbps/
1080p60/GOP 120，并通过包含实际生产源码的 release runner 执行。
这是本轮单次稳定性验证，期间有其他构建检查，**不是新的受控跨实现性能对照**；
不能直接与此前 #459 的测量计算提速百分比。
另补跑了两端 6 秒 debug 素材的同步/异步基准，确认基准不再错误要求 VideoToolbox
具有 Vulkan 编码器的 `async_depth` 参数；该短场景用于基准和提交模式回归，不能外推长片性能。

## 回读跟踪与 SDK 限制

10 秒、600 帧完整导出启用 Khronos validation、API dump 和 Metal API Validation：

- `vkCmdCopyImageToBuffer`、`vkCmdCopyImageToBuffer2`、`vkCopyImageToMemory[EXT]` 均为 **0**。
- 62,561 次 `vkCmdDraw`；1,801 次 `vkQueueSubmit`、12,600 次 `vkQueueSubmit2`。
- 3 次 `vkMapMemory` 是缓冲区映射，不能单独作为帧回读证据；源码没有 CVPixelBuffer base-address 锁定/读取。
- Metal 没有报告 blit 错误；业务回归里的 Skia `readback` 为 0。
- 没有图像创建、视图重解释或 Metal 导出参数的 validation error。

**Khronos validation 并非零错误**：所用 FFmpeg 9.0.2 在每次创建设备时报告
`VUID-VkDeviceCreateInfo-pNext-02830`，本次探测和实际导出各一次。
[上游 hwcontext_vulkan.c](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavutil/hwcontext_vulkan.c)
的 `setup_features` 在 portability 子集下同时链接 Vulkan 1.2 features 和独立 timeline-semaphore
features；这是重复的 promoted feature 结构。此轮没有修改系统 FFmpeg，也没有屏蔽该错误。
两端功能通过不等于所用 SDK 在所有 validation 检查下无错误，后续 SDK 修复仍需重跑。
Windows 本轮使用未回补的 Gyan 9.0.2 做功能回归，已有 Vulkan 编码器清理问题仍见
[SDK patches](../ffmpeg-sdk-patches/README.md)；本轮没有重做 Windows API capture。

跟踪覆盖应用提交的 GPU 命令和桥接实现，不证明 VideoToolbox、Metal 驱动内部不存在复制。
完整视频/音频/画面校验在导出完成后通过独立 CLI oracle 运行；生产链路没有 CLI 进程。

## 构建、打包与复现

依赖与发布设置见 [native-overlay.md](../../docs/development/native-overlay.md)。
打包时使用 `BSR_MOLTENVK_LIBRARY` 或 Homebrew `molten-vk`，将所有包内 Vulkan 链接统一到
MoltenVK，避免 libplacebo 提前加载另一份 loader。包内文件名是 `libvulkan.dylib`，保留
`@rpath` install-name，使 FFmpeg 按裸名称 `dlopen` 时找到预先加载的同一映像。
没有改变用户全局 Vulkan 配置，也没有捆绑或选择 KosmicKrisp。

两端业务回归命令（先按开发文档准备各平台依赖）：

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features \
  --features headless native_gpu_ -- --ignored --nocapture --test-threads=1 \
  --skip native_gpu_benchmark
```

Mac Vulkan validation 使用 LunarG SDK 的 MoltenVK ICD，设置 `DYLD_LIBRARY_PATH`、
`VK_DRIVER_FILES`、`VK_LAYER_PATH`，启用 `VK_LAYER_KHRONOS_validation:VK_LAYER_LUNARG_api_dump`
及 `MTL_DEBUG_LAYER=1`。这些仅用于开发跟踪，正式包不需要。
长场景通过 `scripts/export-comparison/current` runner 的
`--require-gpu --scenario scripts/export-comparison/assets/scenario.json --output OUTPUT.mp4` 执行；
`--require-gpu` 会拒绝能力探测后的 CPU 回退，避免把 CPU 导出误记为 GPU 验证。
输出检查复用 `scripts/export-comparison/verify.py`。

验证范围是上述 M2 Max 与 RTX 5070，尚未实测 Intel Mac 或其他 GPU。
