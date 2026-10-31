# macOS 阶段测量与瓶颈定位

2026-10-31，M2 Max / macOS 27.0.1；沿用五分钟 1080p60、每秒 16 条文字 + 4 条 PNG 的固定样例。测量工具提交 `662f7d5e`。本阶段不改导出算法或默认编码参数，Windows 未访问。

## 同批基线

当前实现保留 MediaPlan、Skia Vulkan / MoltenVK、libplacebo 和 VideoToolbox；#459 固定 `c73c7aa`，在独立副本加计时，保留其 Skia Metal、专用合成 shader、三线程流水线与 CPU activity 辅助线程。两边均为 release、FFmpeg 9.0.2。当前媒体算法基于 `fa4d90f6`；基线计时二进制早于提交中的 Clippy 等价整理，实际二进制及方法哈希在回执内。

| 实现 | 关闭阶段计时，2 次 | 开启阶段计时，3 次 | 关闭计时中位数 |
| --- | --- | --- | ---: |
| 当前 | 41.321 / 40.989 s | 41.393 / 40.972 / 40.595 s | **41.155 s** |
| #459 | 37.973 / 37.967 s | 37.982 / 37.966 / 38.027 s | **37.970 s** |

当前多用 **3.185 s / 8.39%**。这不是上次 27.72% 的稳定复现。计时开关的中位数差为当前 -0.44%、#459 +0.03%，落在这组重复的波动范围内；不能把负值解释为计时加速，也不能由少量样本证明零开销。

## 当前 worker：主要消耗在宿主提交工作

以下为三轮诊断的独立中位数。CPU 指**调用线程**的运行时间，排除该线程阻塞，但不含 FFmpeg、驱动和硬件服务的其他线程。墙钟包含阻塞。每个完整报告的嵌套排他分区均通过检查；独立中位数不要求相加等于总计。

| 阶段 | 墙钟 | 调用线程 CPU |
| --- | ---: | ---: |
| libplacebo 拉取合成帧 | 16.714 s | 16.449 s |
| Skia 绘制与提交 | 9.409 s | 9.336 s |
| CPU 预处理滤镜 | 3.377 s | 3.340 s |
| NV12 上传 | 2.340 s | 2.322 s |
| 三帧槽复用／收尾（含子阶段） | 2.817 s | 0.384 s |
| 编码器送帧 API | 2.953 s | 0.951 s |
| **worker 总计** | **40.508 s** | **35.293 s** |

libplacebo 与 Skia 的宿主工作占 worker CPU 的大部分；这不表示像素合成改在 CPU 上执行。GPU 仍完成渲染和合成。第一轮 Metal 完成阻塞仅 0.004 s，已经不是优先目标。GPU blit 时间戳属于可重叠的设备区间，未加到上述墙钟分区。

## #459：编码回压占据关键路径

#459 的线程并行，**不能将下列线程墙钟相加当作导出时间**。

| 线程／阶段 | 墙钟中位数 | 线程 CPU 中位数 |
| --- | ---: | ---: |
| 解码线程总计 | 37.549 s | 1.658 s |
| 解码线程等待下游队列 | 35.137 s | 0.110 s |
| 渲染线程总计 | 37.908 s | 8.928 s |
| 其中 Skia | 6.439 s | 6.419 s |
| 其中专用合成 shader 的宿主提交 | 0.510 s | 0.510 s |
| 渲染线程等待编码队列 | 28.652 s | 0.091 s |
| 编码线程总计 | 37.674 s | 1.017 s |
| 其中编码器送帧 API | 36.966 s | 0.500 s |
| CPU activity 辅助线程 | 37.880 s | 37.751 s |

送帧 API 墙钟是包括 VideoToolbox 回压的宿主时间，不是独立测得的硬件编码器执行时间。渲染和解码线程大多被下游队列阻塞，表明这组输入下 #459 已有充足的渲染余量。辅助线程消耗近一个完整 CPU 核，所有资源统计都包含它；其性能作用需单独消融，不能从存在该线程就推断必要性。

## 波动：目前能确定与不能确定的部分

- 本轮五次当前实现（2 次控制 + 3 次诊断）为 40.595–41.393 s，未复现上次 44.225–51.447 s。每 600 帧累计的 CPU／墙钟检查点已记录，可区分视频位置变化与整轮变慢。
- 全部基线采样的 thermal state 为 nominal，低电量模式为 false。未取得 CPU/GPU 实时频率；这些读数不能证明没有频率或调度变化。上次没有同步遥测，无法事后确定那次波动的唯一原因。
- 当前关键 worker 请求 QoS 为 Default（21），#459 渲染线程为 UserInitiated（25），辅助线程为 UserInteractive（33）；解码／编码线程均为 Default。这里是请求值，不是有效优先级或核心驻留证明。
- 当前有约 35 秒串行 worker CPU 工作，而 #459 渲染线程约 9 秒、主要受编码回压限制。前者对宿主调度与执行速度更敏感，是有测量依据的解释；QoS、外部负载、频率各自贡献仍需受控试验。
- 两边保留各自默认 GOP／B 帧策略（当前 GOP 120，#459 GOP 240 且 HEVC 重排不同），所以不能把剩余差距全部归因于 Vulkan/Metal，也不能宣称画质等价。

## 验证与复现

12 个产物（10 个完整 + 2 个预热）全部通过：完整帧数与 PTS 网格、音频包载荷一致性，以及文字／PNG 内容采样。所有计时作业串行，验证在计时外；产物仅在通过校验、记录 SHA-256 后删除。调用线程 CPU 分区和 bounded checkpoint 的 3 项单元测试通过；提交的格式、普通／headless Clippy 与普通／headless 测试钩子均通过。

方法见 [profiling/README.md](profiling/README.md)，[逐次阶段数据、遥测、资源和输出校验](validation/macos-stage-baseline-fa4d90f6.json)。原始日志与参考图片在 `/private/tmp/bsr-macos-stage-optimization/baseline-native`。初次沙箱预热因无法访问 Metal/VideoToolbox 而拒绝 GPU 导出，不计入数据；正式测量均在宿主设备执行。

## 第一轮优化与消融

下面是同一批次的单次诊断，用于筛选方向，不当作统计上稳定的百分比收益。完整 300 秒导出与 10 秒预热均已校验。

| 方案 | 完整导出墙钟 | 进程 CPU 总时间 |
| --- | ---: | ---: |
| 原始、未加新计时的当前二进制 | 40.685 s | 126.09 s |
| 当前，同一 QoS 实验二进制继承 Default | 40.861 s | 126.23 s |
| 当前，同一二进制请求 UserInitiated | 41.157 s | 125.95 s |
| 合并 CPU 预处理，继承 Default | 40.882 s | 125.29 s |
| #459，关闭 CPU activity 忙等线程 | 37.931 s | 70.51 s |
| #459，保持原忙等线程 | 37.942 s | 107.71 s |

**保留 CPU 预处理合并**：将尺寸缩放、BT.709 limited-range 转换和 NV12 输出放在同一个 scale 后填黑边，避免第二次遍历。该阶段从 3.296 s / 3.266 CPU s 降到 1.834 s / 1.818 CPU s，减少约 44%；端到端没有可辨认收益，部分余量转成了 GPU 帧槽等待。此处没有改变编码器、GOP、码率或渲染后端。

独立 CLI 像素 oracle 中，实际 1080p 视频、横屏 720p YUV、竖屏 YUV 的旧／新 NV12 原始帧逐字节一致，竖屏黑边 Y=16、UV=128 正确。RGB 彩条因一次缩放／色度转换取代两次重采样而不逐字节一致（8-bit 样本平均绝对差 0.213、最大 32），不能据此宣称所有输入无损等价。正式应用仍完全通过原生库导出，CLI 仅用于独立验证。

**不保留 QoS 修改**：同一二进制切换没有加速。Instruments 的关键 worker 采样中，按 sampled cycles 加权的 99.96% 在性能核，不能把本轮差距归因于落到能效核。较多采样落在 libplacebo dispatch、MoltenVK queue submit、Metal 驱动、Skia 操作合并和内存操作；这是调用栈证据，不是各阶段独立耗时之和。复现实验补丁只存放在 benchmark 目录，没有应用到产品。

**#459 的忙等线程在本机本负载下没有可辨认收益**，关闭后少消耗约 37 CPU 秒；仅对独立 PR 副本做了消融。基线保留其原始行为。

新增 IOKit 遥测显示：上述当前实现的 GPU device utilization 中位数为 **99%**，#459 两种辅助线程配置均约 **26%**。该系统级计数器不是逐进程 GPU 时间，也不衡量 VideoToolbox 的编码单元占用；测量期间没有同时运行其他基准任务。结合帧槽等待，后续应检查 GPU 合成工作量与提交方式，不能只按线程 CPU 排名判断唯一瓶颈。

证据：[候选方案逐次结果](validation/macos-stage-candidates-0adc9bbb.json)、[归一化像素 oracle](validation/macos-normalization-oracle.json)、[关键 worker CPU 栈摘要](validation/macos-stage-cpu-samples.json)。运行像素 oracle：

```sh
python3 scripts/export-comparison/profiling/check_normalization.py \
  --ffmpeg /opt/homebrew/opt/ffmpeg-full/bin/ffmpeg --output /tmp/normalization.json
```

## GPU 等待的定向实验（均未保留）

两组均在已合并 CPU 预处理的基础上单独比较，未混入 QoS 提升。

| 变更 | 同批基线 | 候选方案 | 观察 |
| --- | ---: | ---: | --- |
| 在途帧 3 → 6，命令槽 6 → 12 | 40.515 s | 40.936 s | overlay 槽等待 2.587 → 0.027 s；libplacebo 墙钟 17.208 → 19.642 s，其 CPU 仍约 16.8–16.9 s |
| 两输入先合成 RGBA8，另一个 libplacebo 最后转 NV12 | 40.558 s | 48.425 s | libplacebo CPU 16.854 → 12.156 s，但 output 槽等待 0.398 → 17.080 s |

第一组主要移动了阻塞位置；第二组减少宿主工作，却恶化了 GPU 输出等待。两组当前 GPU utilization 中位数均约 99%。因此不能根据单个 CPU span 变小就宣布导出更快，也没有理由扩大默认队列或保留额外 RGBA 图。两组均撤回，生产保持 3 帧、有界异步交接和原 GPU 合成算法。RGBA8 实验还会改变中间量化，不能视作像素等价优化。

FFmpeg 的多输入 libplacebo 对每个输入分别调用 renderer，再混到目标平面；libplacebo 的 NV12 多平面输出依赖中间 FBO。[FFmpeg 9 源码](https://github.com/FFmpeg/FFmpeg/blob/n9.0/libavfilter/vf_libplacebo.c)、[libplacebo 7.360.1 源码](https://github.com/haasn/libplacebo/blob/v7.360.1/src/renderer.c)。剩余重点是减少这条通用合成路径的 GPU pass／纹理流量和驱动提交开销；简单增加队列、拆成两个通用滤镜都未达到目的。专用 Vulkan 合成应作为后续独立改动，先验证色度采样、预乘透明度与颜色边界，不能仅替换成较低质量采样器来报加速。

证据：[在途帧实验](validation/macos-depth6-fixed-ablation.json)、[RGBA 图实验](validation/macos-rgba-ablation.json)。共 8 个产物全部通过完整帧／PTS／音频校验和文字／表情采样。首次 6 帧预热被旧的 6 命令槽上限拒绝，修正实验上限后重跑，失败预热不计入对照。`profiling/rejected-*-experiment.json` 保存 base 与可提取的 patch 字符串，仅用于重现，未应用于产品。
