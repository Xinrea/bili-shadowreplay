# BiliBili-ShadowReplay contribute guide

## Project Setup

首次克隆请包含 Whisper 子模块：

```bash
git clone --recurse-submodules https://github.com/Xinrea/bili-shadowreplay.git
cd bili-shadowreplay
```

已有 checkout 如未初始化子模块，先运行 `git submodule update --init --recursive`，再按平台准备基础编译工具。

### MacOS

准备 Rust、Node/Yarn 和 Xcode Command Line Tools 后，在仓库目录运行：

```bash
brew install ffmpeg-full pkg-config
yarn install --frozen-lockfile
yarn tauri dev
```

`yarn tauri` 会自动发现 FFmpeg 开发库，并在首次启动时下载、校验和缓存固定版本的 MoltenVK，后续离线复用。无需手动设置库路径、安装 Vulkan SDK 或单独安装 MoltenVK；`dev`、`build` 和 `bundle` 使用相同的准备流程。缺少 FFmpeg 开发库时会在构建前提示安装命令。自定义 SDK 和独立 Cargo 测试配置见 [FFmpeg 开发环境](../docs/getting-started/config/ffmpeg.md#macos)。

### Linux

最低使用 Ubuntu 24.04，并安装 Tauri 和 Skia 的系统开发依赖。Linux 必须从源码构建 Skia，使用系统 FreeType、HarfBuzz 和 ICU；依赖安装命令和 Linux 专用环境变量见 [原生弹幕渲染构建](../docs/development/native-overlay.md)。首次源码构建耗时较长。

### Windows

Windows 下分为两个版本，分别是 `cpu` 和 `cuda` 版本。区别在于 Whisper 是否使用 GPU 加速。
`cpu` 版本使用 CPU 进行推理，`cuda` 版本使用 GPU 进行推理。

基础编译工具：当前 stable Rust（`x86_64-pc-windows-msvc`）、Node/Yarn、Visual Studio 2022 或 Build Tools 的“使用 C++ 的桌面开发”（包含 MSVC x64、Windows SDK、CMake 工具），以及 64 位 LLVM（或 Visual Studio 的 C++ Clang 工具）。标准安装位置会自动发现，无需手动修改 `PATH` 或使用 Developer PowerShell。

```powershell
yarn install --frozen-lockfile
yarn tauri dev
```

启动器自动导入 Visual Studio 编译环境，定位 LLVM，下载并校验 FFmpeg shared SDK 与 Skia Vulkan 预编译包，处理 Skia/V8 的 ICU/STL 符号冲突并准备运行 DLL。后续复用校验过的缓存；无需运行准备脚本或手工设置 `FFMPEG_DIR`、`LIBCLANG_PATH`、`SKIA_BINARIES_URL`。`yarn tauri build/bundle` 复用同一准备流程。详见 [原生弹幕渲染构建](../docs/development/native-overlay.md#windows处理-skia-内置-icu-的符号冲突)。

默认运行为 `cpu` 版本。只有 `yarn tauri dev --features cuda` 才需要另外安装 CUDA Toolkit，详情见
   [CUDA Windows Setup](https://docs.nvidia.com/cuda/cuda-installation-guide-microsoft-windows/index.html)；
安装时请勾选 **VisualStudio integration**，并使用该 CUDA 版本支持的 Visual Studio 工具链。

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
