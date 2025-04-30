## Docker 部署

BiliBili ShadowReplay 支持无界面模式，提供 Web 控制界面，可以用于在服务器等无图形界面环境下部署使用。

使用方法：

```bash
sudo docker run -it -d\
    -p 3000:3000 \
    -v $DATA_DIR:/app/data \
    -v $CACHE_DIR:/app/cache \
    -v $OUTPUT_DIR:/app/output \
    -v $CONFIG_FILE:/app/config.toml \
    --name bili-shadowreplay \
    ghcr.io/xinrea/bili-shadowreplay:latest
```
