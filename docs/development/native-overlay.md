# 原生弹幕渲染构建

原生弹幕渲染使用 `native-overlay` crate 和 Skia 0.82 的 `textlayout` 功能。Linux 使用源码构建 Skia，并链接发行版提供的系统库；macOS 保持上游预编译 Skia 的默认路径；Windows 使用经过处理的上游预编译 Skia（见下文）。Windows 和 macOS 都不需要设置下文的 Linux 专用变量。

## Windows：处理 Skia 内置 ICU 的符号冲突

上游 Windows 预编译包的 `skunicode_icu.lib` 与 V8（`deno_core` 依赖）都内置了 ICU 74。Skia 的 ICU 关闭了版本化重命名，而 `ulocimp_getParent` 等少数 ICU 内部符号在 V8 的 ICU 中同样没有版本后缀，两边定义同名符号。MSVC 链接时会报 `LNK2005 ulocimp_getParent already defined`，未报错的同名符号也可能让 Skia 的 ICU 调用到 V8 的实现。

Vulkan 渲染还会从 `skia.lib` 拉入 `SkExecutor`，其中无异常 STL 的 `std::terminate()` 与 V8 的实现同名，完整应用会出现 `LNK2005`。准备脚本同样隔离这个符号的定义和引用，保留各库自己的实现。独立 Vulkan PoC 没有链接 V8，无法替代这项完整应用链接验证。

`scripts/prepare-windows-skia.mjs` 下载同版本、同目标、同 feature 的官方压缩包并校验 SHA-256，用 `llvm-objcopy --redefine-sym` 把这些共有符号在 Skia 库中的定义和引用统一加上 `bsr_skia_` 前缀，再重新打包，最后输出 `SKIA_BINARIES_URL`。构建前在 **Windows 构建终端**中运行：

```powershell
rustup component add llvm-tools   # 或使用 PATH 中的 llvm-objcopy，也可设置 LLVM_OBJCOPY
node scripts/prepare-windows-skia.mjs
# 按脚本输出设置，例如：
$env:SKIA_BINARIES_URL="file://D:/path/to/bili-shadowreplay/src-tauri/target/skia-binaries/skia-binaries-{key}.tar.gz"
yarn tauri dev
```

- 未设置 `SKIA_BINARIES_URL` 时，`native-overlay` 的构建脚本会直接报错，避免等到最后链接才失败。
- 地址必须是 `file://D:/...` 形式（0.82 直接去掉 `file://` 前缀读取路径），并保留 `{key}` 占位符，由 skia-bindings 填入。
- 若压缩包的 key 与当前构建不一致（例如修改了 Skia feature），skia-bindings 会改为源码构建未处理的 Skia，最终仍会链接失败。
- 升级 `skia-safe`/`skia-bindings` 或 `deno_core`（V8）后，需重新比对两边 ICU 的共有符号，更新脚本中的 key、SHA-256 和符号列表；脚本会在锁定版本不符或符号不存在时报错。
- Release workflow 会在 Windows 任务中自动执行这一步。

Windows Vulkan 构建需要同版本的 `textlayout-vulkan` 包，使用显式选项准备：

```powershell
node scripts/prepare-windows-skia.mjs --vulkan
# 仍按输出设置 SKIA_BINARIES_URL；与 CPU 包可保存在同一个输出目录。
```

该选项选择官方 Skia 0.82 `textlayout-vulkan` 预编译包，校验下载 SHA-256、包内版本和 feature key 后执行相同的 ICU 符号处理。默认调用仍准备 `textlayout` CPU 包。它只准备链接依赖，Vulkan 设备与编码能力由运行时探测；源码构建时的 Skia feature 必须与所选压缩包一致。

## Windows：真实视频 GPU 导出

使用 `vulkan-export` Cargo feature 构建应用，将 GPU 合成接到正式弹幕压制、项目导出和投稿的原生 `MediaPlan` 中：

```powershell
$prepared = node scripts/prepare-windows-skia.mjs --vulkan
$env:SKIA_BINARIES_URL = ($prepared | Where-Object { $_ -like 'SKIA_BINARIES_URL=*' }) -replace '^SKIA_BINARIES_URL=', ''
yarn tauri build --features vulkan-export
```

现有原生解码和项目滤镜图负责剪辑、缩放、静态图层与时间轴，输出 NV12 视频底图；底图上传一次，Skia 在共享 Vulkan 设备上绘制预乘透明叠层，libplacebo 在 GPU 合成并输出 NV12 硬件帧，随后由原 `MediaPlan` 编码和封装。音频复制/混音、进度、取消、临时文件发布及资源 lease 继续使用现有实现，没有 rawvideo 管道或外部 FFmpeg 进程。

速度优先导出先用一个小样本实际渲染、编码和解码检查蓝色底图与半透明红色叠层，成功后才选择 Vulkan。质量优先保留已有编码策略；明确不符合 SDR BT.709 范围的源在启动前选择 CPU 合成。执行中的错误不会静默重跑。CPU 与 Vulkan 分片使用不同缓存身份。macOS 暂不自动选择该后端。

需要匹配的 FFmpeg 9 开发 SDK、`libplacebo`、Vulkan Video 编码器与驱动。FFmpeg 9.0.2 仍有已知的 Vulkan 编码器退出图片视图清理问题；源码回补及校验说明见 [`scripts/ffmpeg-sdk-patches`](../../scripts/ffmpeg-sdk-patches/README.md)。功能测试通过不等于旧 SDK 的 Vulkan validation 全量无错误。实际运行与验证记录见独立工具的 [README](../../scripts/vulkan-video-poc/README.md)。

Windows 的 Skia GPU → FFmpeg Vulkan Video 独立验证入口位于仓库的 [`scripts/vulkan-video-poc`](https://github.com/Xinrea/bili-shadowreplay/blob/main/scripts/vulkan-video-poc/README.md)。它与应用共享 Vulkan 合成器，保留生成底图的独立性能基线，并提供 API 跟踪、输出视频与画面内容的独立校验。正式导出回归记录见 [Windows 验证回执](../../scripts/vulkan-video-poc/validation/windows-production-export-2026-10-30.json)。

## Linux：Ubuntu 24.04 起

桌面发布环境为 **Ubuntu 24.04**，Ubuntu 22.04 不再是受支持的构建或运行环境。原因是：

- 上游 Linux Skia 0.82 预编译 `textlayout` 产物缺少所需的 COLRv1 彩色字体渲染路径，需要源码构建并使用较新的系统 FreeType。
- Skia 内置 ICU 与 V8 内置 ICU 可能产生 `ulocimp_getParent` 等符号冲突；改用发行版带版本化符号的系统 ICU。
- Ubuntu 22.04 的 HarfBuzz 2.7.4 缺少此 Skia 构建所需的字体子集 API，不能只增加编译变量而继续使用旧环境。

先安装 Rust、Node.js 和 Yarn，并初始化仓库子模块。Ubuntu 24.04 的原生构建依赖如下（包括桌面端 Tauri 依赖）：

```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential cmake pkg-config git \
  clang libclang-dev ninja-build python3 \
  libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf \
  libicu-dev libharfbuzz-dev libfreetype-dev libfontconfig1-dev \
  libpng-dev zlib1g-dev libexpat1-dev libjpeg-dev
```

仅在 **Linux 构建终端**中设置：

```bash
export FORCE_SKIA_BUILD=1
export SKIA_USE_SYSTEM_LIBRARIES=1
export CC=clang
export CXX=clang++

git submodule update --init --recursive
yarn install
yarn tauri dev
# 打包桌面应用：
# yarn tauri build
```

这两个 Skia 开关按变量是否存在判断，`0`、`false` 和空字符串也会启用。不要在 Windows/macOS、跨平台的全局环境或 workflow 顶层设置占位值；需要恢复默认路径时使用 `unset FORCE_SKIA_BUILD SKIA_USE_SYSTEM_LIBRARIES`，而不是赋空值。

首次源码构建会下载 Skia 及其依赖，显著增加编译时间、磁盘与内存开销，需要能够访问上游源码。修改构建变量或切换系统库版本后，不应复用旧的 Skia 构建产物；请清理对应的 Cargo 构建缓存后重试。

## 打包与部署

Ubuntu 24.04 的 `.deb` 在 Tauri 自动添加的桌面依赖之外，显式声明 Skia 所需的运行库：`libicu74`、`libharfbuzz0b`、`libharfbuzz-subset0`、`libfreetype6`、`libfontconfig1`、`libpng16-16t64`、`libjpeg-turbo8`、`libexpat1` 和 `zlib1g`。开发包（`*-dev`）、Clang、Ninja 和 Python 仅用于构建，不应作为最终用户的运行依赖。

AppImage 也受到构建系统 ABI 和系统运行库的限制，不代表兼容 Ubuntu 22.04。其他发行版需自行确认相应的共享库名称、ABI 和版本；Ubuntu 包名不能直接用于 Debian。

Docker 的构建阶段与运行阶段应保持在同一 **Debian trixie** 基线上，分别安装对应的开发包和运行库。不要只升级 builder 后继续使用旧版 Debian runtime，也不要把 Ubuntu 构建出的二进制直接复制到 Debian 镜像中。容器使用自身的用户态运行库，不要求宿主机安装上述 Ubuntu 桌面依赖；使用方式见 [Docker 部署](../getting-started/installation/docker.md)。

发布前仍需在 Linux CI 或对应系统上完成实际源码编译、原生渲染测试和安装包冒烟测试，并检查二进制的动态依赖（例如 `ldd`）是否都能在目标系统解析。仅检查 workflow、配置或文档不能替代这些验证。
