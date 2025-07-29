# Docker 部署

BiliBili ShadowReplay 提供了服务端部署的能力，提供 Web 控制界面，可以用于在服务器等无图形界面环境下部署使用。

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

- `$CACHE_DIR`：为缓存目录，对应于桌面版的缓存目录；
- `$OUTPUT_DIR`：为输出目录，对应于桌面版的输出目录；
- `$WHISPER_MODEL`：为 Whisper 模型文件路径，对应于桌面版的 Whisper 模型文件路径。
