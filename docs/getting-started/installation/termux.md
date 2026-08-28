# Termux 部署

BiliBili ShadowReplay 的 headless 模式提供 `linux/arm64` 容器镜像，可以在
64 位 ARM Android 设备上通过 Termux 和 `proot-distro` 运行。此方案不需要
root 权限，也不需要在 Android 上启动 Docker daemon。

> [!IMPORTANT]
> Termux 使用 Android 的 bionic C 库，并不是普通的 GNU/Linux 环境。当前
> 镜像中的程序是 glibc 二进制，因此必须在 `proot-distro` 提供的 Linux
> 用户空间中运行，不能直接从 Termux shell 启动。

## 环境要求

- 从 F-Droid 或 Termux GitHub Releases 安装的新版 Termux；
- `uname -m` 输出 `aarch64`；
- 建议至少预留 4 GB 存储空间。启用字幕功能时还需要为 Whisper 模型和运行
  内存预留额外空间；
- Android 应允许 Termux 在后台运行，否则录制进程可能被系统终止。

## 安装

在 Termux 中执行：

```bash
pkg update
pkg install proot-distro tmux

mkdir -p "$HOME/bsr"/{data,cache,output}
proot-distro install \
  --name bili-shadowreplay \
  --architecture linux/arm64 \
  ghcr.io/xinrea/bili-shadowreplay:latest
```

`proot-distro` 会根据镜像清单拉取 ARM64 版本，不会在手机上模拟 x86。

## 启动

用 `tmux` 保持服务在终端退出后继续运行：

```bash
tmux new -s bsr

proot-distro login bili-shadowreplay \
  --bind "$HOME/bsr/data:/app/data" \
  --bind "$HOME/bsr/cache:/app/cache" \
  --bind "$HOME/bsr/output:/app/output" \
  -- /bin/sh -lc 'cd /app && exec ./bili-shadowreplay'
```

服务启动后，在手机浏览器中打开 <http://127.0.0.1:3000>。这里不需要 Docker
的 `-p` 端口映射。按 `Ctrl+B`、再按 `D` 可以退出 `tmux` 而不停止服务；
之后使用 `tmux attach -t bsr` 返回。

配置文件位于容器内的 `/app/config.toml`，数据库、缓存和输出分别持久化到
`$HOME/bsr/data`、`$HOME/bsr/cache` 和 `$HOME/bsr/output`。需要长期保留或
手动编辑配置时，也应把配置文件复制到 Termux 目录并通过 `--bind` 挂载。
