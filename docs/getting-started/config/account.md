# 账号配置

要添加直播间，至少需要配置一个同平台的账号。在账号页面，你可以通过添加账号按钮添加一个账号。

- B 站账号：目前支持扫码登录和 Cookie 手动配置两种方式，推荐使用扫码登录
- 抖音账号：目前仅支持 Cookie 手动配置登陆

## 账号凭据存储

桌面版默认启用 `credential-encryption` 编译特性，Cookie 和 CSRF Token 使用 AES-256-GCM 加密后写入 SQLite，已有明文账号会在启动时自动迁移。主密钥保存在系统凭据存储中：macOS Keychain、Windows Credential Manager 或 Linux Secret Service。Linux 需要安装并解锁 Secret Service；系统凭据存储不可用时，启用加密的程序会启动失败。

Docker、Termux 和默认无界面构建使用 `--no-default-features --features headless`，关闭加密并以明文保存账号凭据，无需系统密钥库。系统支持密钥库时，可以使用 `--no-default-features --features headless,credential-encryption` 构建启用加密的无界面版本。桌面版也可使用 `--no-default-features --features gui` 关闭加密。

关闭加密的版本不能打开包含已加密账号的数据库，需要改用启用 `credential-encryption` 的版本。恢复加密数据库时仍需访问系统密钥库中的原主密钥；仅复制数据库到其他设备无法恢复账号。升级前的备份仍可能含有明文凭据。

## 抖音账号配置

首先确保已经登录抖音，然后打开[个人主页](https://www.douyin.com/user/self)，右键单击网页，在菜单中选择 `检查（Inspect）`，打开开发者工具，切换到 `网络（Network）` 选项卡，然后刷新网页，此时能在列表中找到 `self` 请求（一般是列表中第一个），单击该请求，查看`请求标头`，在 `请求标头` 中找到 `Cookie`，复制该字段的值，粘贴到配置页面的 `Cookie` 输入框中，要注意复制完全。

![DouyinCookie](/images/douyin_cookie.png)
