# FFmpeg 配置

FFmpeg 是一个开源的音视频处理工具，支持多种格式的音视频编解码、转码、剪辑、合并等操作。
在本项目中，FFmpeg 用于切片生成以及字幕和弹幕的硬编码处理，因此需要确保安装了 FFmpeg。

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
