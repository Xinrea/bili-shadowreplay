# 共享 GPU 导出与 PR #459：五分钟真实场景对照

本文保留旧版 `94b0a322` 的历史结果。完成解码与 compute 优化后的最新对照见
[Windows 三轮重测](REPORT-windows-compute.md)和[macOS compute 对照](REPORT-macos-compute.md)。

2026-10-31，以正式生产源码的 release runner 重新实测。**#459 在两台测试机上仍更快**。
后续已完成[当前实现的分阶段测量](REPORT-stages.md)，用于定位解码、提交和交接等待的占比。
主指标是三组交替运行的进程耗时中位数，包括启动、场景解析、能力探测、解码、渲染、编码、封装、关闭及退出。

| 测试机 | 当前实现 | #459 | #459 吞吐 / 当前 | #459 耗时减少 |
| --- | ---: | ---: | ---: | ---: |
| macOS / M2 Max | 112.003 s / 160.71 FPS | 38.249 s / 470.60 FPS | **2.928×** | **65.85%** |
| Windows / 5900X + RTX 5070 | 96.748 s / 186.05 FPS | 74.269 s / 242.36 FPS | **1.303×** | **23.23%** |

macOS 为 M2 Max（12 CPU / 38 GPU、64 GB、macOS 27.0.1）；Windows 为 Ryzen 9 5900X
（12 核 / 24 线程、约 48 GiB）+ RTX 5070（驱动 617.42）。只比较同一机器内的两种实现，
不能将两台机器的绝对性能差归因于操作系统。

## 版本、参数和测量范围

- 当前生产源码：`94b0a32224e83e993703d843a470926134a50edd`，已接入共享 Skia Vulkan 和 macOS VideoToolbox 适配器。
- [PR #459](https://github.com/Xinrea/bili-shadowreplay/pull/459)：`c73c7aaff9b9dd9abb561c90a48f19b2d6374112`，测试前再次确认仍是最新 head，生产渲染/编码源码未修改。
- 同一份原生 1080p60 连续五分钟 Big Buck Bunny 视频，18,000 帧，SDR BT.709；复用 [固定素材](fixtures.json)，两平台输入和场景 SHA-256 相同。
- 每秒 20 条、共 6,000 条弹幕：4,800 文本 + 1,200 PNG 表情，滚动 8 秒，最多约 160 条同时活跃；沿用同一 ASS/图片场景和同平台字体环境。
- 默认 HEVC、速度优先、10 Mbps、1080p60，直接复制源 AAC 音轨；其余编码策略保留各实现的真实默认值。
- 每个实现先预热 600 帧，再运行三组完整视频：当前→PR、PR→当前、当前→PR；每台机器同一时刻只运行一个导出或校验任务。
- 同平台均使用 FFmpeg 9.0.2。Windows 完整 SDK 与运行目录的七个 DLL 哈希一致；macOS 当前 runner 打包 MoltenVK 1.4.2 和同源 FFmpeg 动态库，依赖重定位不改变媒体算法。
- 所有当前 runner 均传入 `--require-gpu`，并核对 `frame_processor=true` 和实际编码器；不会把能力探测后的 CPU 回退误记为 GPU 结果。

这是**单视频弹幕压制**的完整媒体路径测试。准备素材/弹幕布局、构建、GUI/V8/语音启动、项目分段缓存、上传及输出核验不计时。
测试期间未运行其他 GPU 测试、API capture 或 profiler；清除了相关调试环境变量。
没有为了本次对照修改渲染、解码线程或编码算法，测试驱动仅增加版本来源记录和 GPU 必选检查。

## 全部计时

| 平台 / 实现 | 第 1 次 | 第 2 次 | 第 3 次 | 最小–最大 |
| --- | ---: | ---: | ---: | ---: |
| macOS / M2 Max / 当前 | 110.610 s | 112.003 s | 113.310 s | 110.610–113.310 s |
| macOS / M2 Max / #459 | 38.249 s | 38.105 s | 38.288 s | 38.105–38.288 s |
| Windows / 5900X + RTX 5070 / 当前 | 95.379 s | 96.748 s | 96.869 s | 95.379–96.869 s |
| Windows / 5900X + RTX 5070 / #459 | 74.184 s | 74.269 s | 74.294 s | 74.184–74.294 s |

## 与历史结果的关系

[旧报告](REPORT.md)的当前版本是 `730ebd23`，macOS 当时使用 CPU Skia：

- macOS 当前中位数从历史记录的 **171.683 s** 降至本轮 **112.003 s**，耗时减少 **34.76%**；#459 相应为 37.963 s 与本轮 38.249 s。
- Windows 从历史记录的 **96.314 s** 变为本轮 **96.748 s**，增加 **0.45%**，两次测试的波动区间重叠，未见明显变化。

旧版本没有在本轮重跑，因此上面是历史对照，不是同期的 CPU/GPU 单因素实验。
此前 116.5 s 是有其他构建检查同时进行的单次稳定性验证，也没有混入本轮三次中位数。

## 两条路径仍有什么不同

| 项目 | 当前实现 | #459 |
| --- | --- | --- |
| 共享媒体调度 | MediaPlan worker 推进解码、滤镜、帧处理、编码与封装 | 独立解码、渲染、编码线程，阶段间有界队列 |
| 解码线程 | 未显式设置时沿用 SDK 默认 | 显式 `thread_count=0` 自动选择 |
| Windows 合成 | Skia Vulkan + libplacebo | Skia Vulkan + 专用 compute shader |
| macOS 合成与交接 | Skia Vulkan / MoltenVK + libplacebo，等待 Vulkan 完成后 GPU blit 到 IOSurface，并等待 blit 完成 | Skia Metal + 专用 compute shader 直接写可编码 CVPixelBuffer，帧槽到交接/复用时同步 |
| GPU 在途帧 | 合成侧最多三组；macOS 桥接仍有逐帧完成等待 | 三帧槽，配合独立阶段线程 |
| macOS CPU 活跃线程 | 无额外忙等线程 | 在逻辑 CPU ≥12 且有 scene 时，额外启动一个高 QoS 忙等线程，不做视频工作；本机满足条件 |

两者都保留 CPU 视频解码及底图上传，GPU 合成后的原始帧不需要回读到 CPU。
本轮没有拆分测量线程、等待、合成 shader、MoltenVK、编码参数或忙等线程各自的贡献，
**不能把全部差距归因于 Vulkan 对比 Metal，也不能把无回读等同于吞吐已达到 #459**。

相关源码：[当前 MediaPlan 解码配置](../../src-tauri/crates/ffmpeg_utils/src/media/util.rs)、
[当前 Metal 交接](../../src-tauri/crates/ffmpeg_utils/src/vulkan_video/metal_transfer.m)、
[#459 pipeline](https://github.com/Xinrea/bili-shadowreplay/blob/c73c7aaff9b9dd9abb561c90a48f19b2d6374112/src-tauri/crates/gpu-export/native/pipeline.h)、
[#459 Metal 与 CPUActivity](https://github.com/Xinrea/bili-shadowreplay/blob/c73c7aaff9b9dd9abb561c90a48f19b2d6374112/src-tauri/crates/gpu-export/native/metal.mm)。

## 实际输出参数与资源

| 平台 / 实现 | 最大 GOP 间隔 | has_b_frames | 视频码率中位数 | 文件大小中位数 |
| --- | ---: | ---: | ---: | ---: |
| macOS / M2 Max / 当前 | 120 | 0 | 10.039 Mbps | 366.26 MiB |
| macOS / M2 Max / #459 | 240 | 2 | 10.058 Mbps | 367.10 MiB |
| Windows / 5900X + RTX 5070 / 当前 | 120 | 1 | 9.960 Mbps | 363.55 MiB |
| Windows / 5900X + RTX 5070 / #459 | 300 | 1 | 9.973 Mbps | 364.00 MiB |

`has_b_frames` 是重排相关字段，不是 B 帧总数；完整 I/P/B 数量在回执中。
当前 Windows 显式设置 `async_depth=3, rc_mode=vbr`，#459 使用其速度优先 `quality=0` 配置。
两边实际码率接近，但 GOP、B 帧及编码默认值不同，**本轮不是同等编码画质的性能对照**。

macOS 资源消耗（三次中位数）：

| 指标 | 当前 | #459 |
| --- | ---: | ---: |
| 用户态 + 内核态 CPU 时间 | 127.32 CPU-s | 110.29 CPU-s |
| 平均 CPU 占用 | 1.13 核 | 2.89 核 |
| 峰值常驻集 RSS | 208.95 MiB | 312.45 MiB |
| 峰值物理 footprint | 398.34 MiB | 463.09 MiB |

CPU 总时间按所有线程累加，不能与墙钟耗时混为一谈；#459 的 CPU 数值包含上述忙等线程。
RSS 和物理 footprint 是不同口径，都不是完整 GPU 显存或能耗统计。Windows 未采集同口径资源数据。

## 校验与复现

**两平台 12 个正式成片和 4 个预热成片全部通过**。每个正式成片完整解码 18,000 帧，
检查连续 CFR 时间戳、HEVC 尺寸/帧率/时长，确认 AAC 包载荷与源完全一致；
在 2、30、60、120、180、240、298、299.9 秒分别验证文字和图片叠层证据。
不同 GPU 后端、字体栅格化和有损编码仍有像素差异；通过这些检查不表示逐像素相同或编码质量相等。
核验使用独立 CPU 参考与 FFmpeg CLI，均在计时区间之外；当前生产媒体路径未使用 CLI。

本轮未重新开启 Vulkan validation，因此不宣称 SDK 的 validation 错误已修复；
已知限制仍见 [共享 GPU 适配器验证](../vulkan-video-poc/MACOS_VIDEOTOOLBOX.md)。
Windows 全程使用临时隔离目录，原始工作区仍只有开始时的三个既存修改。

- [macOS 完整数据](validation/macos-shared-gpu-94b0a322.json)
- [Windows 完整数据](validation/windows-shared-gpu-94b0a322.json)
- [源码、MoltenVK 与 Windows DLL 哈希](validation/shared-gpu-source-94b0a322.json)
- [场景和复现命令](README.md)

后续性能工作可优先验证共享 MediaPlan 的阶段并行、解码并行配置和 macOS 桥接的等待重叠；
这些是基于实现差异的优化候选，本轮数据尚未证明各项能带来多少收益。
