# 安装准备

## 桌面端安装

桌面端目前提供了 Windows、Linux 和 MacOS 三个平台的安装包。

安装包分为两个版本，普通版和 debug 版，普通版适合大部分用户使用，debug 版包含了更多的调试信息，适合开发者使用；由于程序会对账号等敏感信息进行管理，请从信任的来源进行下载；所有版本均可在 [GitHub Releases](https://github.com/Xinrea/bili-shadowreplay/releases) 页面下载安装。

### Windows

由于程序内置 Whisper 字幕识别模型支持，Windows 版本分为两种：

- **普通版本**：内置了 Whisper GPU 加速，字幕识别较快，体积较大，只支持 Nvidia 显卡
- **CPU 版本**： 使用 CPU 进行字幕识别推理，速度较慢

请根据自己的显卡情况选择合适的版本进行下载。

### Linux

Linux 版本目前仅支持使用 CPU 推理，且测试较少，可能存在一些问题，遇到问题请及时反馈。

### MacOS

MacOS 版本内置 Metal GPU 加速；安装后首次运行，会提示无法打开从网络下载的软件，请在设置-隐私与安全性下，选择仍然打开以允许程序运行。

## Docker 部署

BiliBili ShadowReplay 提供了服务端部署的能力，提供 Web 控制界面，可以用于在服务器等无图形界面环境下部署使用。

### 镜像获取

```bash
# 拉取最新版本
docker pull ghcr.io/xinrea/bili-shadowreplay:latest
# 拉取指定版本
docker pull ghcr.io/xinrea/bili-shadowreplay:2.5.0
# 速度太慢？从镜像源拉取
docker pull ghcr.nju.edu.cn/xinrea/bili-shadowreplay:latest
```

### 镜像使用

使用方法：

```bash
sudo docker run -it -d\
    -p 3000:3000 \
    -v $DATA_DIR:/app/data \
    -v $CACHE_DIR:/app/cache \
    -v $OUTPUT_DIR:/app/output \
    -v $WHISPER_MODEL:/app/whisper_model.bin \
    --name bili-shadowreplay \
    ghcr.io/xinrea/bili-shadowreplay:latest
```

其中：

- `$DATA_DIR`：为数据目录，对应于桌面版的数据目录，

  Windows 下位于 `C:\Users\{用户名}\AppData\Roaming\cn.vjoi.bilishadowreplay`;

  MacOS 下位于 `/Users/{user}/Library/Application Support/cn.vjoi.bilishadowreplay`

- `$CACHE_DIR`：为缓存目录，对应于桌面版的缓存目录；
- `$OUTPUT_DIR`：为输出目录，对应于桌面版的输出目录；
- `$WHISPER_MODEL`：为 Whisper 模型文件路径，对应于桌面版的 Whisper 模型文件路径。
