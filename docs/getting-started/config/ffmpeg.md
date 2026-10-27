# FFmpeg 配置

FFmpeg 是一个开源的音视频处理工具，支持多种格式的音视频编解码、转码、剪辑、合并等操作。
在本项目中，FFmpeg 用于切片生成以及字幕和弹幕的硬编码处理，因此需要确保安装了 FFmpeg。

## MacOS

在 MacOS 上安装 FFmpeg 非常简单，可以使用 Homebrew 来安装：

```bash
brew install ffmpeg
```

如果没有安装 Homebrew，可以参考 [Homebrew 官网](https://brew.sh/) 进行安装。

macOS 的弹幕压制使用 Skia Metal、Metal 叠加和 VideoToolbox，需安装
**FFmpeg 8.0 或更高版本的 Homebrew 共享库**。运行时的库 ABI 必须与应用构建时包含的版本一致；
只复制 `ffmpeg` 可执行文件不能满足这个要求。macOS 发布包包含 FFmpeg 8 和 9 两套 ABI 适配器。
本地默认构建只包含当前 `ffmpeg` 的一个 ABI；显式设置 `BSR_FFMPEG_PREFIXES` 可构建多个版本，见
[原生渲染构建](../../development/native-overlay.md#macosmetal-弹幕压制)。

在设置的“弹幕压制编码”中选择 H.264/H.265 和目标码率；macOS 默认使用
HEVC、10 Mbps、GOP 120、速度优先。速度优先开关仅在当前编码格式有可用的
VideoToolbox 编码器时显示。选项用于素材和整场录播的弹幕压制；项目导出使用项目自己的输出设置。

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
