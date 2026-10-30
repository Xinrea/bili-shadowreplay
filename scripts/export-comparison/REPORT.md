# 5 分钟密集弹幕：当前实现与 PR #459

2026-10-31，在同一套真实视频与弹幕场景下，**#459 在 macOS 和 Windows 均更快**。
以下均为 release 构建，三组交替运行的外层进程耗时中位数，覆盖启动、解析、解码、渲染、编码、封装、关闭及退出。

| 测试机 | 当前实现 | #459 | #459 吞吐 / 当前 | #459 耗时减少 |
| --- | ---: | ---: | ---: | ---: |
| macOS 27.0.1，M2 Max（12 CPU / 38 GPU，64 GB） | 171.683 s / 104.84 fps | 37.963 s / 474.14 fps | **4.522×** | **77.89%** |
| Windows，5900X（12 核 / 24 线程，约 48 GiB）+ RTX 5070（12 GiB，617.42） | 96.314 s / 186.89 fps | 74.128 s / 242.82 fps | **1.299×** | **23.04%** |

两台机器硬件不同，应在每台机器内部比较两个实现；这里不能把 macOS 与 Windows 的绝对耗时差归因为操作系统差异。

## 固定版本与工作负载

- 当前：`730ebd23d57ae01d7ad30e49d944288c07dfbef4`。
- [#459](https://github.com/Xinrea/bili-shadowreplay/pull/459)：`c73c7aaff9b9dd9abb561c90a48f19b2d6374112`，测试开始时的最新 PR head，原样提取其 GPU exporter 和 renderer。
- 输入为原生 **1920×1080、60 fps** 的连续 300 秒视频，来自 [Big Buck Bunny Sunflower 官方素材](https://peach.blender.org/download/) 的 60–360 秒；未循环短片或放大低分辨率素材。
- 输入统一预处理为 H.264/AAC、SDR BT.709，共 **18,000 帧**，只生成一次并逐字节分发。两平台 source SHA-256 均为 `e99d957cabd2d7346b50c29d6ca50df6ae23fffca9fcff63ee27c7bb77407580`，scenario SHA 也完全相同。
- 每秒 20 条、每 50 ms 一条：**16 条文本 + 4 条图片表情**。总计 4,800 文本、1,200 图片事件；文本含中英及 Unicode emoji，图片为 12 种共享 Twemoji PNG。滚动持续 8 秒，最多 160 条同时活跃，透明度 80%。两边消费同一份预布局场景。
- 输出使用默认 HEVC / 速度优先 / 10 Mbps / 1080p60，复制相同 AAC 音轨。每版本先预热精确 10 秒、600 帧，再按 current→PR、PR→current、current→PR 跑三组完整导出。
- 同一平台两边均使用匹配的 FFmpeg 9.0.2；Windows 的 SDK 头文件和 DLL 未替换，仅给 #459 构建补充从官方头文件派生的 `.pc` 元数据。Windows 原始仓库及三个既存修改保持不变。

这是单视频弹幕压制链路比较。下载、弹幕调度、生成输入、构建、上层 GUI/语音运行时、项目分片缓存及上传不计入；解析共同场景和实际媒体工作计入。没有将当前 macOS 临时替换为 Metal，也没有修改 #459 的渲染、线程或编码算法。

## 全部运行数据

| 平台 / 实现 | 第 1 次 | 第 2 次 | 第 3 次 | 最小–最大 |
| --- | ---: | ---: | ---: | ---: |
| macOS 当前 | 175.263 s | 170.938 s | 171.683 s | 170.938–175.263 s |
| macOS #459 | 38.030 s | 37.952 s | 37.963 s | 37.952–38.030 s |
| Windows 当前 | 94.664 s | 96.314 s | 98.394 s | 94.664–98.394 s |
| Windows #459 | 74.065 s | 74.137 s | 74.128 s | 74.065–74.137 s |

没有选择最快一次，也没有用 10 秒预热或前一轮 debug 短片推断长片吞吐。

macOS 还记录了进程资源消耗（三次中位数）：

| 指标 | 当前 | #459 |
| --- | ---: | ---: |
| 用户态 + 内核态 CPU 时间 | 285.83 CPU-s | 107.12 CPU-s |
| 平均使用 CPU 核数 | 1.665 | 2.817 |
| 峰值常驻集 RSS | 345.33 MiB | 312.14 MiB |
| 系统报告的峰值物理 footprint | 280.22 MiB | 457.06 MiB |

RSS 与物理 footprint 是不同系统口径，均不是完整 GPU 显存统计；尤其不能只凭 RSS 得出 Metal 总内存更低的结论。Windows 本轮未采集相同资源计数，只比较完整导出耗时与成片。

## 默认编码设置差异

这是各自默认链路的端到端结果，**不是严格等编码参数、等画质的单因素对照**。

| 平台 / 实现 | 实际合成后端 | 实际 GOP | `stream.has_b_frames` | 视频码率中位数 |
| --- | --- | ---: | ---: | ---: |
| macOS 当前 | CPU Skia + FFmpeg VideoToolbox | 120 | 0 | 10.040 Mbps |
| macOS #459 | Skia Metal + VideoToolbox | 240 | 2 | 10.058 Mbps |
| Windows 当前 | Skia Vulkan + libplacebo + Vulkan Video | 120 | 1 | 9.960 Mbps |
| Windows #459 | Skia Vulkan + 专用 compute + Vulkan Video | 300 | 1 | 9.973 Mbps |

`has_b_frames` 是流的重排相关字段，不是 B 帧总数。Windows 当前实际为 I=150 / P=6000 / B=11850，#459 为 I=60 / P=6000 / B=11940；完整分布保存在 JSON。两边实际输出码率和文件大小相近，但这不能证明编码画质完全相等。

macOS 的路径差异包含此前暂缓的 Metal 后端。Windows 两边均保留 CPU 解码与一次 NV12 上传；当前通过同一 MediaPlan worker 调度和 libplacebo 合成，#459 使用解码/渲染/编码三个线程及专用合成 shader。#459 还显式启用自动解码线程，当前未指定时保留 SDK 默认。相关源码见 [当前 MediaPlan](https://github.com/Xinrea/bili-shadowreplay/blob/730ebd23d57ae01d7ad30e49d944288c07dfbef4/src-tauri/crates/ffmpeg_utils/src/media/graph.rs)、[#459 pipeline](https://github.com/Xinrea/bili-shadowreplay/blob/c73c7aaff9b9dd9abb561c90a48f19b2d6374112/src-tauri/crates/gpu-export/native/pipeline.h) 和 [Vulkan compositor](https://github.com/Xinrea/bili-shadowreplay/blob/c73c7aaff9b9dd9abb561c90a48f19b2d6374112/src-tauri/crates/gpu-export/native/vulkan.cpp)。本轮未拆分这些因素，不能把全部提速归因于异步、shader、解码线程或消除回读中的某一项。

## 正确性与复现

**两平台 12 个正式成片全部通过**：完整解码 18,000 帧、逐帧 PTS 网格、HEVC 尺寸/帧率/时长、AAC 压缩包 payload 与源完全一致；在 2、30、60、120、180、240、298、299.9 秒分别确认文本和图片叠加证据。尾部也包含在核验内。GPU 栅格化、抗锯齿和有损编码仍有像素差异，没有把“肉眼有字幕”当作像素完全相同。

核验、PNG 参考生成、完整视频解码和音频包哈希均在计时外执行。计时期间没有 API capture、Vulkan validation 或额外 GPU 测试。此轮不重新宣称 Vulkan validation 全量干净；旧 SDK 清理诊断仍应参考先前回执。

- [macOS 可移植数据](validation/macos-2026-10-31.json)
- [Windows 可移植数据](validation/windows-2026-10-31.json)
- [复现命令、固定素材及版本说明](README.md)

之前的 0.37% / 3.31% 是短片 debug 条件下当前实现的同步/异步对照；本次是完整场景下当前实现与 #459 的对照，比较对象和构建条件均不同。
