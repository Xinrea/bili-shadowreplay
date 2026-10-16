# 桌面应用内更新

关于页在启动时检查 GitHub 最新正式 Release 的 `latest.json`。有适配当前系统和架构的新版时，显示「更新至」按钮。点击后下载、验证签名、安装并自动重启；下载或安装失败可重试，重启失败可单独重试重启。更新会中断正在运行的录制和任务，请先结束这些操作。

Windows CPU 与 CUDA 使用不同的更新目标，不会相互覆盖。Linux 应用内更新适用于 AppImage；deb/rpm 和 Docker/Web 部署仍通过原有方式升级。

## 首次配置发布签名

已生成的私钥保存在本机项目目录的 `.updater/bili-shadowreplay.key`，该目录被 Git 忽略，权限仅限当前用户。对应公钥已写入 `src-tauri/tauri.conf.json`。

1. 将私钥文件的完整内容存入 GitHub 仓库的 Actions Secret `TAURI_SIGNING_PRIVATE_KEY`。不要将私钥提交到仓库、日志或 Release。
2. 当前私钥未设密码，`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 可留空。若替换成有密码的密钥，将密码保存为同名 Secret。
3. 备份私钥。以后所有版本都必须用同一把私钥签名，否则已安装的客户端无法验证更新。

其他维护者如需重新初始化（仅在尚未发布此公钥对应的客户端时），运行 `yarn tauri signer generate -w .updater/bili-shadowreplay.key`，将生成的 `.pub` 文件完整内容写入配置的 `plugins.updater.pubkey`，并同步更新 Secret。

本地打包也需要提供签名环境变量：

```bash
export TAURI_SIGNING_PRIVATE_KEY="$PWD/.updater/bili-shadowreplay.key"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
yarn tauri build
```

## 发布流程

Release workflow 为五个平台生成签名安装包和独立 manifest，再统一合并 `latest.json` 上传到 Release 草稿。所有平台成功后才会生成完整更新索引，避免并发覆盖或将 CPU 用户升级为 CUDA 包。检查草稿中的安装包、`.sig` 与 `latest.json` 后发布正式 Release，客户端即可检测。

Git tag 必须与 `package.json` 版本完全一致（例如 `v2.25.0-rc.1` 对应 `2.25.0-rc.1`）。包含预发布标识的版本会标记为 GitHub prerelease，并跳过稳定版 `latest.json` 的上传；更新索引合并脚本也拒绝预发布版本。Tauri Action 固定在 v0.6.2 的提交，以保持更新包下载名称稳定。

旧版本 Release 没有签名更新包和 `latest.json`，无法通过更新器安装。首次使用此功能需要手动安装包含更新器的版本；后续签名版本才可应用内升级。

官方更新器配置：[Tauri Updater](https://v2.tauri.app/plugin/updater/)。
