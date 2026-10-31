# 原生弹幕渲染构建

原生弹幕渲染使用 `native-overlay` crate 和 Skia 0.82 的 `textlayout` 功能。Linux 使用源码构建 Skia，并链接发行版提供的系统库；macOS 保持上游预编译 Skia 的默认路径；Windows 使用经过处理的上游预编译 Skia（见下文）。Windows 和 macOS 都不需要设置下文的 Linux 专用变量。

## Windows：处理 Skia 内置 ICU 的符号冲突

上游 Windows 预编译包的 `skunicode_icu.lib` 与 V8（`deno_core` 依赖）都内置了 ICU 74。Skia 的 ICU 关闭了版本化重命名，而 `ulocimp_getParent` 等少数 ICU 内部符号在 V8 的 ICU 中同样没有版本后缀，两边定义同名符号。MSVC 链接时会报 `LNK2005 ulocimp_getParent already defined`，未报错的同名符号也可能让 Skia 的 ICU 调用到 V8 的实现。

Vulkan 渲染还会从 `skia.lib` 拉入 `SkExecutor`，其中无异常 STL 的 `std::terminate()` 与 V8 的实现同名，完整应用会出现 `LNK2005`。准备脚本同样隔离这个符号的定义和引用，保留各库自己的实现；更新依赖后须验证完整应用链接。

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

Windows 应用默认构建 Vulkan 后端，准备脚本默认下载同版本的 `textlayout-vulkan` 包：

```powershell
node scripts/prepare-windows-skia.mjs
# 按输出设置 SKIA_BINARIES_URL；升级前已准备 CPU 包的环境须重新运行。
```

脚本校验官方包的 SHA-256、包内版本和 feature key 后隔离 ICU/STL 符号。`--vulkan` 保留兼容；`--cpu` 仅用于单独构建没有开启 Vulkan 的 crate，不能用于普通 Windows 应用。CPU 与 Vulkan 包可保存在同一输出目录。脚本只准备链接依赖，Vulkan 设备与编码能力由运行时探测；源码构建时的 Skia feature 必须与所选压缩包一致。

## Windows：真实视频 GPU 导出

Windows 的普通桌面和 headless 构建都默认包含 GPU 合成后端，并接到正式弹幕压制、项目导出和投稿的原生 `MediaPlan` 中，无需额外的 Cargo feature：

```powershell
$prepared = node scripts/prepare-windows-skia.mjs
$env:SKIA_BINARIES_URL = ($prepared | Where-Object { $_ -like 'SKIA_BINARIES_URL=*' }) -replace '^SKIA_BINARIES_URL=', ''
yarn tauri build
```

现有原生解码和项目滤镜图负责剪辑、缩放、静态图层与时间轴，输出 NV12 视频底图；底图上传一次，Skia 在共享 Vulkan 设备上绘制预乘透明叠层，专用 Vulkan compute shader 在 GPU 合成并输出 NV12 硬件帧，随后由原 `MediaPlan` 编码和封装。音频复制/混音、进度、取消、临时文件发布及资源 lease 继续使用现有实现，没有 rawvideo 管道或外部 FFmpeg 进程。

应用后端允许最多三帧 GPU 工作在途。Skia 提交后通过同一图形队列和 `AVVkFrame` timeline 连接后续转换与编码，不再在每次绘制及桥接屏障后固定等待 CPU；复用帧槽、最终 drain、错误或取消时仍会等待 GPU，并持有帧和纹理直到消费者结束。编码器使用 `async_depth=3`。这消除了渲染帧回读，CPU 视频解码和底图上传仍然存在。

默认的速度优先导出先用一个小样本实际渲染、编码和解码检查蓝色底图与半透明红色叠层，成功后自动选择 Vulkan；设备或编码器不支持时在启动前选择现有 CPU 合成路径。质量优先保留已有编码策略；明确不符合受支持 SDR 范围（BT.709 色域、BT.709/sRGB 传递曲线）的源在启动前选择 CPU 合成。执行中的错误不会静默重跑。CPU 与 Vulkan 分片使用不同缓存身份。Windows 与 macOS 默认包含 Vulkan 渲染后端；macOS 通过下述 VideoToolbox 适配器编码，Linux 保留已有路径。

需要匹配的 FFmpeg 9 开发 SDK、Vulkan Video 编码器与驱动。FFmpeg 9.0.2 有已知的 Vulkan 编码器退出图片视图清理问题；源码回补及校验说明见 [`scripts/ffmpeg-sdk-patches`](https://github.com/Xinrea/bili-shadowreplay/tree/main/scripts/ffmpeg-sdk-patches)。升级 SDK 后应执行原生 GPU 回归和 Vulkan validation，功能测试不能替代驱动层验证。

## macOS：MoltenVK 渲染与 VideoToolbox 编码

macOS 与 Windows 共享 `MediaPlan`、Skia Vulkan 场景绘制、专用 Vulkan compute 合成和 NV12 转换，以及原生解码、时间轴、音频、分段缓存、取消和封装逻辑。`RenderPipeline` 显式选择渲染后端与编码器；编码器名称不再决定渲染方式，同样使用 VideoToolbox 的 CPU/GPU 导出具有不同缓存身份。

```text
MediaPlan 解码/滤镜 → NV12 底图上传 → Skia Vulkan + Vulkan compute 合成
                                      ├─ Windows: AVVkFrame → Vulkan Video
                                      └─ macOS: MoltenVK R8/RG8 → Metal blit
                                                → IOSurface/CVPixelBuffer → VideoToolbox
```

macOS 使用 `VK_EXT_metal_objects` 导出两个独立 GPU 纹理，复制到 FFmpeg VideoToolbox 帧池提供的 IOSurface。它不使用 Skia Metal 渲染后端，也不要求 Vulkan Video。桥接默认采用最多三帧的异步交接队列：检查 Vulkan 写入完成后提交 Metal GPU blit，复制完成的帧按顺序交给原 `MediaPlan` 编码器；队列满、收尾或取消时才阻塞推进。没有 `readPixels`、`hwdownload` 或 CVPixelBuffer CPU 映射，但每帧仍有一次 GPU 内部复制，并非严格的零拷贝。底图目前仍由 CPU 多线程解码并上传一次。

开发依赖：

```bash
brew install ffmpeg-full pkg-config molten-vk
export FFMPEG_DIR="$(brew --prefix ffmpeg-full)"
export PKG_CONFIG_PATH="$FFMPEG_DIR/lib/pkgconfig"
export BSR_MOLTENVK_LIBRARY="$(brew --prefix molten-vk)/lib/libMoltenVK.dylib"
# 未打包开发运行时，为 FFmpeg 提供动态库搜索目录：
export DYLD_LIBRARY_PATH="$(dirname "$BSR_MOLTENVK_LIBRARY")${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
yarn tauri dev
```

Skia 默认构建 `textlayout-vulkan`，无需新增 Cargo feature 开关。普通构建仍保留能力探测后的 CPU 回退；不支持的 SDR/HDR 输入和质量优先策略维持已有行为。

发布 hook 将 MoltenVK 复制为 `Contents/Frameworks/libvulkan.dylib`，统一重定位 SDK 内 Vulkan 动态依赖（完整 FFmpeg SDK 若包含 libplacebo，也会处理它的依赖；合成器不再调用该滤镜），并使用适合 FFmpeg `dlopen` 的 install-name。应用从包内绝对路径加载，所有 dylib 继续执行架构、最低系统版本与签名检查。安装后的应用无需 Vulkan SDK、ICD 文件或环境变量。`BSR_MOLTENVK_LIBRARY` 仅指定构建时的库；`BSR_VULKAN_LIBRARY` 是未打包验证工具的加载覆盖，必须指向绝对路径 `libvulkan.dylib`，可用于 Vulkan loader/API capture，打包应用优先使用自己的库。

FFmpeg 9.0.2 在 MoltenVK 设备创建时存在 Vulkan 1.2 features 与独立 timeline semaphore features 重复声明的已知验证层诊断（`VUID-VkDeviceCreateInfo-pNext-02830`）。SDK 升级后需重新验证设备初始化与实际导出，不应屏蔽验证层来替代修复。

## 共享 shader 与回归测试

共享 compute shader 使用 BT.709 预乘透明度和 2×2 色度采样，源码与嵌入式 SPIR-V 位于 `src-tauri/crates/ffmpeg_utils/src/vulkan_video/shaders/`，不需要运行时 GLSL 编译器。修改 shader 时，用 `glslang` 和 `spirv-val` 重新编译、验证并检查提交产物：

```sh
scripts/build-vulkan-shader.sh --write
scripts/build-vulkan-shader.sh --check
```

可通过 `GLSLANG` 和 `SPIRV_VAL` 指定工具路径。纯参数、颜色与所有权回归跟随普通测试；实际 GPU 测试显式执行，需要匹配的 FFmpeg SDK、驱动和运行库：

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --features headless native_gpu_ -- --ignored --test-threads=1
cargo test --locked --manifest-path src-tauri/Cargo.toml -p ffmpeg_utils --features vulkan-video vulkan_video:: -- --ignored --test-threads=1
```

测试覆盖颜色与透明度、BT.709/sRGB 转换、H.264/HEVC、短尾排空、帧时间戳、音频、取消及帧资源生命周期。输出校验使用独立参考；正式导出仍只调用 FFmpeg 库。

共享 compute 支持 `iec61966-2-1`（sRGB）视频输入：在 GPU 内先将视频的 sRGB 传递曲线转换为 BT.709，再合成现有 Skia 预乘 RGBA 叠层，输出 BT.709 NV12。转换按实际 AVFrame 判断，支持拼接时切换传递曲线，不新增 CPU 帧回读。整场录播/弹幕压制和仅使用原生 Skia 叠层的项目可走此路径；带前置 CPU 图片/视频叠层或外部 libass 字幕的项目仍保留 BT.709 预检，避免对已经混合的颜色执行错误转换。

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
