# Docker 部署

BiliBili ShadowReplay 提供了服务端部署的能力，提供 Web 控制界面，可以用于在服务器等无图形界面环境下部署使用。

官方镜像支持 `linux/amd64` 和 `linux/arm64`。Docker 会根据宿主机架构自动
选择对应镜像；在 Android Termux 中运行请参考 [Termux 部署](./termux)。

镜像的构建和运行环境统一使用 Debian trixie，以提供原生弹幕渲染所需的系统 FreeType、HarfBuzz 和 ICU。容器自带这些用户态依赖及匹配的 FFmpeg 共享库，正常运行不需要 `ffmpeg` 或 `ffprobe` 可执行文件，也不要求宿主机安装 Ubuntu 24.04 桌面运行库。自行修改镜像时，请保持 builder 与 runtime 的发行版基线一致，详见 [原生弹幕渲染构建](../../development/native-overlay.md)。

## 镜像获取

```bash
# 拉取最新版本
docker pull ghcr.io/xinrea/bili-shadowreplay:latest
# 拉取指定版本
docker pull ghcr.io/xinrea/bili-shadowreplay:2.5.0
# 速度太慢？从镜像源拉取
docker pull ghcr.nju.edu.cn/xinrea/bili-shadowreplay:latest
```

## 镜像使用

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

- `$CACHE_DIR`：为默认本地录制存储目录。保留 `/app/cache` 挂载位置以兼容旧录制；backend 配置和每场录播的绑定保存在 `$DATA_DIR` 中的数据库，通过设置页管理，不再以 TOML 的 `storage.backends.default.root` 为运行时来源。请同时保留数据库卷和录制文件卷；上传暂存和预览缓存改用容器的系统临时目录，不使用这个持久卷；
- `$OUTPUT_DIR`：为输出目录，对应于桌面版的输出目录；
- `$WHISPER_MODEL`：为 Whisper 模型文件路径，对应于桌面版的 Whisper 模型文件路径。
