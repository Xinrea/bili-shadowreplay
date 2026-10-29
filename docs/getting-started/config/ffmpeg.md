# FFmpeg 配置

FFmpeg 是一个开源的音视频处理工具，支持多种格式的音视频编解码、转码、剪辑、合并等操作。
在本项目中，元数据、音频时间信息和 HLS 分片探测，以及波形、转写用完整 WAV 和 30 秒 WAV 分块，通过 `ffmpeg-next` 直接链接 `libavformat`、`libavcodec`、`libavutil` 和 `libswresample`。录制、转码、范围音频提取、Opus/FLAC 编码及字幕弹幕压制继续使用 FFmpeg CLI；部分回归测试也用 CLI 对照结果。

Windows 和 macOS 安装包携带探测所需的动态库。macOS 仍需按下文安装 FFmpeg CLI，以使用录制和转码功能；Linux 使用目标发行版的 FFmpeg 运行库，`.deb` 会声明对应依赖。

## MacOS

在 MacOS 上安装 FFmpeg 非常简单，可以使用 Homebrew 来安装：

```bash
brew install ffmpeg
```

如果没有安装 Homebrew，可以参考 [Homebrew 官网](https://brew.sh/) 进行安装。

## Linux

在 Linux 上安装 FFmpeg 可以使用系统自带的包管理器进行安装，例如：

- Ubuntu/Debian 系统：

```bash
sudo apt install ffmpeg
```

- Fedora 系统：

```bash
sudo dnf install ffmpeg
```

- Arch Linux 系统：

```bash
sudo pacman -S ffmpeg
```

- CentOS 系统：

```bash
sudo yum install epel-release
sudo yum install ffmpeg
```

### Docker VAAPI 硬件编码

在 Linux Docker 容器中使用核显 VAAPI 编码时，需要把宿主机的 `/dev/dri` 映射到容器中，并确保容器内 FFmpeg 支持 `h264_vaapi` 编码器。程序会自动检测 `/dev/dri/renderD*` 设备，并在测试和转码时追加 `-vaapi_device` 与 `format=nv12,hwupload` 参数。

## Windows

Windows 版本安装后，FFmpeg 已经放置在了程序目录下，因此不需要额外安装。


## 从源码构建

`ffmpeg-next` 需要 FFmpeg 开发头文件和动态库，只有 `ffmpeg`/`ffprobe` 可执行文件不能编译探测层。绑定使用 Clang 生成，开发库必须匹配目标架构；升级 FFmpeg 主版本后需重新编译应用。

### macOS

```bash
brew install ffmpeg pkg-config
export PKG_CONFIG_PATH="$(brew --prefix ffmpeg)/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
cargo test --manifest-path src-tauri/Cargo.toml -p ffmpeg_utils
```

如果使用 Homebrew 的 `ffmpeg-full`，把上面的 `brew --prefix ffmpeg` 改为 `brew --prefix ffmpeg-full`。不应把机器上的 `/opt/homebrew` 或 Cellar 绝对路径提交到共享 Cargo 配置。Intel 和 Apple Silicon 发布任务分别固定使用 `macos-15-intel` 和 `macos-15` 原生 runner，保持产品的 macOS 15 最低版本。

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
sudo apt install clang libclang-dev pkg-config libavformat-dev libavcodec-dev libavutil-dev libswresample-dev ffmpeg
```

CI 和 Docker builder 已安装这些开发包。Docker runtime 保持与 builder 相同的 Debian trixie 基线，通过发行版 `ffmpeg` 包获得匹配的运行库；镜像构建会用 `ldd` 检查所有动态依赖。Ubuntu 24.04 桌面 `.deb` 显式依赖 `libavformat60`、`libavcodec60`、`libavutil58` 和 `libswresample4`；其他发行版需使用自身匹配的包，不能直接照搬这些 ABI 名称。

### Windows

在准备好 Rust、LLVM/Visual Studio 和 7-Zip 的 PowerShell 构建终端中运行：

```powershell
./ffmpeg_setup.ps1
cargo test --manifest-path src-tauri/Cargo.toml -p ffmpeg_utils
```

脚本下载并校验固定版本 Gyan FFmpeg 9.0.2 shared SDK，保留头文件和导入库到 `src-tauri/target/ffmpeg-sdk`，在当前终端设置 `FFMPEG_DIR` 和运行 DLL 搜索路径；GitHub Actions 中也会写入后续步骤环境。DLL 复制到现有 `windows-runtime` 资源目录，与 CLI 一起随安装包发布。不要用原来的 essentials 可执行文件压缩包替代 shared SDK。更换构建终端后需要重新运行脚本，或显式设置同一个 SDK 的 `FFMPEG_DIR`、`PATH` 和 `LIBCLANG_PATH`。
