# 原生弹幕渲染构建

原生弹幕渲染使用 `native-overlay` crate 和 Skia 0.82 的 `textlayout` 功能。Linux 使用源码构建 Skia，并链接发行版提供的系统库；macOS 保持上游预编译 Skia 的默认路径；Windows 使用经过处理的上游预编译 Skia（见下文）。Windows 和 macOS 都不需要设置下文的 Linux 专用变量。

## macOS：Metal 弹幕压制

`native-overlay` 的 `metal` feature 为现有渲染器添加 GPU 后端，预览、字幕和项目布局仍共享
上游的同一套解析、字体与缓存。`metal-export` 只负责软件解码 → Metal 绘制/叠加 → VideoToolbox
编码的原生流水线；应用通过自己的 `--metal-export` 子进程入口执行任务并处理进度与取消。

构建时安装 Homebrew 开发依赖：

```sh
brew install ffmpeg pkg-config
```

默认使用 `brew --prefix ffmpeg` 的头文件，要求 **FFmpeg >= 8.0**，只构建一个 ABI 适配器。
不下载、不源码编译 FFmpeg，也不要求第二套 FFmpeg。只有显式设置构建变量
`BSR_FFMPEG_PREFIXES`（以 `:` 分隔的绝对安装前缀）才会构建多个 ABI。例如安装了 8 和 9 后：

```sh
BSR_FFMPEG_PREFIXES="$(brew --prefix ffmpeg@8):$(brew --prefix ffmpeg@9)" \
  cargo build --manifest-path src-tauri/Cargo.toml
```

运行时从标准 Homebrew 路径和构建时指定的前缀中加载一套完整、匹配已构建适配器的动态库，
也可通过 `BSR_FFMPEG_LIBRARY_DIR` 指定库目录。没有匹配版本时导出会报错，不影响应用启动。
同一个安装包支持 8/9 的前提是构建时包含了两个 ABI；默认单版本包需要其对应 ABI。
macOS 发布 workflow 会安装 `ffmpeg@8`、`ffmpeg@9` 并设置上述变量，每个架构的发布包均包含两个 ABI。
本地未设置变量时仍只构建当前 `ffmpeg` 的一个 ABI。
Intel 和 Apple Silicon 使用各自架构的构建机和运行库，原生组件部署目标为 macOS 13.3。
详细的流水线、时间轴、旋转处理和库加载说明见
[`metal-export/README.md`](../../src-tauri/crates/metal-export/README.md)。

## Windows：处理 Skia 内置 ICU 的符号冲突

上游 Windows 预编译包的 `skunicode_icu.lib` 与 V8（`deno_core` 依赖）都内置了 ICU 74。Skia 的 ICU 关闭了版本化重命名，而 `ulocimp_getParent` 等少数 ICU 内部符号在 V8 的 ICU 中同样没有版本后缀，两边定义同名符号。MSVC 链接时会报 `LNK2005 ulocimp_getParent already defined`，未报错的同名符号也可能让 Skia 的 ICU 调用到 V8 的实现。

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
