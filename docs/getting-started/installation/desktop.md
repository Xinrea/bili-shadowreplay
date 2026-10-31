# 桌面端安装

桌面端目前提供了 Windows、Linux 和 MacOS 三个平台的安装包。

由于程序会对账号等敏感信息进行管理，请从信任的来源进行下载；所有版本均可在 [GitHub Releases](https://github.com/Xinrea/bili-shadowreplay/releases) 页面下载安装。

Windows 和 macOS 安装包已携带媒体处理运行库，无需单独安装 `ffmpeg` 或 `ffprobe`。Linux 使用匹配的系统共享库，详情见 [FFmpeg 配置](../config/ffmpeg.md)。

## Windows

由于程序内置 Whisper 字幕识别模型支持，Windows 版本分为两种：

- **普通版本**：内置了 Whisper GPU 加速，字幕识别较快，体积较大，只支持 Nvidia 显卡
- **CPU 版本**： 使用 CPU 进行字幕识别推理，速度较慢

请根据自己的显卡情况选择合适的版本进行下载。

## Linux

Linux 桌面安装包最低支持 **Ubuntu 24.04**，不再支持 Ubuntu 22.04。其他发行版需要提供兼容的系统运行库，不能仅凭发行时间判断兼容性；AppImage 也不保证能在旧系统上运行。

程序的原生弹幕渲染依赖系统 FreeType、HarfBuzz 和 ICU 等运行库，以支持 COLRv1 彩色 Emoji，并避免与 V8 内置 ICU 的符号冲突。`.deb` 包会声明这些依赖，推荐使用 `sudo apt install ./下载的文件名.deb` 安装，由包管理器一并安装依赖；不要用忽略依赖的方式强行安装到 Ubuntu 22.04。

Linux 版本目前仅支持使用 CPU 推理，且测试较少，可能存在一些问题，遇到问题请及时反馈。需要自行编译时，请参考 [原生弹幕渲染构建](../../development/native-overlay.md)。

## MacOS

MacOS 版本内置 Metal GPU 加速；安装后首次运行，会提示无法打开从网络下载的软件，请在设置-隐私与安全性下，选择仍然打开以允许程序运行。
