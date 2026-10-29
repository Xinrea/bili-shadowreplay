# FFmpeg 配置

FFmpeg 是一个开源的音视频处理工具，支持多种格式的音视频编解码、转码、剪辑、合并等操作。
在本项目中，元数据、音频时间信息和 HLS 分片探测，以及波形、转写用完整 WAV、30 秒 WAV 分块、范围 WAV 提取、素材和 PowerLive 的 Opus 编码、编辑器预览 FLAC 分块、音频库导入、MP3 导出、缩略图、视频提帧、FLV 到 HLS 的录制，以及单输入视频转封装、素材转码和复制／编码裁剪，通过 `ffmpeg-next` 直接链接 `libavformat`、`libavcodec`、`libavutil`、`libswresample` 和 `libavfilter`。跨输入滤镜拼接、项目合成及 SRT 字幕和弹幕画面合成也使用共享的原生滤镜图和编码管线。正常运行不启动 `ffmpeg`、`ffprobe` 或 `ffplay` 进程；部分开发回归测试可用匹配 SDK 的 CLI 作独立对照。

范围 WAV 提取保留原 `-ss ... -i ... -t ...` 的时间行为，包括 AAC 预滚、容器起点、采样率变化及末帧边界。输出固定为 16 kHz 单声道 PCM；由于沿用 CLI 的逐帧截止规则，样本数不保证恰好等于请求秒数乘以采样率。

素材 Opus 音频保留原来的 16 kHz 单声道、64 kbps VBR、压缩级别 10 配置，通过 Ogg 封装交给浏览器播放。输入的浮点／整数格式协商、音轨延迟和时间戳间隙会保留；编码失败或取消不会替换已有输出。

PowerLive 上传音频使用 128 kbps VBR，保留源声道布局，采样率由滤镜协商为编码器支持的值。首次协商后会固定输出格式，即使输入流中途改变采样率或声道数，也保持同一条 Opus 音轨。其时间戳、标签、章节和尾部补齐处理与素材音频共用原生管线；该调用不再启动 FFmpeg 子进程。

编辑器预览按首音轨的样本网格裁剪，保留 AAC 预滚，输出 48 kHz 双声道 16 位 FLAC。正常分块使用固定的 4096 样本编码块，尾块可短于该长度，空结果只写文件头；编码失败或取消不会发布半成品。实现统一采用 FFmpeg 9 的首帧块长规则：FFmpeg 6.1 旧 CLI 默认会重新拼成 4608 样本，原生输出改用 4096；不足 16 个输出样本的非空短片按编码器规则报错。

音频库始终导入第一音轨：16 位 WAV、FLAC 与 AAC 直接转封装，其他 PCM WAV 转成 16 位，其他编码转成 256 kbps AAC；M4A 写入 faststart 索引。MP3 导出沿用自动选轨和 libmp3lame 的 q2 质量配置，保留延迟、尾部 padding 和 Xing/Info 信息。WAV 转写分块只有原生路径，不再保留未使用的 MP3 分块命令。

缩略图保留默认 JPEG 质量，面向客户端的提帧保留 q2，按输入时间定位并处理旋转、颜色范围及像素比例；图片成功编码后才原子发布，base64 提帧直接返回内存中的 JPEG。FFmpeg 6.1 对单图 `seek(0)` 的丢帧行为被修正：单图零时刻直接解码，视频仍执行准确定位。

FLV 直播录制直接复制音视频包到 event HLS，保留请求头、User-Agent、读取超时、四秒目标分片和已有列表追加行为，忽略字幕流。关闭录制开关会中断网络读取并完成最后一个分片，再关闭播放列表；任务取消则中断输入／输出。录制器在结束时补报最后分片的时长和大小，避免快速结束或尾段未进入定时进度更新。

单输入视频操作使用结构化媒体计划。复制裁剪保留关键帧预滚；编码裁剪区分输入定位与输出裁剪，保留帧率同步、旋转、颜色、SAR、负时间戳和编码器延迟。音视频及字幕编码包逐个进入封装器；临时输出在文件尾和关闭成功后原子发布，最后才报告完成。MKV 沿用容器默认 ASS 字幕编码，MP4 沿用自动字幕选轨规则；字幕事件保留旧 CLI 的定位和时长行为，不会按音视频滤镜的规则重新截断事件。

兼容完整片段的无损拼接和无转场合并使用原生 concat demuxer，保留时长提示、H.264 参数集转换和 MPEG-TS 默认封装延迟。临时列表由工作线程保留至输入关闭，任务取消后也会清理；不兼容片段、时间轴范围片段与转场也已迁移到共享原生滤镜图，逐输入解码以保留 AAC 预滚和 edit list 行为，支持缺失音轨补静音以及 xfade／acrossfade。

外部 ASS 字幕通过原生 `libavfilter` 的 `subtitles` 滤镜烧录，保留路径转义、`force_style`、`fontsdir`、缩放填充和音轨复制行为。链接的 SDK 必须启用 libass；不会改用 PATH 上的可执行文件补齐组件。SRT 和弹幕使用原生字体渲染器生成有界 BGRA 帧流，经同一原生滤镜图叠加；渲染器与工作线程共同持有临时素材，取消时队列两端都会被唤醒。

弹幕表情图片也在进程内标准化：仅接受不超过 8 MiB 的本地 PNG/APNG、JPEG、GIF、WebP 普通文件，画布不超过 4,194,304 像素，单线程提取首帧并缩放为不大于 512×512 的 RGBA PNG。压缩 PNG 文本和 ICC 元数据的解压总量另限 8 MiB。失败、取消或十秒超时不会覆盖已有图片；原生调用协作取消，不能强制终止正在运行的解码器。输入限制不等于每任务的硬内存上限，也不会修改进程全局的 FFmpeg 分配限制。

Windows 和 macOS 安装包携带原生媒体处理所需的动态库，不要求用户安装 CLI。Linux 使用目标发行版的匹配共享库，`.deb` 会声明对应依赖，Docker 镜像直接安装同一发行版的运行库。

启动日志中的版本和可编译编码器列表直接读取所链接的 FFmpeg SDK，与 `PATH` 上的可执行文件无关。硬件可用性通过实际打开编码器、编码三帧并排空来探测，保留原候选顺序、缓存和软件回退策略；VAAPI 直接建立硬件设备和帧池。已注册的编码器仍可能因为设备或驱动不可用而探测失败。

## MacOS

安装官方 `.app` 即可使用，所需共享库位于应用的 `Contents/Frameworks` 中；无需安装 Homebrew 或命令行 FFmpeg。自行构建或运行 CLI 对照测试时，参考后面的源码构建说明。

## Linux

桌面 `.deb` 最低基线为 Ubuntu 24.04。推荐使用 `sudo apt install ./下载的文件名.deb`，由包管理器安装匹配的共享库。Docker 镜像已经包含这些运行库；其他发行版或自定义 AppImage 必须提供匹配 ABI 和所需编解码器，不能照搬 Ubuntu 的库包名。

### Docker VAAPI 硬件编码

在 Linux Docker 容器中使用核显 VAAPI 编码时，需要把宿主机的 `/dev/dri` 映射到容器中，并提供兼容的用户态驱动及带 `h264_vaapi` 的 FFmpeg 共享库。程序会检测 `/dev/dri/renderD*`，直接建立硬件设备和帧池，通过 `format=nv12,hwupload` 滤镜上传；硬件可用性由实际编码测试确认。

## Windows

Windows 安装包中的 FFmpeg 资源为原生媒体运行 DLL；不再捆绑 `ffmpeg.exe`、`ffprobe.exe` 或 `ffplay.exe`，也无需用户另行安装。


## 从源码构建

`ffmpeg-next` 需要 FFmpeg 开发头文件和动态库，只有 `ffmpeg`/`ffprobe` 可执行文件不能编译原生媒体层。绑定使用 Clang 生成，开发库必须匹配目标架构；升级 FFmpeg 主版本后需重新编译应用。

可选 CLI 对照测试需另将同一个 SDK 的 `ffmpeg` 和 `ffprobe` 放到 `PATH`；基于已存 fixture 的原生测试无需可执行文件。

所链接的 SDK 还必须启用 `libopus`、`libmp3lame`、`libx264`、`libx265`、AAC、FLAC、PCM S16、MJPEG、PNG 和所需字幕编码器，以及 Ogg/Opus、MP3、WAV、FLAC、M4A muxer。系统 `ffmpeg` 可执行文件具备这些能力，不代表另一个自定义 SDK 也已启用；原生实现缺少组件时会返回错误。

### macOS

```bash
brew install ffmpeg-full pkg-config
export PKG_CONFIG_PATH="$(brew --prefix ffmpeg-full)/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
# 独立 CLI 对照测试应使用同一个 SDK，避免误用已有的其他版本。
export PATH="$(brew --prefix ffmpeg-full)/bin:$PATH"
cargo test --manifest-path src-tauri/Cargo.toml -p ffmpeg_utils
```

构建示例使用带 libass 的 `ffmpeg-full`；普通 `ffmpeg` formula 未必启用所需字幕滤镜。自定义 SDK 也必须满足上述组件要求。不应把机器上的 `/opt/homebrew` 或 Cellar 绝对路径提交到共享 Cargo 配置。Intel 和 Apple Silicon 发布任务分别固定使用 `macos-15-intel` 和 `macos-15` 原生 runner，保持产品的 macOS 15 最低版本。

`tauri.macos.conf.json` 的 `beforeBundleCommand` 会运行 `scripts/bundle-macos-ffmpeg.mjs`，从最终可执行文件递归收集非系统动态库，将 install name 改为应用内部路径并签名，然后通过 `bundle.macOS.files` 放入 `Contents/Frameworks`。脚本会先读取可执行文件及每个传递依赖的 Mach-O 最低系统版本，若高于产品的 `minimumSystemVersion`（当前 15.0）则拒绝打包；验包时还会按包内 `LSMinimumSystemVersion` 复查。较新 macOS 上安装的 Homebrew bottle 可能要求 26/27，不能仅设置 `MACOSX_DEPLOYMENT_TARGET=15.0` 将其降级，应使用 macOS 15 发布 runner 或为 15 构建的完整 FFmpeg SDK 及匹配依赖。该步骤不修改 Homebrew 原件，也不会改写 Mach-O 最低版本来伪装兼容。打包后的检查命令为：

```bash
node scripts/bundle-macos-ffmpeg.mjs --verify /absolute/path/bili-shadowreplay.app
codesign --verify --deep --strict /absolute/path/bili-shadowreplay.app
```

使用显式 `--target` 打包时，同时设置同值的 `RUST_TARGET`，例如 `RUST_TARGET=aarch64-apple-darwin yarn tauri build --target aarch64-apple-darwin`。脚本会严格选择该目标的产物；没有显式目标时兼容常规 `target/release`，若两种产物同时存在则报歧义，避免打包旧文件。

自定义签名时设置 `APPLE_SIGNING_IDENTITY`，使动态库签名与应用身份一致。此脚本处理开发机 FFmpeg 的传递依赖，因此最终包大小取决于该构建启用了哪些组件；分发前应核对相应许可证、源码及构建说明。

### Linux / Docker

Ubuntu 24.04 / Debian 的开发依赖：

```bash
sudo apt install clang libclang-dev pkg-config libavformat-dev libavcodec-dev libavutil-dev libswresample-dev libavfilter-dev
```

CI 和 Docker builder 已安装这些开发包。Docker runtime 保持与 builder 相同的 Debian trixie 基线，安装 `libavformat61`、`libavcodec61`、`libavutil59`、`libswresample5`、`libavfilter10` 和 `libswscale8`，不安装 CLI 可执行文件；镜像构建会用 `ldd` 检查所有动态依赖。Ubuntu 24.04 桌面 `.deb` 显式依赖 `libavformat60`、`libavcodec60`、`libavutil58`、`libswresample4` 和 `libavfilter9`；其他发行版需使用自身匹配的包，不能直接照搬这些 ABI 名称。

### Windows

在准备好 Rust、LLVM/Visual Studio 和 7-Zip 的 PowerShell 构建终端中运行：

```powershell
./ffmpeg_setup.ps1
cargo test --manifest-path src-tauri/Cargo.toml -p ffmpeg_utils
```

脚本下载并校验固定版本 Gyan FFmpeg 9.0.2 shared SDK，保留头文件和导入库到 `src-tauri/target/ffmpeg-sdk`，在当前终端设置 `FFMPEG_DIR` 和运行 DLL 搜索路径；GitHub Actions 中也会写入后续步骤环境。DLL 复制到现有 `windows-runtime` 资源目录随安装包发布；SDK 内的可执行文件仅供开发者可选的独立对照测试，不会复制到应用资源。不要用原来的 essentials 可执行文件压缩包替代 shared SDK。更换构建终端后需要重新运行脚本，或显式设置同一个 SDK 的 `FFMPEG_DIR`、`PATH` 和 `LIBCLANG_PATH`。
