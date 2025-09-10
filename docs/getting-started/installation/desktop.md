# 桌面端安装

桌面端目前提供了 Windows、Linux 和 MacOS 三个平台的安装包。

由于程序会对账号等敏感信息进行管理，请从信任的来源进行下载；所有版本均可在 [GitHub Releases](https://github.com/Xinrea/bili-shadowreplay/releases) 页面下载安装。

## Windows

由于程序内置 Whisper 字幕识别模型支持，Windows 版本分为两种：

- **普通版本**：内置了 Whisper GPU 加速，字幕识别较快，体积较大，只支持 Nvidia 显卡
- **CPU 版本**： 使用 CPU 进行字幕识别推理，速度较慢

请根据自己的显卡情况选择合适的版本进行下载。

## Linux

Linux 版本目前仅支持使用 CPU 推理，且测试较少，可能存在一些问题，遇到问题请及时反馈。

## MacOS

MacOS 版本内置 Metal GPU 加速；安装后首次运行，会提示无法打开从网络下载的软件，请在设置-隐私与安全性下，选择仍然打开以允许程序运行。
