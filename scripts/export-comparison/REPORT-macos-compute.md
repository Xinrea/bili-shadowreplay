# macOS：专用 Vulkan compute 与 #459 对照

2026-10-31，Mac Studio / Apple M2 Max（12 CPU、38 GPU、64 GiB）、macOS 27.0.1。
当前实现固定 `8c9016cf`；#459 固定 `c73c7aaff9b9dd9abb561c90a48f19b2d6374112`，
使用此前保存的只增加阶段计时的独立二进制。两边均为 release / FFmpeg 9.0.2。
Skia、MediaPlan、解码、编码参数和两端编码适配器未换用 #459 的实现。

## 五分钟默认场景

300 秒、1920×1080、60 fps，共 18,000 帧；每秒 16 条文字 + 4 条 PNG 表情，
共 6,000 条事件，滚动时长 8 秒。默认 HEVC、速度优先、10 Mbps、AAC 直接复制。
只测 macOS；当前继续使用 Skia Vulkan / MoltenVK 1.4.2，#459 使用 Skia Metal。

| 实现 | 三轮控制组墙钟 | 中位数 | 按进程墙钟折算吞吐 |
| --- | --- | ---: | ---: |
| 当前，Vulkan compute | 33.850 / 34.305 / 34.337 s | **34.305 s** | 524.7 fps |
| #459 | 38.337 / 38.427 / 38.102 s | **38.337 s** | 469.5 fps |

当前耗时少 **4.032 秒 / 10.52%**，吞吐高 **11.75%**。这是本机本场景下三次控制组的
中位数比较，不是所有素材／设备的加速保证。完整输出校验不计入导出耗时。

每边另有一轮阶段诊断：当前 33.948 s、#459 38.234 s；诊断轮不进入控制组中位数。
外部墙钟包括进程启动、GPU 探测和关闭，阶段 worker 总计不包含全部进程开销。
计时期间没有并行构建、其他基准或输出校验；两个实现按轮次交替顺序串行执行。
重复范围分别为 0.488 s 和 0.325 s，没有重现先前 44–51 秒的大幅波动。
全部遥测 thermal state 为 nominal、low-power 为 false；没有 CPU/GPU 实时频率数据，
不能据此排除调频或给这不到半秒的变化指定唯一原因。

## 阶段变化

下表来自当前的一轮诊断，单位秒。墙钟含阻塞，CPU 是调用线程时间；
CPU 不含其他解码／驱动线程。嵌套区间不能重复相加。

| 当前阶段 | 宿主墙钟 | 调用线程 CPU |
| --- | ---: | ---: |
| compute 提交（含命令槽检查） | **0.458** | **0.451** |
| Skia 绘制／提交 | 9.501 | 9.293 |
| CPU 归一化滤镜 | 1.970 | 1.903 |
| NV12 上传 | 2.483 | 2.425 |
| 在途帧槽等待／收尾 | 0.113 | 0.108 |
| 编码帧交接（含 Metal 子阶段） | 0.994 | 0.924 |
| 编码器送帧 API | **15.508** | **0.920** |
| worker 总计 | **33.534** | **17.670** |

上一轮已验证的 libplacebo 诊断为合成拉取约 17.197 s / 16.910 CPU s、
worker 40.414 s / 34.829 CPU s；这是[历史阶段记录](REPORT-macos-stages.md)，
本次没有重新跑旧合成器。专用 shader 消除了大量通用合成的宿主工作；
不能把减少的约 17 秒直接当作端到端收益，因为编码回压随之显露。
现在主要时间在 Skia 和编码器送帧等待，compute 已不是主要宿主瓶颈。
这些 span 没有测出 compute shader 或硬编单元的独立 GPU 执行时间。

#459 同轮：渲染线程 38.127 s / 10.670 CPU s，其中 Skia 7.906 / 7.731 s、
专用合成提交 0.657 / 0.641 s、等待编码队列 26.813 / 0.111 s；
编码送帧 36.779 / 0.679 s。其线程重叠运行，不能把这些墙钟相加比较。
CPU activity 辅助线程仍保留，消耗 37.614 CPU 秒。

当前三轮进程 CPU 总量为 69.94–71.68 s，#459 为 109.75–112.66 s；
后者包含辅助忙等线程，不能把全部 CPU 差异归因于 Vulkan／Metal 或 shader。
最大 RSS 分别为 247.7–250.4 MiB 和 310.3–312.0 MiB，不含完整系统 GPU 内存占用。

当前 GPU device utilization 的逐轮中位数为 51%、51%、45%，#459 为 36%、47%、31%。
旧合成器历史记录约 99%。这个系统级 AGX 计数器不是逐进程 GPU 时间，
也不是 VideoToolbox 编码单元利用率，不能仅凭该百分比判断性能或画质。

## 正确性与 CPU 帧回读

共享实现直接读取 NV12 底图和 Skia 预乘 RGBA，输出 NV12；Windows 使用多平面视图，
MoltenVK 使用 R8/RG8 分离平面。两端共用 shader、调度、帧时间线和有界异步资源。
原 MediaPlan 和编码适配器保持不变，正式导出默认使用新路径，缓存标识同步升级。

- 原始 GPU oracle：38 帧、10,260 个样本，逐个与独立双精度 BT.709 参考比较，
  最大字节差 **0**；透明区逐字节保持底图。覆盖原色、黑白、半透明、混合覆盖、
  有／无底图、步长填充、非工作组整数尺寸和三帧排队后的短尾。
- 正式 Skia → compute → VideoToolbox 的十秒导出审计：603 次 CPU→GPU 上传
  （600 帧正文加 3 帧能力探测）、**0 次 GPU→CPU 下载、0 次硬件帧映射**。
  同一审计在刻意回读的 oracle 中捕获 38 次下载，证明拦截有效。
  这些是 FFmpeg API 计数，另检查了 Skia／Metal 适配器没有读像素／锁定像素基址。
- 此处“无 CPU 帧回读”不等于消除全部复制：CPU 解码后的 NV12 上传、Mac 上 GPU
  NV12→IOSurface blit 仍存在。测试 oracle 自己的回读不属于正式导出路径。
- 7 项正式 GPU 集成测试通过：H.264／HEVC、同步／异步、1/2/3/7 帧尾部、运动视频、
  音频、项目缓存、取消／失败时保留已有输出。原生库 147 项通过、2 项硬件测试默认忽略；
  GPU oracle 与项目 GPU 测试已额外显式执行。普通／headless 提交检查通过。
- Khronos 验证层没有新增 compute 调用诊断，但 FFmpeg 9.0.2 的设备创建仍触发
  `VUID-VkDeviceCreateInfo-pNext-02830`。独立 FFmpeg 设备创建也复现；详见日志，
  不能称为完全无验证层错误。Windows 本轮没有设备实测。

[原始颜色、生命周期和回读审计证据](../vulkan-compute-validation/validation/macos-correctness.json)
及[shader 合约与复现方法](../vulkan-compute-validation/README.md)。

全部 **10 个产物**（8 个完整视频、2 个预热）通过完整解码：共 **145,200 帧**，
PTS 网格最大误差小于 0.34 微秒，所有音频包载荷与源文件一致。每个完整视频的
8 个文字／表情采样点均通过，包含 299.9 秒附近的尾部。
[逐次完整回执](validation/macos-compute-8c9016cf.json)、
[控制组汇总](validation/macos-compute-summary-8c9016cf.json)。

## 色度与画质边界

新合成器采用覆盖率加权的 2×2 色度 box average，保留现有编码器色度元数据。
它不复刻 libplacebo 通用重采样核，细小彩色边缘可能不同；不宣称逐像素等价。
不会为这个固定 SDR 操作额外做线性光混合、色调映射或 RGB 视频中间帧。

两边保留各自默认编码策略：当前 GOP 120 / B=0，#459 GOP 240 / HEVC B=2。
本轮测的是默认导出体验，**不是等画质或相同编码配置的竞赛**。
完整流与内容采样用于排除丢帧、音频变化和漏画；有损编码后的 RGB 误差不能替代
严格的感知画质评测，也不把“比缺失弹幕更接近参考”当作无损证明。

## 复现

固定素材和源视频 SHA-256、各二进制 SHA-256、实际命令、逐轮阶段／遥测及产物校验
保存在同目录的 JSON 回执。#459 的计时工具见 [profiling/README.md](profiling/README.md)。

```sh
python3 scripts/export-comparison/compare_stages.py \
  --current /private/tmp/bsr-vulkan-compute/Compute.app/Contents/MacOS/bili-shadowreplay \
  --pr459 /private/tmp/bsr-macos-stage-optimization/pr459-profile \
  --telemetry /private/tmp/bsr-macos-stage-optimization/telemetry-gpu \
  --output /tmp/compute-comparison-new-run --revision 8c9016cf \
  --pairs 1 --controls 3 \
  --references /private/tmp/bsr-macos-stage-optimization/baseline-native/results.json \
  --defer-validation
```

这里引用的 `/private/tmp` 路径是本机原始证据位置，不是仓库内置二进制。
产物只在验证通过并记录哈希后删除；原始日志和采样图仍在本机比较目录。
