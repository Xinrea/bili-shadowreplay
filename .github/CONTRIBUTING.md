# BiliBili-ShadowReplay contribute guide

## Project Setup

### MacOS

项目无需额外配置，直接 `yarn tauri dev` 即可编译运行。

### Linux

最低使用 Ubuntu 24.04，并安装 Tauri 和 Skia 的系统开发依赖。Linux 必须从源码构建 Skia，使用系统 FreeType、HarfBuzz 和 ICU；依赖安装命令和 Linux 专用环境变量见 [原生弹幕渲染构建](../docs/development/native-overlay.md)。首次源码构建耗时较长。

### Windows

Windows 下分为两个版本，分别是 `cpu` 和 `cuda` 版本。区别在于 Whisper 是否使用 GPU 加速。
`cpu` 版本使用 CPU 进行推理，`cuda` 版本使用 GPU 进行推理。

默认运行为 `cpu` 版本，使用 `yarn tauri dev --features cuda` 命令运行 `cuda` 版本。

首次构建前须运行 `node scripts/prepare-windows-skia.mjs` 并按输出设置 `SKIA_BINARIES_URL`，以避免 Skia 与 V8 内置符号的链接冲突。Windows 应用默认包含 Vulkan 导出后端，脚本默认准备匹配的 Vulkan 包；之前使用 CPU 包的环境须重新运行脚本，无需额外 Cargo feature。详见 [原生弹幕渲染构建](../docs/development/native-overlay.md#windows处理-skia-内置-icu-的符号冲突)。

在运行前，须要安装以下依赖：

1. 安装 LLVM 且配置相关环境变量，详情见 [LLVM Windows Setup](https://llvm.org/docs/GettingStarted.html#building-llvm-on-windows)；

2. 安装 CUDA Toolkit，详情见
   [CUDA Windows Setup](https://docs.nvidia.com/cuda/cuda-installation-guide-microsoft-windows/index.html)；
   要注意，安装时请勾选 **VisualStudio integration**。

### 常见问题

#### 1. error C3688

构建前配置参数 `/utf-8`：

```powershell
$env:CMAKE_CXX_FLAGS="/utf-8"
```

#### 2. error: 'exists' is unavailable: introduced in macOS 10.15

配置环境变量 `CMAKE_OSX_DEPLOYMENT_TARGET`，不低于 `13.3`。

```bash
# 在 macOS Tahoe 上编译时，必须设置 SDKROOT
export SDKROOT=$(xcrun --sdk macosx --show-sdk-path) && export CMAKE_OSX_DEPLOYMENT_TARGET=13.3
```

### 3. CUDA arch 错误

配置环境变量 `CMAKE_CUDA_ARCHITECTURES`，可以参考 Workflows 中的配置。
