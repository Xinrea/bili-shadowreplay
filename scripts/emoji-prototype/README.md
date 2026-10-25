# 彩色 emoji 渲染原型

这是一个**独立、可复跑的验证原型**，不导入应用代码。它验证的目标是：将中文/英文和完整 Unicode emoji 序列统一排版为透明图层，再交给 FFmpeg 合成视频。正式实现位于 `src-tauri/crates/native-overlay`，应用的生成字幕预览/压制及弹幕压制已经接入该核心。

## 运行

依赖：

- Python 3（仅使用标准库）
- Rust/Cargo（本机使用 Rust 1.90）
- `curl`
- FFmpeg 和 `ffprobe`，需要 `libx264` 编码器

在仓库根目录执行：

```sh
python3 scripts/emoji-prototype/run.py
```

脚本会：

1. 下载固定提交版本的 Noto CJK、Noto COLRv1 彩色 emoji 字体，以及 Unicode 16.0 测试数据；
2. 校验每个文件的 SHA-256；
3. 构建并运行 Rust/Skia 原型和单元测试；
4. 检查 3,781 个 fully-qualified emoji 序列是否能各自形成一个字形；
5. 对比 Skia 的 PNG 编码与 raw RGBA readback，并检查 raw RGBA 在黑/白背景和 0/0.8/1.0 透明度下的合成结果；
6. 生成 24 条和 100 条并发弹幕精灵，按 1080p/60 FPS 逐帧合成；
7. 将未压缩 RGBA 帧通过 stdin 交给 FFmpeg，编码并严格解码验证 6 秒 H.264 视频，并在实际编码帧中检查字幕切换边界。

Cargo 会下载匹配版本的 Skia 预编译库。保留 `skia-safe` 默认功能是为了匹配上游提供的二进制组合；原型本身不使用 PDF 或 GPU。若目标平台没有匹配的预编译库，Cargo 可能转为耗时的 Skia 源码编译。

只下载资源、不编译：

```sh
python3 scripts/emoji-prototype/run.py --fetch-only
```

只验证字体、排版、alpha 和单元测试，不编码视频：

```sh
python3 scripts/emoji-prototype/run.py --skip-video
```

`assets/`、`output/` 和 Cargo 的 `target/` 都是原型生成物，不应提交。字体和 Unicode 数据的许可证文件会一并下载到 `assets/`。

## 原型结构

```text
Unicode 文本
    ↓
Unicode grapheme 分段 + emoji-test 序列识别
    ↓
SkParagraph：Body / Emoji 两种字体 run
    ↓
Skia CPU raster：彩色 emoji、正文、描边、阴影、透明背景
    ↓
按时间移动已缓存的 SkImage 精灵
    ↓
RGBA pipe → FFmpeg overlay → H.264
```

原型显式关闭系统字体 fallback，并固定加载两种字体，因此成功结果不依赖开发机是否安装了 emoji 字体。`NotoColorEmoji.ttf` 等位图字体没有作为方案使用；当前 macOS Skia 后端对它加载失败，而 `Noto-COLRv1.ttf` 可以正确渲染 COLRv1 彩色字形。

原型按完整 grapheme 处理肤色修饰符、ZWJ 家庭/职业序列、旗帜、变体选择符和键帽，不能把这个识别逻辑直接理解为最终产品级 emoji 解析器。生产实现仍应明确处理字体版本、Unicode 版本、未知序列 fallback 和缓存淘汰。

## 最初的 macOS 基线

以下是最初使用 skia-safe 0.90.0 在 macOS ARM64、Apple M2 Max、Rust 1.90、FFmpeg 9.0.2 上完成的 CPU raster 结果。之后为了兼容现有 Windows 工具链，原型与正式实现统一固定为 0.82.0；不要把这组历史数字当作版本间基准比较。

- Unicode 16.0：3,781 个 fully-qualified 序列，**缺失字形数均为 0，且每个序列均形成一个字形**。这不是对所有 emoji 美术内容逐一截图验收；混排、肤色、家庭/职业、国旗、键帽等代表性样例另做了图像查看。
- 缺失码点测试能被显式检测，不会静默改用系统字体。
- 彩色像素、透明像素和抗锯齿边缘均存在；RGBA straight-alpha 合成在黑/白背景、0/0.8/1.0 透明度下的最大通道误差为 **0**。
- 200 条不同消息的排版、渲染、描边和阴影平均约 **4.85 ms/条**；首个样例约 **16.81 ms**。因此动画复用精灵，不每帧重绘文本。
- 24 条精灵：合成和 RGBA readback 平均 **2.69 ms/frame**，P95 **3.69 ms**，精灵像素缓存约 **4.27 MiB**。
- 100 条精灵：平均 **4.50 ms/frame**，P95 **5.81 ms**，精灵像素缓存约 **15.21 MiB**。
- 两组均编码为 1920×1080、60 FPS、360 帧、6 秒视频，并通过 `ffmpeg -xerror` 全量解码检查；端到端编码速度分别约 **152 FPS**（24 条）和 **112 FPS**（100 条），包括初始精灵生成与管道传输。
- 检查视频第 149、150、179、180 帧，确认第一条字幕在 2.5 秒消失、第二条在 3 秒出现，中间没有残留字幕。

结果文件在 `scripts/emoji-prototype/output/`：

- `report.json`：总报告；
- `fixtures.json`：字体加载、排版、Unicode 序列和冷/热渲染测试；
- `alpha.json`：透明度和合成检查；
- `contact-sheet.png`：黑白背景对照；
- `video-24.png`、`video-100.png`：视频抽帧；
- `demo-24.mp4`、`demo-100.mp4`：实际 FFmpeg 输出。

## Windows 验证与工具链兼容

在用户提供的 Windows x64 / build 26200、Ryzen 9 5900X、Rust 1.89、MSVC 14.42.34433、Gyan FFmpeg 9.0.2 环境中验证：

- 原型完整脚本通过：3 个单元测试、3,781 个 emoji 序列检查、raw/PNG 与六组 alpha 合成均通过（最大通道误差 0）。
- 24 / 100 精灵分别完成 6 秒、1080p60、360 帧视频，严格全量解码和四个字幕边界帧检查通过。
- 合成及 RGBA 读回平均耗时分别为 **5.78 / 8.10 ms**，P95 **6.92 / 9.76 ms**；端到端速度约 **99 / 79 FPS**。
- 正式 `native-overlay` 核心也在 Windows 独立编译并通过 **21 项测试**，包含字体、组合 emoji、alpha、图片比例、运动、缓存和时序。
- 未修改 Windows 仓库和全局开发环境；运行目录为 `%TEMP%/bsr-color-emoji-prototype-20261025`。这不是整个 Windows 应用安装包的验收。

兼容性修正：

1. skia-safe 0.90 官方预编译库引用本机旧 MSVC 没有的 `__std_search_1`；改为 0.82，避免强制升级 VS。
2. 0.82 的 ICU 需要显式链接 Windows 系统库 `advapi32`；由 `build.rs` 处理。
3. Python 捕获 Cargo JSON/读取报告需显式 UTF-8，不能依赖 Windows GBK locale。构建失败时要输出 Cargo JSON 内的诊断，否则会隐藏真正的链接错误。
4. Windows 无法顺畅下载 GitHub release 时，可以先下载**同版本、同目标、同 feature**的官方 Skia 压缩包，再为当前构建进程设置 `SKIA_BINARIES_URL`。0.82 的本地文件读取器直接去掉 `file://` 前缀，Windows 形式为 `file://C:/path/skia.tar.gz`，不是 `file:///C:/...`。不要改变用户的永久环境变量。

## 结论与下一步

原型证明了：**不使用 libass，Skia/SkParagraph 可以完成中文混排、彩色 emoji、描边/阴影、透明图层和 FFmpeg 视频合成。** 当前 CPU 方案在 100 条并发精灵的受控测试中达到 60 FPS 以上，因此技术路线可行。

此原型仍使用固定测试场景；正式核心的接入与其分开维护。后续验证重点：

1. 设计有界的 sprite/glyph 缓存和淘汰策略；
2. 将实际字幕样式、弹幕速度/轨道和现有图片表情合成接入统一 compositor；
3. 在 Windows、Linux 和目标发行版 FFmpeg 上重复 alpha、字体和性能测试；
4. 决定应用随包携带的彩色字体及其 Unicode 版本；
5. 比较 CPU readback/pipe 与更直接的 FFmpeg/硬件接入方式，避免超高分辨率时的带宽瓶颈。

### 尚未验证的边界

- 视频背景是固定的深/浅色测试图，不含真实视频解码、音频、转场或硬件编解码开销。
- 24/100 是提交给 compositor 的固定精灵数量，不等于每一帧均完整可见的真实弹幕数量；部分精灵在画面外，100 条场景允许重叠，不验证碰撞调度。
- 每段仅 6 秒，没有验证长视频内存增长、持续高频新消息、缓存淘汰或 4K。
- 当前 RGBA pipe 在 1080p60 下传输约 **498 MB/s**，不能由本次结果直接外推 4K 性能。
- 缓存数字只统计精灵像素，不包括 Skia 内部缓存、整帧缓冲、字体、进程和 FFmpeg 内存。
- 变体 VS15 使用正文字体，VS16 使用彩色字体；裸文本符号的默认 presentation、未知/更新版本 emoji 及缺字降级策略仍需产品化。本原型发现缺字时明确失败，不作为应用的最终 fallback 行为。
- 不是任意外部 ASS 特效的重实现；未改变当前预览、ASS 导出或压制结果。
