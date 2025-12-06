# BiliBili-ShadowReplay contribute guide

## Project Setup

### MacOS

项目无需额外配置，直接 `yarn tauri dev` 即可编译运行。

### Linux

也无需额外配置。

### Windows

Windows 下分为两个版本，分别是 `cpu` 和 `cuda` 版本。区别在于 Whisper 是否使用 GPU 加速。
`cpu` 版本使用 CPU 进行推理，`cuda` 版本使用 GPU 进行推理。

默认运行为 `cpu` 版本，使用 `yarn tauri dev --features cuda` 命令运行 `cuda` 版本。

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
export SDKROOT=$(xcrun --sdk macosx --show-sdk-path) && export MACOSX_DEPLOYMENT_TARGET=13.3
```

### 3. CUDA arch 错误

配置环境变量 `CMAKE_CUDA_ARCHITECTURES`，可以参考 Workflows 中的配置。
