# 配置使用

## 账号配置

要添加直播间，至少需要配置一个同平台的账号。在账号页面，你可以通过添加账号按钮添加一个账号。

- B 站账号：目前支持扫码登录和 Cookie 手动配置两种方式，推荐使用扫码登录
- 抖音账号：目前仅支持 Cookie 手动配置登陆

### 抖音账号配置

首先确保已经登录抖音，然后打开[个人主页](https://www.douyin.com/user/self)，右键单击网页，在菜单中选择 `检查（Inspect）`，打开开发者工具，切换到 `网络（Network）` 选项卡，然后刷新网页，此时能在列表中找到 `self` 请求（一般是列表中第一个），单击该请求，查看`请求标头`，在 `请求标头` 中找到 `Cookie`，复制该字段的值，粘贴到配置页面的 `Cookie` 输入框中，要注意复制完全。

![DouyinCookie](/images/douyin_cookie.png)

## FFmpeg 配置

如果想要使用切片生成和压制功能，请确保 FFmpeg 已正确配置；除了 Windows 平台打包自带 FFfmpeg 以外，其他平台需要手动安装 FFfmpeg，请参考 [FFfmpeg 配置](/getting-started/ffmpeg)。

## Whisper 模型配置

要使用 AI 字幕识别功能，需要在设置页面配置 Whisper 模型路径，模型文件可以从网络上下载，例如：

- [Whisper.cpp（国内镜像，内容较旧）](https://www.modelscope.cn/models/cjc1887415157/whisper.cpp/files)
- [Whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp/tree/main)

可以跟据自己的需求选择不同的模型，要注意带有 `en` 的模型是英文模型，其他模型为多语言模型。
